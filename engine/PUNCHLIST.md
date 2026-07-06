# The arest capability inventory (the 0.9.0 merge map)

MERGE PLAN, superseding the earlier nuke framing (Samuel, 2026-07-05):
pyarest is NEVER pushed as its own repo. Its polyglot engine is IMPORTED
INTO arest (C:\Users\lippe\Repos\arest, github.com/graphdl/arest) as the
0.9.0 release. arest is NOT deleted; it absorbs the new engine as
internals and KEEPS its outer shell. So this document is no longer a
deletion gate. It is the interop map: every capability the arest repo
carries, sorted into what the pyarest import REPLACES (the engine
internals) and what arest RETAINS (the outer shell the imported engine
must sit under). A retained-shell entry is not a porting burden; it
persists in arest, and the only work it implies is INTEROP, its binding
calling the new engine's verb surface. Surveyed 2026-07-05 from the arest
tree and docs; each entry's disposition below is being re-read against
this plan (originals kept where still accurate).

RECLASSIFICATION (2026-07-05): REPLACED-by-import are the engine
internals, the old TS and Rust reducer, the compiler and translators,
run_rules, the runtime generators, the derivation; pyarest supplies these
cleaner. RETAINED-shell, persisting in arest and needing only interop,
are entries 1 (Cloudflare Worker), 2 (WASM build), 3 (kernel OS), 4
(FPGA goal), 5 (Solidity), 6 (MCP long tail, joined by the resident), 7
(REST and HATEOAS and OpenAPI and SSE), 7b (ui.do and its React target),
8 (the generator family, now also a pyarest runtime arc), 9 (federation),
12 (the paper), 13 (the docs suite), and 14 (the npm and GitHub identity).
The per-entry text below predates this reclassification in places; the
dispositions that say NOT PORTED now mostly mean RETAINED IN AREST, a
very different thing from a blocker.

## Deployment targets (the portability matrix, docs/11-portability.md)

The old engine defines a five-column primitive target map: Cloudflare
Workers, Local CLI, x86_64 Kernel, WASM browser, and FPGA, with each
primitive (apply, fetch, store, def, compile, freeze/thaw, validate,
derive, query, snapshot/rollback) marked supported, stub, planned, or
architecturally excluded per target.

