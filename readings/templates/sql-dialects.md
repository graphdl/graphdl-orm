# SQL Dialect Type Mappings

SQL dialect type-mapping vocabulary (#896, broadened in #279 P2b). Each
`SQL Dialect maps Value Type to SQL Type` row in the Instance Facts section
drives one branch of the dialect-specific DDL emitter in
`compile.rs::generate_ddl`. The `Value Type` role plays the part of an
abstract SQL type (the NORMA DCIL "predefined type" layer): the four
original buckets (TEXT / INTEGER / REAL / BOOLEAN) cover untyped RMAP
columns, and the additional SQL-standard abstract types (CHARACTER VARYING,
SMALLINT, BIGINT, DOUBLE PRECISION, DECIMAL, UUID, the BINARY family, DATE /
TIME / TIMESTAMP, …) cover columns whose source noun carries a Conceptual
Data Type — `compile.rs` resolves the CDT to its Abstract SQL Type via the
`readings/core/core.md` catalog (stage one), then looks up the native vendor
type here (stage two). The lift keeps the mapping data: adding a new
value-type row, or a new dialect, is a readings change with no Rust to touch.

Boot fallback: `SqlTypeMappingTable::boot()` mirrors the same eight dialects
in declaration order so a bare engine (no readings loaded) emits identical
DDL. The unknown-value-type fallback (per-dialect `_ =>` branch in the
legacy match) stays in Rust as a thin default — every dialect's
`_` branch already aliased its `TEXT` row, so the accessor falls through to
the dialect's TEXT mapping on an unknown / unmapped input. (That fallback is
also what keeps a CDT whose Abstract SQL Type lacks a row for some dialect
from breaking the emitter — it degrades to that dialect's TEXT type.)

## Value Types

SQL Dialect is a value type.
  The possible values of SQL Dialect are 'Sqlite', 'PostgreSql', 'MySql', 'SqlServer', 'Oracle', 'Db2', 'Standard', 'ClickHouse'.

Value Type is a value type.
  The possible values of Value Type are 'TEXT', 'INTEGER', 'REAL', 'BOOLEAN', 'CHARACTER VARYING', 'CHARACTER', 'CHARACTER LARGE OBJECT', 'SMALLINT', 'BIGINT', 'DOUBLE PRECISION', 'DECIMAL', 'UUID', 'BINARY', 'BINARY VARYING', 'BINARY LARGE OBJECT', 'DATE', 'TIME', 'TIMESTAMP'.

SQL Type is a value type.

## Fact Types

SQL Dialect maps Value Type to SQL Type.
  Each SQL Dialect, Value Type combination occurs at most once in the population of SQL Dialect maps Value Type to SQL Type.

## Instance Facts

The first four rows of each dialect block mirror the pre-#896 hardcoded
nested match in `generate_ddl` (the TEXT / INTEGER / REAL / BOOLEAN
buckets, same order as the legacy eight-arm dialect match × four-arm
base-type sub-match so a side-by-side diff stays readable). The remaining
rows (#279 P2b) map the SQL-standard abstract types that a Conceptual Data
Type resolves to, so every Abstract SQL Type in `readings/core/core.md` has
a native vendor type for each of the eight dialects. INTEGER and BOOLEAN are
both legacy buckets and abstract types, so they appear once per dialect and
serve both roles. uuid / BINARY abstract types degrade to the dialect's
text / blob type where there is no native equivalent.

### Sqlite

Sqlite is dynamically typed (storage classes TEXT / INTEGER / REAL / BLOB /
NUMERIC); the abstract types map onto those classes.

SQL Dialect 'Sqlite' maps Value Type 'TEXT' to SQL Type 'TEXT'.
SQL Dialect 'Sqlite' maps Value Type 'INTEGER' to SQL Type 'INTEGER'.
SQL Dialect 'Sqlite' maps Value Type 'REAL' to SQL Type 'REAL'.
SQL Dialect 'Sqlite' maps Value Type 'BOOLEAN' to SQL Type 'INTEGER'.
SQL Dialect 'Sqlite' maps Value Type 'CHARACTER VARYING' to SQL Type 'TEXT'.
SQL Dialect 'Sqlite' maps Value Type 'CHARACTER' to SQL Type 'TEXT'.
SQL Dialect 'Sqlite' maps Value Type 'CHARACTER LARGE OBJECT' to SQL Type 'TEXT'.
SQL Dialect 'Sqlite' maps Value Type 'SMALLINT' to SQL Type 'INTEGER'.
SQL Dialect 'Sqlite' maps Value Type 'BIGINT' to SQL Type 'INTEGER'.
SQL Dialect 'Sqlite' maps Value Type 'DOUBLE PRECISION' to SQL Type 'REAL'.
SQL Dialect 'Sqlite' maps Value Type 'DECIMAL' to SQL Type 'NUMERIC'.
SQL Dialect 'Sqlite' maps Value Type 'UUID' to SQL Type 'TEXT'.
SQL Dialect 'Sqlite' maps Value Type 'BINARY' to SQL Type 'BLOB'.
SQL Dialect 'Sqlite' maps Value Type 'BINARY VARYING' to SQL Type 'BLOB'.
SQL Dialect 'Sqlite' maps Value Type 'BINARY LARGE OBJECT' to SQL Type 'BLOB'.
SQL Dialect 'Sqlite' maps Value Type 'DATE' to SQL Type 'TEXT'.
SQL Dialect 'Sqlite' maps Value Type 'TIME' to SQL Type 'TEXT'.
SQL Dialect 'Sqlite' maps Value Type 'TIMESTAMP' to SQL Type 'TEXT'.

### PostgreSql

SQL Dialect 'PostgreSql' maps Value Type 'TEXT' to SQL Type 'TEXT'.
SQL Dialect 'PostgreSql' maps Value Type 'INTEGER' to SQL Type 'INTEGER'.
SQL Dialect 'PostgreSql' maps Value Type 'REAL' to SQL Type 'DOUBLE PRECISION'.
SQL Dialect 'PostgreSql' maps Value Type 'BOOLEAN' to SQL Type 'BOOLEAN'.
SQL Dialect 'PostgreSql' maps Value Type 'CHARACTER VARYING' to SQL Type 'varchar'.
SQL Dialect 'PostgreSql' maps Value Type 'CHARACTER' to SQL Type 'char'.
SQL Dialect 'PostgreSql' maps Value Type 'CHARACTER LARGE OBJECT' to SQL Type 'text'.
SQL Dialect 'PostgreSql' maps Value Type 'SMALLINT' to SQL Type 'smallint'.
SQL Dialect 'PostgreSql' maps Value Type 'BIGINT' to SQL Type 'bigint'.
SQL Dialect 'PostgreSql' maps Value Type 'DOUBLE PRECISION' to SQL Type 'double precision'.
SQL Dialect 'PostgreSql' maps Value Type 'DECIMAL' to SQL Type 'numeric'.
SQL Dialect 'PostgreSql' maps Value Type 'UUID' to SQL Type 'uuid'.
SQL Dialect 'PostgreSql' maps Value Type 'BINARY' to SQL Type 'bytea'.
SQL Dialect 'PostgreSql' maps Value Type 'BINARY VARYING' to SQL Type 'bytea'.
SQL Dialect 'PostgreSql' maps Value Type 'BINARY LARGE OBJECT' to SQL Type 'bytea'.
SQL Dialect 'PostgreSql' maps Value Type 'DATE' to SQL Type 'date'.
SQL Dialect 'PostgreSql' maps Value Type 'TIME' to SQL Type 'time'.
SQL Dialect 'PostgreSql' maps Value Type 'TIMESTAMP' to SQL Type 'timestamp'.

### MySql

SQL Dialect 'MySql' maps Value Type 'TEXT' to SQL Type 'VARCHAR(255)'.
SQL Dialect 'MySql' maps Value Type 'INTEGER' to SQL Type 'INT'.
SQL Dialect 'MySql' maps Value Type 'REAL' to SQL Type 'DOUBLE'.
SQL Dialect 'MySql' maps Value Type 'BOOLEAN' to SQL Type 'TINYINT(1)'.
SQL Dialect 'MySql' maps Value Type 'CHARACTER VARYING' to SQL Type 'VARCHAR(255)'.
SQL Dialect 'MySql' maps Value Type 'CHARACTER' to SQL Type 'CHAR(255)'.
SQL Dialect 'MySql' maps Value Type 'CHARACTER LARGE OBJECT' to SQL Type 'LONGTEXT'.
SQL Dialect 'MySql' maps Value Type 'SMALLINT' to SQL Type 'SMALLINT'.
SQL Dialect 'MySql' maps Value Type 'BIGINT' to SQL Type 'BIGINT'.
SQL Dialect 'MySql' maps Value Type 'DOUBLE PRECISION' to SQL Type 'DOUBLE'.
SQL Dialect 'MySql' maps Value Type 'DECIMAL' to SQL Type 'DECIMAL'.
SQL Dialect 'MySql' maps Value Type 'UUID' to SQL Type 'CHAR(36)'.
SQL Dialect 'MySql' maps Value Type 'BINARY' to SQL Type 'BINARY'.
SQL Dialect 'MySql' maps Value Type 'BINARY VARYING' to SQL Type 'VARBINARY(255)'.
SQL Dialect 'MySql' maps Value Type 'BINARY LARGE OBJECT' to SQL Type 'LONGBLOB'.
SQL Dialect 'MySql' maps Value Type 'DATE' to SQL Type 'DATE'.
SQL Dialect 'MySql' maps Value Type 'TIME' to SQL Type 'TIME'.
SQL Dialect 'MySql' maps Value Type 'TIMESTAMP' to SQL Type 'DATETIME'.

### SqlServer

SQL Dialect 'SqlServer' maps Value Type 'TEXT' to SQL Type 'NVARCHAR(255)'.
SQL Dialect 'SqlServer' maps Value Type 'INTEGER' to SQL Type 'INT'.
SQL Dialect 'SqlServer' maps Value Type 'REAL' to SQL Type 'FLOAT'.
SQL Dialect 'SqlServer' maps Value Type 'BOOLEAN' to SQL Type 'BIT'.
SQL Dialect 'SqlServer' maps Value Type 'CHARACTER VARYING' to SQL Type 'NVARCHAR(255)'.
SQL Dialect 'SqlServer' maps Value Type 'CHARACTER' to SQL Type 'NCHAR(255)'.
SQL Dialect 'SqlServer' maps Value Type 'CHARACTER LARGE OBJECT' to SQL Type 'NVARCHAR(MAX)'.
SQL Dialect 'SqlServer' maps Value Type 'SMALLINT' to SQL Type 'SMALLINT'.
SQL Dialect 'SqlServer' maps Value Type 'BIGINT' to SQL Type 'BIGINT'.
SQL Dialect 'SqlServer' maps Value Type 'DOUBLE PRECISION' to SQL Type 'FLOAT'.
SQL Dialect 'SqlServer' maps Value Type 'DECIMAL' to SQL Type 'DECIMAL'.
SQL Dialect 'SqlServer' maps Value Type 'UUID' to SQL Type 'UNIQUEIDENTIFIER'.
SQL Dialect 'SqlServer' maps Value Type 'BINARY' to SQL Type 'BINARY'.
SQL Dialect 'SqlServer' maps Value Type 'BINARY VARYING' to SQL Type 'VARBINARY(255)'.
SQL Dialect 'SqlServer' maps Value Type 'BINARY LARGE OBJECT' to SQL Type 'VARBINARY(MAX)'.
SQL Dialect 'SqlServer' maps Value Type 'DATE' to SQL Type 'DATE'.
SQL Dialect 'SqlServer' maps Value Type 'TIME' to SQL Type 'TIME'.
SQL Dialect 'SqlServer' maps Value Type 'TIMESTAMP' to SQL Type 'DATETIME2'.

### Oracle

SQL Dialect 'Oracle' maps Value Type 'TEXT' to SQL Type 'VARCHAR2(255)'.
SQL Dialect 'Oracle' maps Value Type 'INTEGER' to SQL Type 'NUMBER(10)'.
SQL Dialect 'Oracle' maps Value Type 'REAL' to SQL Type 'NUMBER'.
SQL Dialect 'Oracle' maps Value Type 'BOOLEAN' to SQL Type 'NUMBER(1)'.
SQL Dialect 'Oracle' maps Value Type 'CHARACTER VARYING' to SQL Type 'VARCHAR2(255)'.
SQL Dialect 'Oracle' maps Value Type 'CHARACTER' to SQL Type 'CHAR(255)'.
SQL Dialect 'Oracle' maps Value Type 'CHARACTER LARGE OBJECT' to SQL Type 'CLOB'.
SQL Dialect 'Oracle' maps Value Type 'SMALLINT' to SQL Type 'NUMBER(5)'.
SQL Dialect 'Oracle' maps Value Type 'BIGINT' to SQL Type 'NUMBER(19)'.
SQL Dialect 'Oracle' maps Value Type 'DOUBLE PRECISION' to SQL Type 'BINARY_DOUBLE'.
SQL Dialect 'Oracle' maps Value Type 'DECIMAL' to SQL Type 'NUMBER'.
SQL Dialect 'Oracle' maps Value Type 'UUID' to SQL Type 'RAW(16)'.
SQL Dialect 'Oracle' maps Value Type 'BINARY' to SQL Type 'RAW(2000)'.
SQL Dialect 'Oracle' maps Value Type 'BINARY VARYING' to SQL Type 'RAW(2000)'.
SQL Dialect 'Oracle' maps Value Type 'BINARY LARGE OBJECT' to SQL Type 'BLOB'.
SQL Dialect 'Oracle' maps Value Type 'DATE' to SQL Type 'DATE'.
SQL Dialect 'Oracle' maps Value Type 'TIME' to SQL Type 'TIMESTAMP'.
SQL Dialect 'Oracle' maps Value Type 'TIMESTAMP' to SQL Type 'TIMESTAMP'.

### Db2

SQL Dialect 'Db2' maps Value Type 'TEXT' to SQL Type 'VARCHAR(255)'.
SQL Dialect 'Db2' maps Value Type 'INTEGER' to SQL Type 'INTEGER'.
SQL Dialect 'Db2' maps Value Type 'REAL' to SQL Type 'DOUBLE'.
SQL Dialect 'Db2' maps Value Type 'BOOLEAN' to SQL Type 'SMALLINT'.
SQL Dialect 'Db2' maps Value Type 'CHARACTER VARYING' to SQL Type 'VARCHAR(255)'.
SQL Dialect 'Db2' maps Value Type 'CHARACTER' to SQL Type 'CHAR(255)'.
SQL Dialect 'Db2' maps Value Type 'CHARACTER LARGE OBJECT' to SQL Type 'CLOB'.
SQL Dialect 'Db2' maps Value Type 'SMALLINT' to SQL Type 'SMALLINT'.
SQL Dialect 'Db2' maps Value Type 'BIGINT' to SQL Type 'BIGINT'.
SQL Dialect 'Db2' maps Value Type 'DOUBLE PRECISION' to SQL Type 'DOUBLE'.
SQL Dialect 'Db2' maps Value Type 'DECIMAL' to SQL Type 'DECIMAL'.
SQL Dialect 'Db2' maps Value Type 'UUID' to SQL Type 'CHAR(16) FOR BIT DATA'.
SQL Dialect 'Db2' maps Value Type 'BINARY' to SQL Type 'BINARY'.
SQL Dialect 'Db2' maps Value Type 'BINARY VARYING' to SQL Type 'VARBINARY(255)'.
SQL Dialect 'Db2' maps Value Type 'BINARY LARGE OBJECT' to SQL Type 'BLOB'.
SQL Dialect 'Db2' maps Value Type 'DATE' to SQL Type 'DATE'.
SQL Dialect 'Db2' maps Value Type 'TIME' to SQL Type 'TIME'.
SQL Dialect 'Db2' maps Value Type 'TIMESTAMP' to SQL Type 'TIMESTAMP'.

### ClickHouse

SQL Dialect 'ClickHouse' maps Value Type 'TEXT' to SQL Type 'String'.
SQL Dialect 'ClickHouse' maps Value Type 'INTEGER' to SQL Type 'Int64'.
SQL Dialect 'ClickHouse' maps Value Type 'REAL' to SQL Type 'Float64'.
SQL Dialect 'ClickHouse' maps Value Type 'BOOLEAN' to SQL Type 'UInt8'.
SQL Dialect 'ClickHouse' maps Value Type 'CHARACTER VARYING' to SQL Type 'String'.
SQL Dialect 'ClickHouse' maps Value Type 'CHARACTER' to SQL Type 'String'.
SQL Dialect 'ClickHouse' maps Value Type 'CHARACTER LARGE OBJECT' to SQL Type 'String'.
SQL Dialect 'ClickHouse' maps Value Type 'SMALLINT' to SQL Type 'Int16'.
SQL Dialect 'ClickHouse' maps Value Type 'BIGINT' to SQL Type 'Int64'.
SQL Dialect 'ClickHouse' maps Value Type 'DOUBLE PRECISION' to SQL Type 'Float64'.
SQL Dialect 'ClickHouse' maps Value Type 'DECIMAL' to SQL Type 'Decimal'.
SQL Dialect 'ClickHouse' maps Value Type 'UUID' to SQL Type 'UUID'.
SQL Dialect 'ClickHouse' maps Value Type 'BINARY' to SQL Type 'String'.
SQL Dialect 'ClickHouse' maps Value Type 'BINARY VARYING' to SQL Type 'String'.
SQL Dialect 'ClickHouse' maps Value Type 'BINARY LARGE OBJECT' to SQL Type 'String'.
SQL Dialect 'ClickHouse' maps Value Type 'DATE' to SQL Type 'Date'.
SQL Dialect 'ClickHouse' maps Value Type 'TIME' to SQL Type 'String'.
SQL Dialect 'ClickHouse' maps Value Type 'TIMESTAMP' to SQL Type 'DateTime'.

### Standard

SQL Dialect 'Standard' maps Value Type 'TEXT' to SQL Type 'CHARACTER VARYING(255)'.
SQL Dialect 'Standard' maps Value Type 'INTEGER' to SQL Type 'INTEGER'.
SQL Dialect 'Standard' maps Value Type 'REAL' to SQL Type 'DOUBLE PRECISION'.
SQL Dialect 'Standard' maps Value Type 'BOOLEAN' to SQL Type 'BOOLEAN'.
SQL Dialect 'Standard' maps Value Type 'CHARACTER VARYING' to SQL Type 'CHARACTER VARYING(255)'.
SQL Dialect 'Standard' maps Value Type 'CHARACTER' to SQL Type 'CHARACTER(255)'.
SQL Dialect 'Standard' maps Value Type 'CHARACTER LARGE OBJECT' to SQL Type 'CHARACTER LARGE OBJECT'.
SQL Dialect 'Standard' maps Value Type 'SMALLINT' to SQL Type 'SMALLINT'.
SQL Dialect 'Standard' maps Value Type 'BIGINT' to SQL Type 'BIGINT'.
SQL Dialect 'Standard' maps Value Type 'DOUBLE PRECISION' to SQL Type 'DOUBLE PRECISION'.
SQL Dialect 'Standard' maps Value Type 'DECIMAL' to SQL Type 'DECIMAL'.
SQL Dialect 'Standard' maps Value Type 'UUID' to SQL Type 'CHARACTER(36)'.
SQL Dialect 'Standard' maps Value Type 'BINARY' to SQL Type 'BINARY'.
SQL Dialect 'Standard' maps Value Type 'BINARY VARYING' to SQL Type 'BINARY VARYING'.
SQL Dialect 'Standard' maps Value Type 'BINARY LARGE OBJECT' to SQL Type 'BINARY LARGE OBJECT'.
SQL Dialect 'Standard' maps Value Type 'DATE' to SQL Type 'DATE'.
SQL Dialect 'Standard' maps Value Type 'TIME' to SQL Type 'TIME'.
SQL Dialect 'Standard' maps Value Type 'TIMESTAMP' to SQL Type 'TIMESTAMP'.

### Domain Metadata

Domain 'sql-dialects' has Access 'public'.
Domain 'sql-dialects' has Description 'SQL dialect type-mapping vocabulary. Each SQL Dialect maps Value Type to SQL Type row drives one branch of the dialect-specific DDL emitter in compile.rs::generate_ddl. Lifted from hardcoded Rust per the Sweep-1 dispatch-to-data recipe (#896).'.
