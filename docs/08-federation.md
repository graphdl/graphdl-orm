# 08 · Federation

Some facts are not yours. Customer records live in your CRM, payments live in Stripe, auth tokens live in an identity provider. Federation is how graphdl-orm treats those external systems as first-class fact sources without copying their data.

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

The fetched facts enter `P` under OWA (Open World Assumption). Their absence is not a violation — only their presence against a deontic constraint is. Alethic constraints that span an OWA noun are treated as sound-but-not-complete.

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
User has payment method := User owns Stripe Customer and that Stripe Customer has some Payment Method.
```

The engine does not distinguish local from federated at query time. It fetches the federated data when needed, joins it with the local population, and evaluates the rule as usual.

## Built-in integrations

The bundled metamodel includes `organizations.md` which declares:

- `auth.vin` — identity provider at `https://auth.vin` with `users API-Key` prefix
- `auto.dev` — automotive data API
- `stripe` — billing, with all eight standard resources (customer, subscription, invoice, charge, payment_method, price, product, payment_intent)

You get these for free when you import the metamodel. Add your own External Systems in your domain readings.

## Writing a new integration

Four steps:

1. Declare the External System with URL, header, and optional prefix.
2. Declare each noun backed by it with a URI path.
3. Set `AREST_SECRET_{SYSTEM}` in your runtime environment.
4. Write constraints and derivations that use the federated noun alongside your local nouns.

No middleware, no schema translation, no separate query language. The fact type declared in your readings is the response schema; JSON keys map to role bindings through the compiled schema.

## What's next

Your readings now reach everything — schema, constraints, workflows, derivations, and external systems. [The MCP verb set](09-mcp-verbs.md) is how agents and tools talk to all of it.
