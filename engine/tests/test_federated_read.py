"""The noun-backed federated read (contact-federation.md's contract):
a backed noun demand-fetches its external system on read, columns map
to <Noun>_has_<Field> facts, provenance lands as federatedFrom, and
the fetch is swappable with a fixture twin."""
import os

import pyarest.prims  # noqa: F401
from pyarest import apps, protocol


MODEL = """Submission Id is a value type.
Submitter Name is a value type.
Body is a value type.
Contact Submission(.Submission Id) is an entity type.
Contact Submission has Submitter Name.
Each Contact Submission has at most one Submitter Name.
Contact Submission has Body.
Each Contact Submission has at most one Body.
External System 'test-logs' has URL 'https://example.test'.
External System 'test-logs' has Header 'Authorization'.
External System 'test-logs' has Prefix 'Basic'.
Noun 'Contact Submission' is backed by External System 'test-logs'.
Noun 'Contact Submission' has URI '/?query=q'.
"""


def _mk(tmp_path):
    root = str(tmp_path)
    d = os.path.join(root, "fed", "readings")
    os.makedirs(d)
    with open(os.path.join(d, "app.md"), "w", encoding="utf-8") as f:
        f.write(MODEL)
    reg = apps.Registry(root)
    reg.compile("fed")
    return reg


def test_backed_noun_fetches_on_read(tmp_path):
    calls = []

    def fixture(url, headers):
        calls.append((url, dict(headers)))
        return {"data": [
            {"id": "abc123", "Submitter Name": "Ada", "Body": "hello"},
            {"id": "def456", "Submitter Name": "Sam", "Body": "hi"},
        ]}

    protocol.set_federated_fetch(fixture)
    protocol.Registry._FED_MEMO.clear()
    os.environ["AREST_SECRET_TEST_LOGS"] = "dGVzdA=="
    try:
        reg = _mk(tmp_path)
        ids = reg.entities("fed", "Contact Submission")
        assert "abc123" in ids and "def456" in ids
        # the fetch went to base+uri with the Basic credential
        assert calls and calls[0][0] == "https://example.test/?query=q"
        assert calls[0][1].get("Authorization") == "Basic dGVzdA=="
        # columns landed as <Noun>_has_<Field> facts
        got = reg.get("fed", "Contact Submission", "abc123")
        assert got["fields"].get("Submitter Name") == "Ada"
        # provenance cites the system on the refreshed D
        from pyarest import system
        _ts, D = protocol.Registry._FED_MEMO[("fed", "Contact Submission")]
        cites = {tuple(r) for r in system._pop_rows(D, "federatedFrom")}
        assert ("abc123", "test-logs") in cites
        # the memo makes the second read fetch-free
        n = len(calls)
        reg.entities("fed", "Contact Submission")
        assert len(calls) == n
    finally:
        protocol.set_federated_fetch(None)
        os.environ.pop("AREST_SECRET_TEST_LOGS", None)


BRIDGED = MODEL + """Support Request(.Submission Id) is an entity type.
Description is a value type.
Support Request has Description.
Each Support Request has at most one Description.
Support Request is Contact Submission.
Each Support Request is at most one Contact Submission.
Noun 'Contact Submission' is surfaced as Noun 'Support Request'.
* Support Request has Description iff that Contact Submission has Body and Support Request is Contact Submission and Description is Body.
"""


def test_the_surfaced_noun_gets_bridge_rows_and_rekeyed_fields(tmp_path):
    # THE BRIDGE MINT (identity is transduction): the connector asserts
    # one same-id bridge row per fetched entity — rule-minted cross-noun
    # bridges skolemize — and the canon re-key derives the field
    def fixture(url, headers):
        return {"data": [{"id": "abc123", "Body": "hello"}]}

    protocol.set_federated_fetch(fixture)
    protocol.Registry._FED_MEMO.clear()
    try:
        root = str(tmp_path)
        d = os.path.join(root, "fed", "readings")
        os.makedirs(d)
        with open(os.path.join(d, "app.md"), "w", encoding="utf-8") as f:
            f.write(BRIDGED)
        reg = apps.Registry(root)
        reg.compile("fed")
        reg.entities("fed", "Contact Submission")
        from pyarest import system
        _ts, D = protocol.Registry._FED_MEMO[("fed", "Contact Submission")]
        bridge = {tuple(r) for r in system._pop_rows(
            D, "Support_Request_is_Contact_Submission")}
        assert ("abc123", "abc123") in bridge
        rekey = {tuple(r) for r in system._pop_rows(
            D, "Support_Request_has_Description")}
        assert ("abc123", "hello") in rekey
    finally:
        protocol.set_federated_fetch(None)
