# SQL Dialect Type Mappings

SQL dialect type-mapping vocabulary (#896). Each `SQL Dialect maps Value Type
to SQL Type` row in the Instance Facts section drives one branch of the
dialect-specific DDL emitter in `compile.rs::generate_ddl` — when an RMAP
column has a base type (TEXT / INTEGER / REAL / BOOLEAN), the emitter looks
up the corresponding native SQL type for the active dialect. The lift makes
the mapping data: adding a new value-type row, or a new dialect, is a
readings change with no Rust to touch.

Boot fallback: `SqlTypeMappingTable::boot()` mirrors the same eight dialects
in declaration order so a bare engine (no readings loaded) emits identical
DDL. The unknown-value-type fallback (per-dialect `_ =>` branch in the
legacy match) stays in Rust as a thin default — every dialect's
`_` branch already aliased its `TEXT` row, so the accessor falls through to
the dialect's TEXT mapping on an unknown input.

## Value Types

SQL Dialect is a value type.
  The possible values of SQL Dialect are 'Sqlite', 'PostgreSql', 'MySql', 'SqlServer', 'Oracle', 'Db2', 'Standard', 'ClickHouse'.

Value Type is a value type.
  The possible values of Value Type are 'TEXT', 'INTEGER', 'REAL', 'BOOLEAN'.

SQL Type is a value type.

## Fact Types

SQL Dialect maps Value Type to SQL Type.
  Each SQL Dialect, Value Type combination occurs at most once in the population of SQL Dialect maps Value Type to SQL Type.

## Instance Facts

The 32 rows below mirror the pre-#896 hardcoded nested match in
`generate_ddl`. Order is the same as the legacy code's eight-arm dialect
match × four-arm base-type sub-match so a side-by-side diff stays
readable.

### Sqlite

SQL Dialect 'Sqlite' maps Value Type 'TEXT' to SQL Type 'TEXT'.
SQL Dialect 'Sqlite' maps Value Type 'INTEGER' to SQL Type 'INTEGER'.
SQL Dialect 'Sqlite' maps Value Type 'REAL' to SQL Type 'REAL'.
SQL Dialect 'Sqlite' maps Value Type 'BOOLEAN' to SQL Type 'INTEGER'.

### PostgreSql

SQL Dialect 'PostgreSql' maps Value Type 'TEXT' to SQL Type 'TEXT'.
SQL Dialect 'PostgreSql' maps Value Type 'INTEGER' to SQL Type 'INTEGER'.
SQL Dialect 'PostgreSql' maps Value Type 'REAL' to SQL Type 'DOUBLE PRECISION'.
SQL Dialect 'PostgreSql' maps Value Type 'BOOLEAN' to SQL Type 'BOOLEAN'.

### MySql

SQL Dialect 'MySql' maps Value Type 'TEXT' to SQL Type 'VARCHAR(255)'.
SQL Dialect 'MySql' maps Value Type 'INTEGER' to SQL Type 'INT'.
SQL Dialect 'MySql' maps Value Type 'REAL' to SQL Type 'DOUBLE'.
SQL Dialect 'MySql' maps Value Type 'BOOLEAN' to SQL Type 'TINYINT(1)'.

### SqlServer

SQL Dialect 'SqlServer' maps Value Type 'TEXT' to SQL Type 'NVARCHAR(255)'.
SQL Dialect 'SqlServer' maps Value Type 'INTEGER' to SQL Type 'INT'.
SQL Dialect 'SqlServer' maps Value Type 'REAL' to SQL Type 'FLOAT'.
SQL Dialect 'SqlServer' maps Value Type 'BOOLEAN' to SQL Type 'BIT'.

### Oracle

SQL Dialect 'Oracle' maps Value Type 'TEXT' to SQL Type 'VARCHAR2(255)'.
SQL Dialect 'Oracle' maps Value Type 'INTEGER' to SQL Type 'NUMBER(10)'.
SQL Dialect 'Oracle' maps Value Type 'REAL' to SQL Type 'NUMBER'.
SQL Dialect 'Oracle' maps Value Type 'BOOLEAN' to SQL Type 'NUMBER(1)'.

### Db2

SQL Dialect 'Db2' maps Value Type 'TEXT' to SQL Type 'VARCHAR(255)'.
SQL Dialect 'Db2' maps Value Type 'INTEGER' to SQL Type 'INTEGER'.
SQL Dialect 'Db2' maps Value Type 'REAL' to SQL Type 'DOUBLE'.
SQL Dialect 'Db2' maps Value Type 'BOOLEAN' to SQL Type 'SMALLINT'.

### ClickHouse

SQL Dialect 'ClickHouse' maps Value Type 'TEXT' to SQL Type 'String'.
SQL Dialect 'ClickHouse' maps Value Type 'INTEGER' to SQL Type 'Int64'.
SQL Dialect 'ClickHouse' maps Value Type 'REAL' to SQL Type 'Float64'.
SQL Dialect 'ClickHouse' maps Value Type 'BOOLEAN' to SQL Type 'UInt8'.

### Standard

SQL Dialect 'Standard' maps Value Type 'TEXT' to SQL Type 'CHARACTER VARYING(255)'.
SQL Dialect 'Standard' maps Value Type 'INTEGER' to SQL Type 'INTEGER'.
SQL Dialect 'Standard' maps Value Type 'REAL' to SQL Type 'DOUBLE PRECISION'.
SQL Dialect 'Standard' maps Value Type 'BOOLEAN' to SQL Type 'BOOLEAN'.

### Domain Metadata

Domain 'sql-dialects' has Access 'public'.
Domain 'sql-dialects' has Description 'SQL dialect type-mapping vocabulary. Each SQL Dialect maps Value Type to SQL Type row drives one branch of the dialect-specific DDL emitter in compile.rs::generate_ddl. Lifted from hardcoded Rust per the Sweep-1 dispatch-to-data recipe (#896).'.
