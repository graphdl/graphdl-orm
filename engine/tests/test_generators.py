"""The generator family (punchlist entry 8), starting with dsl: the per-noun
model summary the old engine persists as dsl:<Noun> cells (noun, object
type, the reading texts, the verbalized constraints as kind-text pairs, the
machine transitions). Generated at compile beside the layout cells, so the
claude cutover carries its generator complement forward."""
import pyarest.prims  # noqa: F401
from pyarest import forml, system


MODEL = """Status is a value type.
Ticket is an entity type.
Ticket has Status.
Each Ticket has at most one Status.
State Machine Definition 'Flow' is for Noun 'Ticket'.
Status 'open' is initial in State Machine Definition 'Flow'.
Transition 'close' is from Status 'open'.
Transition 'close' is to Status 'done'.
Transition 'close' is triggered by Fact Type 'close'.
"""


def test_dsl_cells_generate_per_noun():
    D, rep = forml.compile_model(MODEL)
    D = system.run_rules(D)
    D = system.generator_cells(D)
    rows = system._pop_rows(D, "dsl:Ticket")
    assert len(rows) == 1
    row = rows[0]
    got = dict(zip(("noun", "object_type", "readings", "constraints",
                    "transitions"), row))
    assert got["noun"] == "Ticket"
    assert got["object_type"] == "entity"
    assert "Ticket has Status" in got["readings"]
    assert any(k == "UC" and "at most one Status" in text
               for (k, text) in got["constraints"])
    assert ("close", "open", "done") in got["transitions"]
    # the value type gets its own cell with its kind
    vrows = system._pop_rows(D, "dsl:Status")
    assert vrows and vrows[0][1] == "value"


def _pipeline(model):
    # mirror the protocol compile order (protocol.py): layout before
    # generators, so rmapColumns exists when the canon classifies
    D, _ = forml.compile_model(model)
    D = system.layout_cells(D)
    D = system.generator_cells(D)
    return D


def test_the_xsd_generator_is_opt_in_and_canon_classified():
    # docs/07-generators.md (restored): a generator not opted in
    # produces nothing; opted in, xsd:{Noun} carries an xs:complexType
    # whose elements are system:ev_cols' classified columns
    import xml.etree.ElementTree as ET

    D = _pipeline(MODEL)
    assert not system._pop_rows(D, "xsd:Ticket")

    OPTED = MODEL + "App 'flowapp' uses Generator 'xsd'.\n"
    D2 = _pipeline(OPTED)
    xsd = system._pop_rows(D2, "xsd:Ticket")[0][0]
    root = ET.fromstring(xsd.replace("xs:", ""))       # namespace-light parse
    assert root.tag == "complexType" and root.get("name") == "Ticket"
    names = [e.get("name") for e in root.iter("element")]
    assert "status" in names
    assert root.find("attribute").get("use") == "required"


