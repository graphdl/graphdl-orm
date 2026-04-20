# 08 · Federation

Some facts are not yours. Customer records live in your CRM, payments live in Stripe, auth tokens live in an identity provider. Federation is how arest treats those external systems as first-class fact sources without copying their data.

## Declaring an external system

An External System gets a URL, an auth header, and an optional prefix:

```forml2
External System 'stripe' has URL 'https://api.stripe.com/v1'.
External System 'stripe' has Header 'Authorization'.
External System 'stripe' has Prefix 'Bearer'.
```

Then point one or more nouns at it:

```forml2
Noun 'Stripe Customer' is backed by External System 'stripe'.
Noun 'Stripe Customer' has URI '/customers'.

Noun 'Stripe Invoice' is backed by External System 'stripe'.
Noun 'Stripe Invoice' has URI '/invoices'.
```

The compiler emits a `populate:{noun}` def for each backed noun containing the URL, URI path, header, prefix, noun name, and role names. The MCP server reads this def when a query targets the noun and issues the HTTP call.

## How fetch works

When an MCP `get Stripe Customer` request arrives:

1. The server fetches the `populate:Stripe Customer` config from `D`.
2. It constructs the URL: `https://api.stripe.com/v1/customers` (or `.../customers/{id}` for a specific entity).
3. It looks up the credential: `AREST_SECRET_STRIPE` from the environment.
4. It sets the auth header: `Authorization: Bearer {secret}`.
5. It issues the HTTPS GET.
6. It maps the JSON response fields to role bindings using the noun's fact types.

The fetched facts enter `P` under OWA (Open World Assumption). Their absence is not a violation; only their presence against a deontic constraint counts as a violation. Alethic constraints that span an OWA noun are treated as sound-but-not-complete.

## Credentials

Environment variable naming: `AREST_SECRET_{SYSTEM}` where `{SYSTEM}` is the external system name uppercased with non-alphanumerics replaced by underscores.

```bash
AREST_SECRET_STRIPE='sk_test_xxxx'
AREST_SECRET_AUTH_VIN='your-api-key'
```

Secrets are never stored in readings or state. If the environment variable is missing, the `Authorization` header is omitted. For systems that do not require auth, you can leave the env var unset.

## SSRF protection

At compile time, the engine validates every External System URL against an SSRF denylist. Internal (RFC 1918), loopback, and link-local addresses are rejected. Attempting to declare:

```forml2
External System 'evil' has URL 'http://169.254.169.254/latest/meta-data'.
```

is caught during compile, not at runtime. The check runs once when the External System instance fact is parsed.

## Caching and consistency

