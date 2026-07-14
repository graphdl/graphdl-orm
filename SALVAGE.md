# The salvage ledger

Quarry: canon-first @ 0fa14b7c, read-only (SPEC 14.1). One line per artifact:
source @ commit → disposition, gate evidence. Rejections are listed, never silent
(SPEC 14.3).

| source (quarry) | disposition | gate evidence |
|---|---|---|
| AREST.tex @ 0fa14b7c | adopted verbatim | the source itself (0.1) |
| engine/apps/redo-decision/readings/core.md @ 0fa14b7c | adopted as conformance app | G6 target; verdict ledger 181bc775 |
| engine/shared/arest.canon @ 0fa14b7c | adopted verbatim → /arest.canon | G1: 17 passed (362/345/17, boundary atoms ⊆ the 5 registered prims) |
| engine/python/kernel.py @ 0fa14b7c | adopted → host_py/kernel.py; one adaptation: `.reduce` alias import → direct self-reference in boundary() | G1 harness executes it; μ + BASE + defs store + Cor-6 boundary() |
| engine/python/canon.py @ 0fa14b7c | adopted → host_py/canon.py; one adaptation: shared() resolves the repo ROOT (canon at /arest.canon per D7/PLAN) | G1; binds θ₁ (Codd §2.2), never authors |
| engine/python/tromp.py @ 0fa14b7c | adopted → host_py/tromp.py; lazy imports relativized | it IS the G1 checker (tasks 20/22 lineage) |
| engine/python/__init__.py @ 0fa14b7c | REPLACED: minimal seed (kernel + alias table + canon); the quarry init boots engine/compiler/protocol/tools — Day-3 material, not salvaged blind | SPEC 14.2: host code enters on H-shape only |
| engine/tests/test_tromp.py @ 0fa14b7c | adopted → gates/g1_litmus.py; header swapped to host_py, assertions byte-identical | G1 green 2026-07-14, 17 passed in 5.82s |
| (rebuild addition) system:registered manifest DEF in /arest.canon | authored, not salvaged — SPEC 7.2/8.2: the five boundary prims declared ⟨name, dom, cod, origin⟩; dom/cod transcribed from the quarry impls (engine.py lex/implode/slug/escape_html/strip_prefix) | G1: 18 passed; census 363/346/17; manifest names == boundary atoms |
| engine/python/compiler.py @ 0fa14b7c | adopted → host_py/compiler.py; pyarest imports relativized | paper-order compiles 13/13 zero unparsed; G2 gates the behaviors Day 5 |
| engine/python/engine.py @ 0fa14b7c | adopted → host_py/engine.py (internals: cells, derive, violation expressions, boundary impls); the VERB surface is NOT the gate — SPEC 2.2 gate builds fresh on top | paper-order SM rows exact (from/to/initial); G1 still green |
| engine/python/tools.py @ 0fa14b7c | adopted → host_py/tools.py; pyarest imports relativized | imported by engine/compiler; optimizer + rust seam (Day-4 material) |
| engine/python/protocol.py persist section @ 0fa14b7c | SLICE transcribed → host_py/persist.py: symbol codec, save/load_sqlite, ingest_frozen; fingerprint ADAPTED (host sources + canon + grammar); seal branches dormant (NotImplementedError) | grammar freeze/thaw works; paper-order compile rides it |
| engine/shared/forml2-grammar.md @ 0fa14b7c | adopted → /forml2-grammar.md ("the grammar file is the parser") | ingested by grammar_D; G2 conformance-tests it Day 5 (SPEC 11.1) |
| (rebuild addition) apps/paper-order/readings/core.md | authored: the AREST.tex §1 listing verbatim (unwrapped) | compiles 13/13; G3 conformance app; trigger-reading resolution under investigation |
