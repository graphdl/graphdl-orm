"""Migration from an old-engine app .db (the swap tool). The old store rides the
`cells` table as displayed Objects; two encodings appear in live dbs — the keyed
map '{k=<<Role, value>, ...>>, ...}' and the keyless tuple sequence
'<<<Role, value>, ...>>, <<...>>, ...>' (m:n cells) — with the escape alphabet of
the old ast.rs escape_atom_for_display: a backslash escapes each of \\ < > , { }
= inside atom text. Values may quote structural characters (prose descriptions
carry markup), so parsing MASKS every escaped character into the private-use
plane, scans structure, and proves itself by exact round trip; a cell that fails
the proof is reported, never guessed at.

Populations classify against the NEW model: asserted fact types migrate as BATCH
log entries (one derive pass — the old engine's own atomic collection apply is
the precedent; per-row validated creates would cost hours on the big apps),
derived fact types are never replayed — the engine rederives them and the report
VERIFIES old versus new row sets, which is the migration's parity evidence.
Cells the model does not declare are reported unknown."""
import json
import os
import re
import sqlite3

from . import persist, system

# ---- the cells encoding ----
_MASKED = re.compile(r"\\(.)", re.S)
_UNMASKED = re.compile("\x00(.)", re.S)
# a pair '<Role, value>': role names carry no comma or angle; the value is lazy
# up to a '>' that is followed by another pair, a close, or the end
_PAIR = re.compile(r"<([^,<>]+), (.*?)>(?=, <|>|$)", re.S)


def _mask(s):
    """\\X -> NUL + private-use(X): escaped characters leave the structural
    alphabet entirely (a merely NUL-prefixed '>' would still anchor the scan)."""
    return _MASKED.sub(lambda m: "\x00" + chr(0xE000 + ord(m.group(1))), s)


def _unmask(s):
    return _UNMASKED.sub(lambda m: chr(ord(m.group(1)) - 0xE000), s)


def _parse_pairs(text):
    pairs, pos = [], 0
    for m in _PAIR.finditer(text):
        if m.start() != pos:
            return None
        pairs.append((m.group(1), m.group(2)))
        pos = m.end()
        if text[pos:pos + 2] == ", ":
            pos += 2
    if pos != len(text):
        return None
    return tuple(pairs)


def parse_cell(contents):
    """contents -> [(key-or-None, ((role, value), ...)), ...] with values
    unescaped, or None when the round-trip proof fails."""
    masked = _mask(contents)
    out = _parse_masked(masked)
    if out is None:
        return None
    return [(None if k is None else _unmask(k),
             tuple((_unmask(r), _unmask(v)) for (r, v) in ps))
            for (k, ps) in out]


def _parse_masked(contents):
    s = contents.strip()
    if s.startswith("<<<") and s.endswith(">>>"):
        # the keyless SEQUENCE of tuples: '<T1, T2, ...>', each Ti '<<R, v>, ...>'
        body = s[1:-1]
        entries, pos = [], 0
        for m in re.finditer(r"<(<.*?>)>(?=, <<|$)", body, re.S):
            if m.start() != pos:
                return None
            pairs = _parse_pairs(m.group(1))
            if pairs is None:
                return None
            entries.append((None, pairs))
            pos = m.end()
            if body[pos:pos + 2] == ", ":
                pos += 2
        if pos != len(body):
            return None
        rebuilt = "<" + ", ".join(
            "<" + ", ".join(f"<{r}, {v}>" for (r, v) in ps) + ">"
            for (_k, ps) in entries) + ">"
        return entries if rebuilt == s else None
    if not (s.startswith("{") and s.endswith("}")):
        return None
    inner = s[1:-1]
    if not inner:
        return []
    starts = [m.start() for m in re.finditer(r"=<<", inner)]
    if not starts:
        return None
    keys, bodies, prev_end = [], [], 0
    for i, st in enumerate(starts):
        key = inner[prev_end:st]
        if i > 0:
            if not key.startswith(", "):
                return None
            key = key[2:]
        keys.append(key)
        body_start = st + len("=<<")
        end = starts[i + 1] if i + 1 < len(starts) else len(inner)
        seg = inner[body_start:end]
        close = seg.rfind(">>")
        if close < 0:
            return None
        bodies.append(seg[:close])
        prev_end = body_start + close + 2
    if prev_end != len(inner):
        return None
    entries = []
    for key, body in zip(keys, bodies):
        pairs = _parse_pairs("<" + body + ">")
        if pairs is None:
            return None
        entries.append((key, pairs))
    rebuilt = "{" + ", ".join(
        k + "=<" + ", ".join(f"<{r}, {v}>" for (r, v) in ps) + ">"
        for (k, ps) in entries) + "}"
    return entries if rebuilt == s else None


# ---- classification and replay ----
def read_cells(db_path):
    con = sqlite3.connect(db_path)
    try:
        return dict(con.execute("SELECT name, contents FROM cells").fetchall())
    finally:
        con.close()


