# The flow fixture

`flow.store.json` is a real Registry sidecar, not a hand-written file. It holds
exactly one serve-protocol set_store payload (the keys `d`, `process`,
`overrides`, `cases`), which is the contract `tests/test_store_sidecar.py` pins
and `Registry._sidecar` in `python/protocol.py` writes at every snapshot site.
The Rust resident boots the app by feeding this file through the same ingestion
path a `--serve` stdin line takes.

The store compiles this model with a base-free Registry and then applies one
fact to the declared fact type, which the partition routes into the absorbed
layout (the entity cell `Ticket:t1` plus the `Ticket` index), beside the
per-fact-type append cell:

    Status is a value type.
    Note is a value type.
    Ticket is an entity type.
    Ticket has Status.
    Ticket has Note.
    Each Ticket has at most one Status.
    Each Ticket has at most one Note.

    apply("flow", "Ticket_has_Status", ("t1", "open"))

The uniqueness constraints make both fact types functional, so the compile
materializes the `rmapColumns` layout cell, and the canonical
`system:verbalize` dispatches population fetches through it. That is what the
mcp test's synthesize assertion pins: real reading pairs over the absorbed
layout, with the never-written Note fact type contributing nothing instead of
bottoming the fold.

## Regenerating

The store content depends on the engine version, so regenerate the file
whenever the compiler or the sidecar contract changes. Run this from the
repository root:

    python - <<'EOF'
    import conftest  # noqa: F401  (registers the pyarest package from python/)
    import pathlib, shutil, tempfile
    import pyarest.prims  # noqa: F401
    from pyarest import apps

    MODEL = ("Status is a value type.\nNote is a value type.\n"
             "Ticket is an entity type.\n"
             "Ticket has Status.\nTicket has Note.\n"
             "Each Ticket has at most one Status.\n"
             "Each Ticket has at most one Note.\n")
    work = pathlib.Path(tempfile.mkdtemp())
    ap = work / "apps"
    (ap / "flow" / "readings").mkdir(parents=True)
    (ap / "flow" / "readings" / "app.md").write_text(MODEL, encoding="utf-8")
    reg = apps.Registry(str(ap), cache_dir=str(work / "fz"))
    reg.compile("flow")
    receipt = reg.apply("flow", "Ticket_has_Status", ("t1", "open"))
    assert receipt["committed"], receipt
    dest = pathlib.Path("rust/tests/fixtures/apps/flow")
    dest.mkdir(parents=True, exist_ok=True)
    shutil.copy(str(ap / "flow" / "flow.store.json"), str(dest / "flow.store.json"))
    print("wrote", dest / "flow.store.json")
    EOF

After regenerating, re-run `cargo test --test mcp` from `rust/` and update any
row or cell assertions that the new engine output moved.