1. CLOUDFLARE WORKER. Status in the old repo: LIVE AND DEPLOYED (the
   claude app's lesson deploy-live-confirmed records it). The engine
   runs as a WASM module inside a Worker; per-tenant state rides three
   Durable Object classes (EntityDB per entity id, RegistryDB per
   scope, BroadcastDO per scope for SSE); the Worker exposes the full
   v1.0 verb set as REST, an OpenAPI 3.1 manifest, an SSE stream, and
   an MCP endpoint at /mcp and /sse. Sources: docs/cloud.md, wrangler
   config, the deploy script (yarn deploy). Also
   docs/tenant-master-rotation.md for the ops rotation procedure.
   pyarest status: NOT PORTED. No cloud target exists here yet.

2. WASM (browser and Worker builds). Old repo: build:wasm via
   wasm-pack over crates/arest with the cloudflare feature; a
   sanitize-wasm-dts script postprocesses the type declarations; tested
   via cargo test plus vitest including an e2e HATEOAS suite.
   pyarest status: NOT PORTED. The Rust crate here is zero-dependency
   and std-bound; a wasm target has not been attempted.

3. x86_64 KERNEL, THE OS THREAD. Old repo: crates/arest-kernel runs on
   x86_64-unknown-none (bootloader_api, uart_16550, pic8259), boots
   under QEMU (qemu.log, serial.log at the repo root record smokes),
   and esp/EFI holds the EFI system partition image. docs/16-uefi-pivot
   plans the re-target against UEFI so one kernel source tree boots
   x86_64 and aarch64 (laptops, ARM servers, Raspberry Pi 4, QEMU-virt)
   with arch-specific bits isolated below the ExitBootServices handoff.
   pyarest status: NOT PORTED and not started.

4. FPGA. Old repo: a stated design goal (the README's first paragraph:
   designed to lower to FPGA gates eventually) with apply, def,
   validate, and derive marked planned in the portability matrix and
   docs/12-physical-mapping.md carrying the mapping story.
   pyarest status: NOT PORTED; the intersection-source discipline here
   (one canon, four hosts) is the natural on-ramp but no HDL host
   exists.

5. SOLIDITY. Old repo: contracts/ is a complete Foundry project whose
   contracts are GENERATED from readings (contracts/readings/order.md)
   by the AREST Solidity generator; nothing hand-written; forge tests
   and deploy scripts included.
   pyarest status: NOT PORTED. No Solidity generator exists here.

## Server and protocol surfaces

6. THE TYPESCRIPT MCP SERVER. Old repo: src/mcp/server.ts (yarn mcp),
   the daily-driver MCP binding with the full verb set including the
   receipt discipline (context receipts), tutor family, induce, ask,
   propose, select_component, engine_version, apps_check, apps_status,
   apps_register, apps_create, and the p0 per-call app override.
   pyarest status: PARTIALLY SUPERSEDED. The Rust resident plus the
   delegated CLI covers the core verb table (17 tools). The long tail
   (tutor family, induce, ask, propose, select_component, apps_create,
   apps_register, apps_check, apps_status, context receipts) is not
   ported; the standing decision is to port verbs as demanded.

7. REST AND HATEOAS SURFACE. Old repo: the compiled REST API with
   HATEOAS transition links (the README's Hello Order shows it), the
   OpenAPI 3.1 manifest, and the SSE event stream.
   pyarest status: NOT PORTED. The actions verb answers the legal
   transitions (the HATEOAS half as data) but no HTTP server exists.

7b. UI.DO, THE ABSTRACT UI PATTERN (Samuel, 2026-07-05). ui.do lays
    an abstract UI pattern on top of AREST the way iFactr is built on
    MonoCross: the developer authors against the abstraction, and
    per-platform targets realize it. The shipped target is React (the
    web glue at the old repo's apps/ui.do, MIT-licensed with its own
    LICENSE-MIT so applications built on AREST stay unencumbered);
    the OS would ship a Slint target, a Windows app a WPF target, and
    so on per platform. A broader UI workspace also lives at
    C:\Users\lippe\Repos\ui (admin, builder-domains, mdxui.dev, and
    e2e apps), and the iFactr family on disk is the studied prior
    art for the whole pattern.
    pyarest disposition: NOT PORTED and inside the old repo, so the
    nuke would take the React target with it. The iFactr study in
    flight feeds the design language; the ui.do port or re-home is
    its own arc, dependency-wise a leaf on the REST surface (the
    target renders what the verbs answer).

## Engine subsystems catalogued in the old docs

8. GENERATORS (docs/07-generators.md). The old engine generates, per
   compiled model: SQLite DDL, HTML forms, OWL, XSD, EDM, DSL, DTD,
   WSDL, XForms, PLiX, and JSON navigation cells (the claude app's
   cells show the full family: dsl:, dtd:, edm:, html:, owl:, plix:,
   wsdl:, xforms:, xsd:, nav:, resolve:, create:, update:, list:,
   get:, transition: per noun).
   pyarest status: PARTIAL. The SQLite DDL projection exists (ddl
   module, the GraphDL contract); every other generator is not ported.

9. FEDERATION (docs/08-federation.md). Connector pattern: any backend
   is one more Connector declaring its two DEFS names; JS-import
   federation defs existed in the old compile profile.
   pyarest status: DESIGNED FOR, NOT PORTED. The ledger's DEFS-override
   architecture note covers the pattern; no connectors exist.

10. SELF-MODIFICATION (docs/10-self-modification.md), INDUCTION
    (docs/14-induction.md), STAGE-1 TOKENIZATION (docs/13). pyarest
    status: the compile and propose story exists via the grammar
    selfhost; induce is not ported (the long tail); Stage-1 is
    reimplemented via the grammar file discipline.

11. THE UI TOOLKIT DECISION (docs/24-ui-toolkit-decision.md) and the
    HTML generator's form family. pyarest status: NOT PORTED, and the
    decision doc itself should be re-read before any UI work here.

## Documents and artifacts

12. THE PAPER. AREST.tex, AREST.pdf (Compiling Facts into
    Applications), the formal underpinning the whole system cites, at
    the old repo root with its build artifacts. A copy of AREST.pdf
    also sits at C:\Users\lippe\Repos\apps\AREST.pdf and the book
    sources live as _book_*.txt siblings under C:\Users\lippe\Repos.
    pyarest disposition: ARCHIVED 2026-07-05, tracked at paper/
    (source and PDF verbatim with a provenance note). SUPERSEDED at
    the 0.9.0 fold (2026-07-06): the paper lives ONLY at the arest
    repo root (AREST.tex + AREST.pdf, Samuel's live copy); the
    engine/ duplicates were removed. The book
    source siblings under C:\Users\lippe\Repos are WAIVED (Samuel,
    2026-07-05): they are public works, the whitepaper's
    bibliography is the reference of record for them, and his
    personal drive is home enough. The punchlist's first explicit
    waiver.

13. THE DOCS SUITE. Eighteen numbered reference docs (01-introduction
    through 24-ui-toolkit-decision) plus cli.md, cloud.md,
    tenant-master-rotation.md, and the GitHub Pages config. These are
    the system's reference documentation and much of it describes
    behavior pyarest reimplements.
    pyarest disposition: not copied; docs/ here is untracked by policy
    and holds session ledgers, not reference docs. Decide a home.

14. THE NPM PACKAGE IDENTITY. package.json publishes as a graphdl
    package (MIT, repository github.com/graphdl/arest, homepage). The
    npm name, the GitHub org, and the Pages site are externally
    visible artifacts the nuke would orphan.

15. THE OLD ENGINE'S REPORTS AND APPS. _reports/, readings/ at the old
    repo root, apps/ inside the old repo (distinct from
    C:\Users\lippe\Repos\apps), arest.db and paper.db. The live apps
    dir migration is covered by the pyarest manifest; the old repo's
    OWN apps and reports are not.

## Runbook consequence

The swap runbook's step 7 (archive and nuke) gains preconditions: the
paper and any docs Samuel wants live must have a durable home, every
entry above is either ported, re-homed, or explicitly waived by
Samuel, and the archive branch or tag exists. Steps 1 through 6 are
unaffected: the cutover to the resident does not require the punchlist
to be closed, only the deletion does.
