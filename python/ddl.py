"""The DDL generator: Halpin's Rmap output as SQL (the GraphDL lineage's day job).
The grouping comes from rmap_partition (book 10.3: spanning or absent UC keeps a
fact type its own table, a single-role UC absorbs it into the role-1 player's
table); this module renders it as CREATE TABLE (book 11.12): the reference scheme
as the primary key, absorbed functional fact types as columns with NOT NULL
exactly where a mandatory constraint holds, BOOLEAN columns for absorbed unaries,
per-role REFERENCES to entity tables, and spanning keys on own-table fact types.
Fix-not-inherit: a nullable column REFERENCES without NOT NULL, so an incomplete
entity can never cascade valid rows out of a projection."""
import re

from . import system


def _sql_name(name):
    s = re.sub(r"[^0-9A-Za-z]+", "_", name).strip("_").lower()
    return s or "t"


def _q(name):
    """Every emitted identifier is quoted: the base metamodel projects tables named
    constraint, transition, view — SQL reserved words the old .db also carries."""
    return '"' + name + '"'


def _analyze(D):
    partition = system.rmap_partition(D)
    roles = {}
    for r in system._pop_rows(D, "role"):
        if len(r) >= 4:
            roles.setdefault(r[1], []).append((r[2], r[3]))
    for ft in roles:
        roles[ft].sort()
    ref = {r[0]: r[1] for r in system._pop_rows(D, "refScheme") if len(r) >= 2}
    for r in system._pop_rows(D, "refMode"):                  # Person(.nr): the ref mode
        if len(r) >= 2:
            ref.setdefault(r[0], r[1])
    entities = {r[0] for r in system._pop_rows(D, "instanceOf")
                if len(r) >= 2 and r[1] == "ObjectType"}
    cons = system._pop_rows(D, "constraint")
    mandatory = {}
    for c in cons:
        if len(c) >= 4 and c[1] == "mandatory":
            mandatory.setdefault(c[2], set()).add(c[3])       # ft -> mandated players
    return partition, roles, ref, entities, mandatory


def _key_col(name, ref):
    return f"{_sql_name(name)}_{_sql_name(ref.get(name, 'id'))}"


def _entity_columns(table, partition, roles, ref, entities, entity_tables):
    """The ordered absorbed columns of an entity table: (ft, column, kind, other)
    with kind in unary/value/ref. One naming pass shared by generate and project
    (they must never disagree), deduped with the position suffix the own-table
    branch uses — the base metamodel absorbs two Status roles into transition and
    two Texts into constraint."""
    out, seen = [], {}
    for ft in system.table_columns(partition, table):
        rs = roles.get(ft, [])
        if len(rs) == 1:
            base = _sql_name(ft[len(table):] if ft.startswith(table) else ft)
            kind, other = "unary", None
        else:
            other = next((t for (_p, t) in rs if t != table), None)
            if other in entities and other in entity_tables:
                base, kind = _key_col(other, ref), "ref"
            else:
                base, kind = (_sql_name(other) if other else _sql_name(ft)), "value"
        seen[base] = seen.get(base, 0) + 1
        col = base if seen[base] == 1 else f"{base}_{seen[base]}"
        out.append((ft, col, kind, other))
    return out


def generate(D):
    """{table-or-ft: CREATE TABLE statement}."""
    partition, roles, ref, entities, mandatory = _analyze(D)
    tables = {}
    own = [ft for ft, key in partition.items() if key == ft]
    # every declared entity gets a table (Halpin: entity types with functional
    # roles group them; one without any still anchors its references)
    entity_tables = entities | ({t for t in partition.values()} - set(own))

    for table in sorted(entity_tables):
        cols = [f"    {_q(_key_col(table, ref))} TEXT PRIMARY KEY"]
        for (ft, col, kind, other) in _entity_columns(
                table, partition, roles, ref, entities, entity_tables):
            if kind == "unary":                               # absorbed unary: boolean
                cols.append(f"    {_q(col)} BOOLEAN")
                continue
            # Halpin 11.12: the column hardens only when the MANDATED player is
            # this table (a mandatory on the other role never forces this column)
            null = " NOT NULL" if table in mandatory.get(ft, ()) else ""
            refs = ("" if kind != "ref" else
                    f" REFERENCES {_q(_sql_name(other))}({_q(_key_col(other, ref))})")
            cols.append(f"    {_q(col)} TEXT{null}{refs}")
        tables[table] = (f"CREATE TABLE {_q(_sql_name(table))} (\n"
                         + ",\n".join(cols) + "\n);")

    for ft in sorted(own):
        rs = roles.get(ft, [])
        if not rs:                                            # no roles, no relational shape
            continue
        cols, key, seen = [], [], {}
        for (_pos, player) in rs:
            base = (_key_col(player, ref)
                    if player in entities else _sql_name(player))
            seen[base] = seen.get(base, 0) + 1
            col = base if seen[base] == 1 else f"{base}_{seen[base]}"
            refs = (f" REFERENCES {_q(_sql_name(player))}({_q(_key_col(player, ref))})"
                    if player in entities and player in entity_tables else "")
            cols.append(f"    {_q(col)} TEXT NOT NULL{refs}")
            key.append(col)
        stmt = (f"CREATE TABLE {_q(_sql_name(ft))} (\n" + ",\n".join(cols)
                + f",\n    PRIMARY KEY ({', '.join(_q(c) for c in key)})\n);")
        tables[ft] = stmt
    return tables


