"""External federation per the platform arc: fetch-and-store through the same front
door. httpFetch is the paper's named binding (a registered def; tests override it with
a fixture twin through the universal interface), the importer VERBALIZES the external
vocabulary into canonical FORML (namespaced nouns already parse), compile_model ingests
the readings like any others, instances land as instance facts with provenance
(federatedFrom rows carrying source and tx), and refetch is idempotent by set
semantics. Fixtures cover the three named ecosystems: schema.org (JSON-LD), GS1 (GPC
brick shape), O*NET (occupation rows)."""
import pyarest.prims  # noqa: F401
import pyarest.lam as L
from pyarest.lam import from_lam
from pyarest import federate, forml


SCHEMA_ORG = {
    "@graph": [
        {"@id": "schema:Product", "@type": "rdfs:Class"},
        {"@id": "schema:Offer", "@type": "rdfs:Class"},
        {"@id": "schema:name", "@type": "rdf:Property",
         "schema:domainIncludes": {"@id": "schema:Product"},
         "schema:rangeIncludes": {"@id": "schema:Text"}},
        {"@id": "schema:offers", "@type": "rdf:Property",
         "schema:domainIncludes": {"@id": "schema:Product"},
         "schema:rangeIncludes": {"@id": "schema:Offer"}},
    ]
}

SCHEMA_ORG_ITEMS = [
    {"@type": "schema:Product", "@id": "prod-1", "schema:name": "Widget",
     "schema:offers": "offer-9"},
]

GS1_GPC = {"bricks": [{"code": "10000025", "title": "Cheese"},
                      {"code": "10000026", "title": "Butter"}]}

ONET = {"occupations": [{"code": "15-1252.00", "title": "Software Developers"}]}


def _cell(Dpy, name):
    for c in Dpy:
        if isinstance(c, tuple) and len(c) == 3 and c[:2] == ("CELL", name):
            return set(c[2])
    return set()


def test_schema_org_vocabulary_federates_as_readings():
    readings = federate.jsonld_to_readings(SCHEMA_ORG)
    D, rep = forml.compile_model(readings)
    assert rep["unparsed"] == []
    Dpy = from_lam(D)
    types = {r[0] for r in _cell(Dpy, "instanceOf")}
    assert "schema:Product" in types and "schema:Offer" in types
    fts = {f[0] for f in _cell(Dpy, "factType")}
    assert "schema_Product_has_schema_name" in fts
    assert "schema_Product_offers_schema_Offer" in fts


def test_instances_fetch_and_store_with_provenance():
    def fixture_fetch(url):
        return {"vocab": SCHEMA_ORG, "items": SCHEMA_ORG_ITEMS}

    D, rep = federate.fetch_and_store(None, "https://schema.org/Product",
                                      fetch=fixture_fetch)
    assert rep["unparsed"] == []
    Dpy = from_lam(D)
    assert ("prod-1", "Widget") in _cell(Dpy, "schema_Product_has_schema_name")
    assert ("prod-1", "offer-9") in _cell(Dpy, "schema_Product_offers_schema_Offer")
    prov = _cell(Dpy, "federatedFrom")
    assert any(r[0] == "schema:Product" and r[1] == "https://schema.org/Product"
               for r in prov)


def test_refetch_is_idempotent():
    def fixture_fetch(url):
        return {"vocab": SCHEMA_ORG, "items": SCHEMA_ORG_ITEMS}

    D, _ = federate.fetch_and_store(None, "https://schema.org/Product", fetch=fixture_fetch)
    D2, _ = federate.fetch_and_store(D, "https://schema.org/Product", fetch=fixture_fetch)
    name_rows = [r for c in from_lam(D2)
                 if isinstance(c, tuple) and len(c) == 3
                 and c[1] == "schema_Product_has_schema_name"
                 for r in c[2]]
    assert name_rows.count(("prod-1", "Widget")) == 1         # sets, not duplicates


def test_gs1_and_onet_federate_through_the_same_door():
    r1 = federate.gs1_to_readings(GS1_GPC)
    r2 = federate.onet_to_readings(ONET)
    D, rep = forml.compile_model(r1 + r2)
    assert rep["unparsed"] == []
    Dpy = from_lam(D)
    assert ("10000025", "Cheese") in _cell(Dpy, "gs1_Brick_has_gs1_Title")
    assert ("15-1252.00", "Software Developers") in _cell(Dpy, "onet_Occupation_has_onet_Title")


def test_describe_speaks_from_the_m_facts():
    def fixture_fetch(url):
        return {"vocab": SCHEMA_ORG, "items": SCHEMA_ORG_ITEMS}

    from pyarest import system
    D, _ = federate.fetch_and_store(None, "https://schema.org/Product", fetch=fixture_fetch)
    d = system.describe(D, "schema:Product")
    assert d["kind"] == ["ObjectType"]
    assert any(ft == "schema_Product_offers_schema_Offer" for (ft, _p, _r) in d["roles"])
    assert d["federated_from"] == ["https://schema.org/Product"]


def test_sources_declare_in_m_and_resolve_through_defs():
    # the federation system is FORML + DEFS: sources/connectors are M-facts, fetcher
    # and translator are DEFINITION NAMES resolved by rho, and swapping the fetch is
    # re-registering the name — DEFS as the DI container, per the whitepaper
    from pyarest import defs as d
    DECL = federate._module_readings() + """
Source 'schemaorg' has Url 'https://example.test/schema'.
Source 'schemaorg' uses Connector 'jsonld-http'.
Connector 'jsonld-http' fetches with Fetcher 'httpFetch'.
Connector 'jsonld-http' translates with Translator 'translate_jsonld'.
"""
    D, rep = forml.compile_model(DECL)
    assert rep["unparsed"] == []

    def fixture(mu):
        def g(o):
            return L.atom({"vocab": SCHEMA_ORG, "items": SCHEMA_ORG_ITEMS})
        return g

    d.register("httpFetch", fixture)                          # IoC: swap by re-registering
    try:
        D2, rep2 = federate.fetch_source(D, "schemaorg")
        assert rep2["unparsed"] == []
        Dpy = from_lam(D2)
        assert ("prod-1", "Widget") in _cell(Dpy, "schema_Product_has_schema_name")
        assert ("schemaorg", "https://example.test/schema") in _cell(Dpy, "federatedFrom")
    finally:
        federate.register_bindings()                          # restore the real binding


WILD = {"@graph": [
    {"@id": "https://en.wikipedia.org/wiki/Catholic_Church", "@type": "schema:Article",
     "about": "wd:Q9592", "inLanguage": "en",
     "isPartOf": "https://en.wikipedia.org/",
     "name": {"@language": "en", "@value": "Catholic Church"}},
    {"@id": "https://la.wikipedia.org/wiki/Ecclesia_Catholica", "@type": "schema:Article",
     "about": "wd:Q9592", "inLanguage": "la",
     "isPartOf": "https://la.wikipedia.org/",
     "name": {"@language": "la", "@value": "Ecclesia Catholica"}},
]}


def test_wild_instance_graphs_ingest_and_verbalize():
    from pyarest import system
    readings = federate.jsonld_instance_graph_to_readings(WILD)
    D, rep = forml.compile_model(readings)
    assert rep["unparsed"] == []
    facts = system.facts_about(D, "wd:Q9592")
    assert any("en.wikipedia.org" in str(r) for (_ft, r, _s) in facts)
    sentences = [s for (_ft, _r, s) in facts]
    assert any("has schema:about 'wd:Q9592'" in s for s in sentences)   # NORMA-style
    assert any(s.startswith("schema:Article '") for s in sentences)     # players named
