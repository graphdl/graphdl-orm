#!/usr/bin/env python3
"""Compile the shared canon into DEFS STORES (spec v3: a host is a store
loader and a mu). Each DEF lands as a CELL row <"CELL", name, term> with the
term in the native encoding (an atom is a scalar, a sequence is an array,
K(x) is the pair ("CONST", x)), the same shape compiled app stores carry
their DEFs in. A host that boots these stores needs no vocabulary binding,
no exec, no include, and no per-host generator: a JSON reader and mu
suffice, which is the point. The stores are the canonical CROSS-HOST
artifact; regenerate whenever shared/*.canon changes (the coverage gate
pins freshness by name).

Replaces java/gen_canon.py, whose per-host wrap this artifact retires."""
import importlib.util
import json
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def _load_pyarest():
    spec = importlib.util.spec_from_file_location(
        "pyarest", os.path.join(ROOT, "python", "__init__.py"),
        submodule_search_locations=[os.path.join(ROOT, "python")])
    m = importlib.util.module_from_spec(spec)
    sys.modules["pyarest"] = m
    spec.loader.exec_module(m)
    return m


def build(src, dst):
    from pyarest import canon as C
    rows = [("CELL", name, obj) for name, obj in C.read_native(src)]
    path = os.path.join(ROOT, "shared", dst)
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        json.dump({"d": rows}, f, ensure_ascii=False)
    return len(rows)


def main():
    _load_pyarest()
    import pyarest.prims  # noqa: F401  (the vocabulary's prim bindings)
    a = build("arest.canon", "canon.store.json")
    s = build("scenarios.canon", "scenarios.store.json")
    print(f"canon.store.json: {a} defs; scenarios.store.json: {s} defs")
    return 0


if __name__ == "__main__":
    sys.exit(main())