def script(D):
    """The whole schema as one executable document, entities before references."""
    return "\n\n".join(generate(D).values())


def project(D, con):
    """Create the schema and POPULATE it from the store. Entity rows are the ids
    playing the entity's roles anywhere (the reference scheme's population,
    derived); absorbed functional fact types fill columns and absorbed unaries fill
    booleans, with an absent value projecting NULL — the row stays (the dangling-FK
    cascade is impossible by construction). Own-table fact types insert row per
    fact. Answers {table: rowcount}."""
    partition, roles, ref, entities, mandatory = _analyze(D)
    own = [ft for ft, key in partition.items() if key == ft]
    entity_tables = entities | ({t for t in partition.values()} - set(own))
    for stmt in generate(D).values():
        # the projection is SOFT where generate is hard (the old engine's
        # projected tables are a data mirror: no NOT NULL beyond the keys), so
        # a migrated population missing a mandatory value lands as a NULL row
        # instead of crashing the compile — visibility over cascade
        stmt = stmt.replace(" TEXT NOT NULL", " TEXT")
        con.execute(stmt.replace("CREATE TABLE", "CREATE TABLE IF NOT EXISTS"))

    def ensure_columns(table, colnames, coltypes):
        # schema evolution on a live db: IF NOT EXISTS never revisits an
        # existing table, so a later compile's new absorbed fact types ALTER
        # in, typed as generate types them (BOOLEAN unaries, TEXT values).
        # Columns the model no longer declares stay behind untouched — the
        # mirror is soft both ways.
        have = {r[1] for r in con.execute(
            f"PRAGMA table_info({_q(_sql_name(table))})")}
        for c in colnames:
            if c not in have:
                con.execute(f"ALTER TABLE {_q(_sql_name(table))} ADD COLUMN "
                            f"{_q(c)} {coltypes.get(c, 'TEXT')}")

    counts = {}
    pops = {}

    def pop(ft):
        if ft not in pops:
            pops[ft] = [tuple(r) for r in system._pop_rows(D, ft)]
        return pops[ft]

    for table in sorted(entity_tables):
        # the derived entity population: every id the entity's roles mention
        ids = set()
        for ft, rs in roles.items():
            for (p, player) in rs:
                if player == table:
                    for row in pop(ft):
                        if len(row) >= p:
                            ids.add(row[p - 1])
        for row in pop(table):                                # plus its own cell
            if row:
                ids.add(row[0])
        colnames = [_key_col(table, ref)]
        coltypes = {}
        per_id = {i: {} for i in ids}
        for (ft, col, kind, _other) in _entity_columns(
                table, partition, roles, ref, entities, entity_tables):
            colnames.append(col)
            coltypes[col] = "BOOLEAN" if kind == "unary" else "TEXT"
            if kind == "unary":
                members = {r[0] for r in pop(ft) if r}
                for i in ids:
                    per_id[i][col] = 1 if i in members else 0
                continue
            val = {r[0]: r[1] for r in pop(ft) if len(r) >= 2}
            for i in ids:
                per_id[i][col] = val.get(i)
        ensure_columns(table, colnames, coltypes)
        marks = ", ".join("?" for _ in colnames)
        for i in sorted(ids):
            con.execute(
                f"INSERT OR REPLACE INTO {_q(_sql_name(table))} "
                f"({', '.join(_q(c) for c in colnames)}) VALUES ({marks})",
                [i] + [per_id[i].get(c) for c in colnames[1:]])
        counts[table] = len(ids)

    for ft in sorted(own):
        rs = roles.get(ft, [])
        if not rs:
            # a fact type with no role rows (a reading over undeclared types) has
            # no relational mapping: named None, never malformed SQL
            counts[ft] = None
            continue
        rows = pop(ft)
        if not rows:
            counts[ft] = 0
            continue
        marks = ", ".join("?" for _ in rs)
        for row in rows:
            con.execute(f"INSERT OR REPLACE INTO {_q(_sql_name(ft))} VALUES ({marks})",
                        list(row[:len(rs)]))
        counts[ft] = len(rows)
    con.commit()
    return counts