def test_the_generator_family_transduces_one_canon_classification():
    # the runtime-parity list (owl edm html dtd wsdl xforms plix nav;
    # NORMA's XML/OIALto* transforms the oracle): every format renders
    # the SAME system:ev_cols classification — ref columns become
    # ObjectProperty/NavigationProperty/data-ref/nav links, machine
    # triples become wsdl operations and nav transitions, identity is
    # the id key everywhere
    import json
    import xml.etree.ElementTree as ET

    OPTED = MODEL + (
        "Customer is an entity type.\n"
        "Ticket is assigned to Customer.\n"
        "Each Ticket is assigned to at most one Customer.\n"
        "Noun 'Ticket' has Plural 'tickets'.\n"
        + "".join("App 'flowapp' uses Generator '%s'.\n" % g
                  for g in ("owl", "edm", "html", "dtd", "wsdl",
                            "xforms", "plix", "nav")))
    D = _pipeline(OPTED)

    owl = system._pop_rows(D, "owl:Ticket")[0][0]
    assert '<owl:Class rdf:about="#Ticket"/>' in owl
    assert "owl:ObjectProperty" in owl and '"#Customer"' in owl
    assert "XMLSchema#string" in owl

    edm = ET.fromstring(system._pop_rows(D, "edm:Ticket")[0][0])
    assert edm.get("Name") == "Ticket"
    assert edm.find("Key/PropertyRef").get("Name") == "id"
    assert any(p.get("Name") == "status" for p in edm.iter("Property"))
    assert any(n.get("Type") == "Customer"
               for n in edm.iter("NavigationProperty"))

    html = system._pop_rows(D, "html:Ticket")[0][0]
    assert 'action="/tickets"' in html and 'name="status"' in html
    assert 'data-ref="Customer"' in html

    dtd = system._pop_rows(D, "dtd:Ticket")[0][0]
    assert "<!ATTLIST ticket id CDATA #REQUIRED>" in dtd
    assert "<!ELEMENT status (#PCDATA)>" in dtd

    wsdl = system._pop_rows(D, "wsdl:Ticket")[0][0]
    assert '<wsdl:operation name="createTicket">' in wsdl
    assert '<wsdl:operation name="closeTicket">' in wsdl    # Theorem 4a

    xf = system._pop_rows(D, "xforms:Ticket")[0][0]
    assert '<xf:bind nodeset="@id" required="true()"/>' in xf
    assert '<xf:input ref="status">' in xf

    plix = system._pop_rows(D, "plix:Ticket")[0][0]
    assert '<plx:class name="Ticket" visibility="public">' in plix
    assert 'dataTypeName="Customer"' in plix

    nav = json.loads(system._pop_rows(D, "nav:Ticket")[0][0])
    assert nav["self"] == "/tickets/{id}"
    assert any(n["target"] == "Customer"
               and n["href"] == "/tickets/{id}/customers"
               for n in nav["navigation"])
    assert any(t["event"] == "close" and t["to"] == "done"
               for t in nav["transitions"])

    # a generator that is not opted in produces nothing (docs/07)
    D0 = _pipeline(MODEL)
    for fam in ("owl", "edm", "html", "dtd", "wsdl", "xforms",
                "plix", "nav"):
        assert not system._pop_rows(D0, fam + ":Ticket")


def _solidity_fixture():
    OPTED = MODEL + (
        "Customer is an entity type.\n"
        "Ticket is assigned to Customer.\n"
        "Each Ticket is assigned to at most one Customer.\n"
        "App 'flowapp' uses Generator 'solidity'.\n")
    return _pipeline(OPTED)


def test_the_solidity_generator_emits_the_docs07_contract():
    # docs/07 + the recovered oracle (generators/solidity.rs at
    # d3104058~1): struct Data, facts-as-events, onlyInStatus guard,
    # create with UC require + initial status, one function per
    # transition. 0.9 correction: the machine's bytes32 status
    # REPLACES the status value column (status(e) = RMAP column)
    D = _pipeline(MODEL + "App 'flowapp' uses Generator 'solidity'.\n")
    sol = system._pop_rows(D, "solidity:Ticket")[0][0]
    assert "pragma solidity ^0.8.20;" in sol
    assert "contract Ticket {" in sol
    assert "string id;" in sol
    assert "bytes32 status;" in sol
    assert sol.count(" status;") == 1              # replaced, not doubled
    assert "event TicketHasStatus(string indexed id," in sol
    assert "modifier onlyInStatus(string memory id," in sol
    assert 'require(bytes(records[id].id).length == 0, "UC:' in sol
    assert 'records[id].status = keccak256(bytes("open"));' in sol
    assert ('function close(string memory id) external onlyInStatus(id,'
            ' keccak256(bytes("open")))') in sol
    assert 'records[id].status = keccak256(bytes("done"));' in sol


def test_the_solidity_output_forge_builds(tmp_path):
    # the Foundry leg of the chip: the emitted contracts are real
    # solc-compilable Solidity, proven by forge build
    import shutil
    import subprocess
    forge = shutil.which("forge")
    if forge is None:
        import pytest
        pytest.skip("foundry not on disk")
    D = _solidity_fixture()
    (tmp_path / "src").mkdir()
    for noun in ("Ticket", "Customer"):
        sol = system._pop_rows(D, "solidity:" + noun)[0][0]
        (tmp_path / "src" / (noun + ".sol")).write_text(
            sol, encoding="utf-8")
    (tmp_path / "foundry.toml").write_text(
        '[profile.default]\nsrc = "src"\nout = "out"\n',
        encoding="utf-8")
    r = subprocess.run([forge, "build", "--root", str(tmp_path)],
                       capture_output=True, text=True, timeout=300)
    assert r.returncode == 0, r.stdout + r.stderr
