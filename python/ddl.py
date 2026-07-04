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
    mandatory = {c[2] for c in cons if len(c) >= 3 and c[1] == "mandatory"}
    return partition, roles, ref, entities, mandatory


def _key_col(name, ref):
    return f"{_sql_name(name)}_{_sql_name(ref.get(name, 'id'))}"


def generate(D):
    """{table-or-ft: CREATE TABLE statement}."""
    partition, roles, ref, entities, mandatory = _analyze(D)
    tables = {}
    own = [ft for ft, key in partition.items() if key == ft]
    # every declared entity gets a table (Halpin: entity types with functional
    # roles group them; one without any still anchors its references)
    entity_tables = entities | ({t for t in partition.values()} - set(own))

    for table in sorted(entity_tables):
        cols = [f"    {_key_col(table, ref)} TEXT PRIMARY KEY"]
        for ft in system.table_columns(partition, table):
            rs = roles.get(ft, [])
            if len(rs) == 1:                                  # absorbed unary: boolean
                stem = _sql_name(ft[len(table):] if ft.startswith(table) else ft)
                cols.append(f"    {stem} BOOLEAN")
                continue
            other = next((t for (_p, t) in rs if t != table), None)
            colname = _sql_name(other) if other else _sql_name(ft)
            null = " NOT NULL" if ft in mandatory else ""
            refs = ""
            if other in entities and other in entity_tables:
                colname = _key_col(other, ref)
                refs = f" REFERENCES {_sql_name(other)}({_key_col(other, ref)})"
            cols.append(f"    {colname} TEXT{null}{refs}")
        tables[table] = (f"CREATE TABLE {_sql_name(table)} (\n"
                         + ",\n".join(cols) + "\n);")

    for ft in sorted(own):
        rs = roles.get(ft, [])
        cols, key, seen = [], [], {}
        for (_pos, player) in rs:
            base = (_key_col(player, ref)
                    if player in entities else _sql_name(player))
            seen[base] = seen.get(base, 0) + 1
            col = base if seen[base] == 1 else f"{base}_{seen[base]}"
            refs = (f" REFERENCES {_sql_name(player)}({_key_col(player, ref)})"
                    if player in entities and player in entity_tables else "")
            cols.append(f"    {col} TEXT NOT NULL{refs}")
            key.append(col)
        stmt = (f"CREATE TABLE {_sql_name(ft)} (\n" + ",\n".join(cols)
                + f",\n    PRIMARY KEY ({', '.join(key)})\n);")
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
        con.execute(stmt.replace("CREATE TABLE", "CREATE TABLE IF NOT EXISTS"))

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
        per_id = {i: {} for i in ids}
        for ft in system.table_columns(partition, table):
            rs = roles.get(ft, [])
            if len(rs) == 1:
                stem = _sql_name(ft[len(table):] if ft.startswith(table) else ft)
                colnames.append(stem)
                members = {r[0] for r in pop(ft) if r}
                for i in ids:
                    per_id[i][stem] = 1 if i in members else 0
                continue
            other = next((t for (_p, t) in rs if t != table), None)
            col = (_key_col(other, ref)
                   if other in entities and other in entity_tables
                   else (_sql_name(other) if other else _sql_name(ft)))
            colnames.append(col)
            val = {r[0]: r[1] for r in pop(ft) if len(r) >= 2}
            for i in ids:
                per_id[i][col] = val.get(i)
        marks = ", ".join("?" for _ in colnames)
        for i in sorted(ids):
            con.execute(
                f"INSERT OR REPLACE INTO {_sql_name(table)} "
                f"({', '.join(colnames)}) VALUES ({marks})",
                [i] + [per_id[i].get(c) for c in colnames[1:]])
        counts[table] = len(ids)

    for ft in sorted(own):
        rs = roles.get(ft, [])
        rows = pop(ft)
        if not rows:
            counts[ft] = 0
            continue
        marks = ", ".join("?" for _ in rs)
        for row in rows:
            con.execute(f"INSERT OR REPLACE INTO {_sql_name(ft)} VALUES ({marks})",
                        list(row[:len(rs)]))
        counts[ft] = len(rows)
    con.commit()
    return counts