Cell isolation (the paper's Definition 2) ensures federated facts are cached for the request duration. Two concurrent `μ` applications do not re-fetch. Between requests, the cache is invalidated; every new request fetches fresh data unless the generator (or the HTTP layer) caches the response.

There is no cross-request cache by default. If you need one, attach an HTTP caching middleware (like Cloudflare's) in front of the populate endpoint.

## Unified queries

Once federated, a noun behaves like any other. Constraints span local and federated facts:

```forml2
Subscription belongs to Stripe Customer.
  Each Subscription belongs to exactly one Stripe Customer.
If some User has Email and some Stripe Customer has Email that matches then that User is linked to that Stripe Customer.
```

Derivation rules join across:

```forml2
* User has payment method iff User owns Stripe Customer and that Stripe Customer has some Payment Method.
```

The engine does not distinguish local from federated at query time. It fetches the federated data when needed, joins it with the local population, and evaluates the rule as usual.

## Provenance via Citation (#305)

Every fact produced by `ρ(populate_n)` carries a paired `Citation` record. An LLM reading the API — or a constraint evaluated over `P` — can ask *where did this fact come from?* and get a queryable answer without the engine having to invent a side-channel. The Citation is itself a fact in `P`, so Theorem 5 continues to hold: every value in the REST representation is produced by a ρ-application, including the Citation record.

Each federated fetch emits one Citation:

```forml2
Citation 'cite:<hash>' has URI 'https://api.stripe.com/v1/customers/cus_1'.
Citation 'cite:<hash>' has Retrieval Date '2026-04-20T12:00:00Z'.
Citation 'cite:<hash>' has Authority Type 'Federated-Fetch'.
Citation 'cite:<hash>' is backed by External System 'stripe'.
```

All facts returned by a given fetch share the same Citation (they came from the same response at the same moment). The caller — the MCP server's absorb path, or the engine's populate arm — emits paired `Fact cites Citation` links so each entity fact points back at its origin.

The `Authority Type` enum includes two provenance kinds:

- **`'Federated-Fetch'`** — emitted here, by `federatedFetch` in `src/mcp/federation.ts`.
- **`'Runtime-Function'`** — emitted by the Rust engine's `ast::emit_citation_fact` when a runtime-registered Platform function (e.g., `send_email`, `httpFetch`, an ML scorer) produces a fact. See *IoC/DI (↓DEFS)* below.

A domain can declare obligations over provenance the same way it declares any other deontic constraint:

```forml2
It is obligatory that each Fact of Fact Type 'ML Score' cites some Citation
  where that Citation has Authority Type 'Runtime-Function'.
```

The enforcement is the usual restriction over `P`. No new mechanism.

### Citation id scheme

Citation ids are content-addressed over `(URI, Authority Type, Retrieval Date)`. Two emissions for the same triple yield the same id, so repeated absorption of the same response is idempotent at the cell level.

### HATEOAS

The `_links.citations` relation on every federated-noun response points at the Citation collection for the entity. Clients walking the link graph can reach the provenance chain without special-casing.

## IoC/DI (↓DEFS) and the platform layer

The paper's §3.2 Platform Binding names two writers to `DEFS`:

- **Compile** writes the **domain layer** from FORML 2 readings. Covered elsewhere in this guide.
- **Runtime** writes the **platform layer** via `↓DEFS` — each runtime (browser, server, storage backend) registers the functions it owns. The paper's canonical examples are `httpFetch`, `upsert`, `notify`, `render`.

On the Rust engine side, the runtime writer is `ast::register_runtime_fn(name, func, state)`. It pushes `(name, func)` into `DEFS` and records `name` in a `runtime_registered_names` cell. Dispatch via `apply(Func::Def(name), …)` is uniform with compile-derived bindings; the registry cell is the origin marker that `emit_citation_fact` uses to produce the `Runtime-Function` Citation.

The federation path in `src/mcp/federation.ts` is the TS-side analogue — it runs *outside* the synchronous engine because HTTP is async, but the facts it absorbs into `P` carry the same Citation shape the engine would produce for its own runtime-registered primitives.

An open gap: `apply_platform` in `crates/arest/src/ast.rs` currently hardcodes its match over Platform names. Future registrations via `register_runtime_fn` that need engine-native dispatch (synchronous, composable in Func trees) will want a fallback arm that resolves the registered Func body — see `_reports/e3-gap-analysis-2026-04-20.md` for the narrative.

## Federated analytics backends (#219)

The fetch path above loads a single entity from its source. The
same External System mechanism covers read-heavy OLAP queries —
list, count, aggregate — for nouns whose volume makes per-cell
reads impractical. This is the paper's §5.3 platform-binding
shape applied to analytics.

Declare a noun is backed by an analytics backend the same way:

```forml2
External System 'analytics' has URL 'https://clickhouse.internal'.
External System 'analytics' has Kind 'analytics'.
External System 'analytics' has Header 'Authorization'.
External System 'analytics' has Prefix 'Bearer'.

Noun 'Listing' is backed by External System 'analytics'.
Noun 'Listing' has URI '/v1/query'.
```

Two things change at compile time for an `analytics`-kind binding:

- The emitted `populate:Listing` def carries an "OLAP" shape —
  it consumes the same sort / order query params (#218) the
  REST list endpoint advertises, plus any declared fact-type
  filters, and returns a JSON array under OWA.
- Write-path routing is unchanged: create / update still land
  on per-cell DOs (Definition 2). The analytics backend is a
  **read replica**, not a write target. Consistency: the
  backend lags the DOs by its own ingestion window; clients
  that need last-write-read should bypass the analytics binding
  and read from the DOs directly.

Per-noun opt-in means you can back one hot noun with ClickHouse
and leave the rest on per-cell DOs. Typical pattern for a
marketplace: `Listing` and `Event` are analytics-backed (millions
of rows, read-heavy aggregations); `Account`, `Order`, `Payment`
stay on per-cell DOs (single-entity writes dominate, strong
consistency matters).

The SSRF denylist and credential-env-var rules from the fetch
path apply unchanged. A compile-time check rejects analytics URLs
that point into private address space.

## Built-in integrations

The bundled metamodel includes `organizations.md` which declares:

- `auth.vin` is an identity provider at `https://auth.vin` with the `users API-Key` prefix.
- `auto.dev` is an automotive data API.
- `stripe` is a billing system, exposed with all eight standard resources (customer, subscription, invoice, charge, payment_method, price, product, payment_intent).

You get these for free when you import the metamodel. Add your own External Systems in your domain readings.

## Writing a new integration

Four steps:

1. Declare the External System with URL, header, and optional prefix.
2. Declare each noun backed by it with a URI path.
3. Set `AREST_SECRET_{SYSTEM}` in your runtime environment.
4. Write constraints and derivations that use the federated noun alongside your local nouns.

No middleware, no schema translation, no separate query language. The fact type declared in your readings is the response schema; JSON keys map to role bindings through the compiled schema.

## Completeness patterns

Federated reads are OWA by default: a fact not fetched is *unknown*, not *false*. Constraints spanning a federated noun are therefore sound but not complete — a reported violation is genuine, but the absence of a violation is not a guarantee, since the violating fact may live in the remote system and simply not have been fetched.

When you need completeness anyway, reach for one of these three patterns.

### Snapshot into local CWA

Fetch the federated population once, assert each fact locally, then declare the noun CWA going forward. Appropriate when the remote changes slowly (regulatory lists, country codes, chart-of-accounts). After the snapshot, every constraint spanning the noun is complete against the authoritative local population.

```forml2
External System 'iso' has URL 'https://iso.example/countries'.

Noun 'Country' is backed by External System 'iso'.
Noun 'Country' has URI '/countries'.

## Instance Facts

Noun 'Country' has World Assumption 'closed'.
```

The last line flips the noun from OWA (the federation default) to CWA. Further fetches are still permitted, but the compiler treats the locally-asserted population as authoritative when evaluating constraints. Schedule a re-snapshot when the remote source changes.

### Read-only mandatory gate

Declare the federated noun with read-only permission so the compiler rejects alethic constraints that depend on completeness; express federated-spanning rules as deontic instead. This keeps the formalism honest — the system never *claims* a property it can't verify.

```forml2
Noun 'Stripe Customer' is backed by External System 'stripe'.
Noun 'Stripe Customer' has URI '/customers'.
Noun 'Stripe Customer' has Permission 'read'.

## Deontic Constraints

It is obligatory that each Subscription belongs to some Stripe Customer.
```

An alethic `Each Subscription belongs to exactly one Stripe Customer` would be rejected at compile time: the federation path cannot guarantee the Stripe Customer is present. Deontic lets the system record the expectation and report the violations it can observe, without over-claiming.

### Periodic reconciliation

Run a scheduled job that re-fetches the federated population, diffs against the local snapshot, and emits `Signal` entities for changes. The existing evolution workflow (`readings/evolution.md`) picks them up the same way it picks up Signals from any other source.

```forml2
## Entity Types

Reconciliation Run(.id) is an entity type.

## Fact Types

Reconciliation Run occurred at Timestamp.
Reconciliation Run observed Delta Count.
Reconciliation Run emits Signal.

## Instance Facts (emitted by each scheduled run)

Reconciliation Run 'recon-2026-04-20-01' occurred at Timestamp '2026-04-20T00:00:00Z'.
Reconciliation Run 'recon-2026-04-20-01' observed Delta Count '7'.
Signal 'recon-2026-04-20-01' has Signal Source 'Reconciliation'.
Reconciliation Run 'recon-2026-04-20-01' emits Signal 'recon-2026-04-20-01'.
Signal 'recon-2026-04-20-01' leads to Domain Change 'dc-2026-04-20-01'.
```

This is completeness-by-freshness rather than completeness-by-algebra: the incompleteness window is bounded by your reconciliation interval, and the Signal stream is the audit trail.

Note: `'Reconciliation'` is not in the default `Signal Source` enum in `readings/evolution.md` (`Constraint Violation`, `Human`, `Error Pattern`, `Feature Request`, `Support Request`). Extend the enum via a Domain Change before running a reconciliation job that uses it, or pick an existing value that fits your semantics.

### Picking a pattern

- **Snapshot** when the remote is slow-moving and fetch-once is viable.
- **Read-only gate** when the remote is authoritative but large or frequently changing — trade completeness for honest OWA semantics.
- **Reconciliation** when you need both: freshness plus constraint completeness, with Signals as the audit trail.

All three keep the federation path working; they differ only in how they reconcile OWA's sound-not-complete limitation with constraints that want CWA semantics.

## What's next

Your readings now reach everything: schema, constraints, workflows, derivations, and external systems. The next chapter, [The MCP verb set](09-mcp-verbs.md), shows how agents and tools talk to the whole surface.