def plan(D, cells):
    """Classify the old cells against the compiled model: asserted (rows to
    migrate, in role order), derived-with-a-rule (kept aside for verification —
    the engine rederives), STORED STATE (marked derived but NO rule derives it:
    the old base's own comments record the engine's imperative writers owning
    such cells, e.g. State_Machine_is_for_Resource after its underspecified rule
    was removed — these migrate as data), unknown and unparsed (reported)."""
    fts = {f[0] for f in system._pop_rows(D, "factType") if f}
    kinds = {r[0]: r[1] for r in system._pop_rows(D, "derivation")
             if len(r) >= 2}
    ruled = {r[1] for r in system._pop_rows(D, "ruleDerives") if len(r) >= 2}
    out = {"asserted": {}, "derived": {}, "stored_state": [],
           "unknown": [], "unparsed": []}
    for name, contents in cells.items():
        parsed = parse_cell(contents or "{}")
        if parsed is None:
            out["unparsed"].append(name)
            continue
        rows = [tuple(v for (_r, v) in ps) for (_k, ps) in parsed]
        if kinds.get(name) == "fully-derived" and name in ruled:
            # a PURE derivation: the engine rederives it; the report verifies
            out["derived"][name] = rows
        elif name in fts or name in kinds:
            # asserted, or derived-and-stored/semiderived/ruled-but-plain: the
            # old engine's imperative writers own such populations — data
            out["asserted"][name] = rows
            if name in kinds or name in ruled:
                out["stored_state"].append(name)
        else:
            out["unknown"].append(name)
    return out


def _prose_like(value):
    """Sentence-shaped content where an atomic value belongs: several words
    with sentence punctuation, or outright paragraph length. The heuristic is
    deliberately conservative — the audit flags for re-authoring at swap time,
    it never blocks."""
    if not isinstance(value, str):
        return False
    words = value.split()
    if len(value) > 160:
        return True
    return len(words) >= 6 and any(m in value for m in (". ", "; ", ", "))


def audit_authoring(plan_out, D=None):
    """The mis-authoring audit: prose crammed into VALUES (catch-all text
    fields), prose used as IDS (the first role of a row is the reference; a
    sentence there is an authoring defect), and prose ENUM MEMBERS in the
    readings' possible-values declarations. Answers {cell, kind, count,
    sample} findings — the swap-time cleanup list, never a block."""
    findings = []
    for ft, rows in sorted(plan_out["asserted"].items()):
        hits_v = [r for r in rows if any(_prose_like(v) for v in r[1:])]
        hits_i = [r for r in rows if r and isinstance(r[0], str)
                  and len(r[0].split()) >= 5]
        if hits_v:
            findings.append({"cell": ft, "kind": "prose_value",
                             "count": len(hits_v),
                             "sample": str(hits_v[0])[:160]})
        if hits_i:
            findings.append({"cell": ft, "kind": "prose_id",
                             "count": len(hits_i),
                             "sample": str(hits_i[0][0])[:160]})
    if D is not None:
        for r in system._pop_rows(D, "valueConstraint"):
            if len(r) >= 2 and isinstance(r[1], str):
                members = re.findall(r"'([^']*)'", r[1])
                bad = [m for m in members
                       if _prose_like(m) or len(m.split()) >= 6]
                if bad:
                    findings.append({"cell": r[0], "kind": "prose_enum",
                                     "count": len(bad),
                                     "sample": bad[0][:160]})
    return findings


def replay_into(registry, app, old_db):
    """Migrate an old .db's asserted populations into the app as BATCH log
    entries, recompile (the log replays through the same path every compile
    after), and answer the report — including the derived-population
    verification, old engine versus this one."""
    if os.path.abspath(old_db) == os.path.abspath(registry._db(app)):
        raise ValueError("old_db is the app's own database: compiling would "
                         "overwrite the source cells before they are read — "
                         "pass a snapshot copy")
    registry.compile(app)
    D = registry._load(app)
    p = plan(D, read_cells(old_db))
    log = os.path.join(registry._app_dir(app), f"{app}.events.jsonl")
    with open(log, "a", encoding="utf-8") as f:
        for ft, rows in sorted(p["asserted"].items()):
            if rows:
                f.write(json.dumps({"op": "migrate", "ft": ft,
                                    "facts": [list(r) for r in rows]},
                                   ensure_ascii=False) + "\n")
    registry.compile(app)
    D = registry._load(app)
    verify = {}
    for ft, old_rows in sorted(p["derived"].items()):
        # compare as STRINGS: the old cells serialize every value as text, and
        # an aggregate this engine derives is a number (Fact_Type_has_Arity)
        new_rows = {tuple(str(x) for x in r) for r in system._pop_rows(D, ft)}
        old_set = {tuple(str(x) for x in r) for r in old_rows}
        verify[ft] = {"old": len(old_set), "new": len(new_rows),
                      "match": old_set == new_rows,
                      "missing": sorted(old_set - new_rows)[:5],
                      "extra": sorted(new_rows - old_set)[:5]}
    return {"migrated": {ft: len(rows) for ft, rows in p["asserted"].items()
                         if rows},
            "stored_state": sorted(p["stored_state"]),
            "verify": verify, "unknown": sorted(p["unknown"]),
            "unparsed": sorted(p["unparsed"]),
            "authoring": audit_authoring(p, D)}
