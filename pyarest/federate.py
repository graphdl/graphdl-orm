"""External federation (the platform arc): fetch-and-store through the same front door.
httpFetch is the paper's NAMED binding — a host fetch the caller may override with a
fixture twin (the universal-interface principle applied at the module edge). The
importer VERBALIZES the external vocabulary into canonical FORML readings
(verbalize-then-ingest, never a second metamodel): namespaced nouns carry the source
prefix (schema:Product — the grammar already parses them), classes become entity types,
object properties become fact types, datatype properties become value types with
has-readings, items become instance facts, and provenance lands as federatedFrom rows.
Refetch is idempotent by set semantics."""
import json

from . import ast, forml, meta, system
from .lam import to_lam
from .reduce import apply as _ap
import pyarest.lam as L


def _S(*xs):
    l = L.NIL
    for x in reversed(xs):
        l = L.CONS(x)(l)
    return L.SEQ(l)


def _local(iri):
    return iri.split(":", 1)[1] if ":" in iri else iri


def _http_fetch(url):
    """The paper's binding, for real deployments; tests hand a fixture twin instead."""
    from urllib.request import urlopen
    with urlopen(url) as r:
        return json.loads(r.read().decode("utf-8"))


# ============================ schema.org (JSON-LD) ============================
def _ids(x):
    """domainIncludes/rangeIncludes/subClassOf come as a dict OR a list of dicts in the
    live feed; normalize to a list of ids."""
    if x is None:
        return []
    if isinstance(x, dict):
        return [x.get("@id")]
    return [e.get("@id") for e in x if isinstance(e, dict)]


def _types(node):
    t = node.get("@type")
    return t if isinstance(t, list) else ([t] if t else [])


def _vocab_shape(vocab):
    classes, props, subclass = [], [], []
    for node in vocab.get("@graph", []):
        ts = _types(node)
        if "rdfs:Class" in ts and "schema:DataType" not in ts:
            classes.append(node["@id"])
            for sup in _ids(node.get("rdfs:subClassOf")):
                if sup:
                    subclass.append((node["@id"], sup))
        elif "rdf:Property" in ts:
            props.append(node)
    return classes, props, subclass


def jsonld_to_readings(vocab):
    """schema.org-style JSON-LD (@graph of rdfs:Class / rdf:Property with possibly
    LIST-valued domainIncludes / rangeIncludes, and rdfs:subClassOf) verbalized as
    canonical FORML — subclass links become the ordinary subtype reading."""
    classes, props, subclass = _vocab_shape(vocab)
    cset = set(classes)
    out = [f"{c} is an entity type." for c in classes]
    out += [f"{sub} is a subtype of {sup}." for (sub, sup) in subclass if sup in cset]
    declared_vts = set()
    for p in props:
        rngs = _ids(p.get("schema:rangeIncludes"))
        for dom in _ids(p.get("schema:domainIncludes")):
            if dom not in cset:
                continue
            obj_rngs = [r for r in rngs if r in cset]
            if obj_rngs:
                out.append(f"{dom} {_local(p['@id'])} {obj_rngs[0]}.")
            elif rngs:
                if p["@id"] not in declared_vts:
                    declared_vts.add(p["@id"])
                    out.append(f"{p['@id']} is a value type.")
                out.append(f"{dom} has {p['@id']}.")
    return "\n".join(out) + "\n"


def jsonld_items_to_readings(items, vocab):
    """Instance items → quoted instance-fact readings through the SAME grammar."""
    classes, props, _sub = _vocab_shape(vocab)
    cset = set(classes)
    bykey = {p["@id"]: p for p in props}
    out = []
    for item in items:
        cls = item.get("@type")
        iid = item.get("@id")
        for key, val in item.items():
            if key.startswith("@") or key not in bykey:
                continue
            rngs = _ids(bykey[key].get("schema:rangeIncludes"))
            obj_rngs = [r for r in rngs if r in cset]
            if obj_rngs:
                out.append(f"{cls} '{iid}' {_local(key)} {obj_rngs[0]} '{val}'.")
            else:
                out.append(f"{cls} '{iid}' has {key} '{val}'.")
    return "\n".join(out) + ("\n" if out else "")


# ============================ GS1 (GPC bricks) and O*NET ======================
def gs1_to_readings(gpc):
    out = ["gs1:Brick is an entity type.", "gs1:Title is a value type.",
           "gs1:Brick has gs1:Title."]
    for b in gpc.get("bricks", []):
        out.append(f"gs1:Brick '{b['code']}' has gs1:Title '{b['title']}'.")
    return "\n".join(out) + "\n"


def onet_to_readings(onet):
    out = ["onet:Occupation is an entity type.", "onet:Title is a value type.",
           "onet:Occupation has onet:Title."]
    for o in onet.get("occupations", []):
        out.append(f"onet:Occupation '{o['code']}' has onet:Title '{o['title']}'.")
    return "\n".join(out) + "\n"


# ============================ fetch-and-store =================================
def fetch_and_store(D, url, fetch=None):
    """Pull the external vocabulary and items, verbalize, ingest through compile_model
    (the same front door as every reading), and record provenance. Returns (D, report)."""
    fetch = fetch or _http_fetch
    payload = fetch(url)
    vocab = payload.get("vocab", payload)
    readings = jsonld_to_readings(vocab) + \
        jsonld_items_to_readings(payload.get("items", []), vocab)
    if D is None:
        D = meta.initial_D()
    D, rep = forml.compile_model(readings, D)
    classes, _props, _sub = _vocab_shape(vocab)
    rows = {tuple(r) for r in system._pop_rows(D, "federatedFrom")} | \
        {(c, url) for c in classes}
    D = _ap(ast.Store("federatedFrom"), _S(to_lam(tuple(sorted(rows))), D))
    return D, rep
