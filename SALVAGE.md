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
