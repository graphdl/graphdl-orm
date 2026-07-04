# Security

## Description
SSRF defense vocabulary (#25, #894). Each `CIDR Block` row in the
instance fact list below is a network range that External System URLs
must NOT resolve to. The deontic constraint below makes the policy
explicit; the engine reads the CIDR list at platform_compile time and
rejects any `External System has URL` whose host sits inside one of
the listed blocks.

Before #894 this list lived as a `forbidden_v4 = a == 127 || …` chain
in `crates/arest/src/parse_forml2.rs::is_forbidden_url`. The Sweep-1
dispatch-to-data lift moves it here so operators can add or retract
ranges without touching Rust — e.g. a tenant operating inside RFC 6598
(`100.64.0.0/10`, carrier-grade NAT) can add that prefix as one extra
instance fact and the next compile re-derives the blocklist.

The `cidr_contains` Platform Func (`crates/arest/src/ast.rs`) is the
membership predicate. Both the engine's SSRF check and any app's own
access-control derivation rules call it; one implementation, both
surfaces.

## Entity Types

CIDR Block(.id) is an entity type.

## Value Types

Block Kind is a value type.
  The possible values of Block Kind are 'internal-loopback', 'private-rfc1918', 'link-local', 'ipv6-loopback', 'ipv6-link-local', 'ipv6-unique-local'.

## Fact Types

### CIDR Block
CIDR Block has Block Kind.
  Each CIDR Block has exactly one Block Kind.

## Deontic Constraints

### SSRF Blocklist

It is forbidden that External System URL resolves to host in CIDR Block.

## Instance Facts

The eight CIDR Block entries below mirror the pre-#894 hardcoded
IPv4/IPv6 dispatch in `is_forbidden_url`. Each row's `Block Kind`
documents the rationale; `cidr_contains` only reads the `.id` (the
CIDR string itself). Order is the same as the legacy code's branch
ordering so a row-by-row audit between the Rust source and this list
is straightforward.

'127.0.0.0/8'    has Block Kind 'internal-loopback'.
'10.0.0.0/8'     has Block Kind 'private-rfc1918'.
'169.254.0.0/16' has Block Kind 'link-local'.
'192.168.0.0/16' has Block Kind 'private-rfc1918'.
'172.16.0.0/12'  has Block Kind 'private-rfc1918'.
'::1/128'        has Block Kind 'ipv6-loopback'.
'fe80::/10'      has Block Kind 'ipv6-link-local'.
'fc00::/7'       has Block Kind 'ipv6-unique-local'.

Domain 'security' has Access 'public'.
Domain 'security' has Description 'SSRF defense vocabulary. CIDR Block entries are the data the engine reads at platform_compile time to reject External System URLs resolving to internal/loopback/link-local hosts. Lifted from hardcoded Rust per the Sweep-1 dispatch-to-data recipe (#894).'.
