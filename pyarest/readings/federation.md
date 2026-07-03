# Federation — sources, connectors, translators (a standard module)

The federation system is declared in FORML and dispatched through DEFS (the
whitepaper's platform-binding move: one SYSTEM varies by DEFS, not by logic). A Source
is where facts live; a Connector names HOW to reach and read it — its Fetcher and
Translator are DEFINITION NAMES resolved by rho at fetch time, so swapping an
implementation is re-registering a name (IoC through the store). Any backend —
clickhouse, postgresql, sqlite, cloudflare, mongo, a file — is one more Connector
declaring its two names.

Source(.Name) is an entity type.
Connector(.Name) is an entity type.
Url is a value type.
Fetcher is a value type.
Translator is a value type.

Source has Url.
Source uses Connector.
Connector fetches with Fetcher.
Connector translates with Translator.
