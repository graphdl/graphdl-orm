# Connectors

External System auth shapes for the third-party services arest-based apps integrate
with. URL, Header, Prefix, Kind, and Country Code (for cross-border data-transfer
derivations) live here as the canonical source of truth. Per-app Domain Connection
facts carrying the Secret References (actual API keys) live in each consuming app's
gitignored `.env` file.

The lowercase identifier convention (`'stripe'`, `'auth.vin'`, `'github'`,
`'resend'`, `'auto.dev'`) is canonical. Earlier capital-S `'Stripe'` and capital-R
`'Resend'` references in `apps/auto.dev/website.md` were case-inconsistent and have
been removed in favour of this file.

## Instance Facts

### auth.vin — customer authentication, account management, API key issuance

External System 'auth.vin' has URL 'https://auth.vin'.
External System 'auth.vin' has Header 'Authorization'.
External System 'auth.vin' has Prefix 'users API-Key'.
External System 'auth.vin' has Kind 'rest'.
External System 'auth.vin' is established in Country Code 'US'.

### auto.dev — auto.dev API gateway

External System 'auto.dev' has URL 'https://api.auto.dev'.
External System 'auto.dev' has Header 'X-API-Key'.
External System 'auto.dev' has Kind 'rest'.
External System 'auto.dev' is established in Country Code 'US'.

### stripe — payments and subscription billing

External System 'stripe' has URL 'https://api.stripe.com/v1'.
External System 'stripe' has Header 'Authorization'.
External System 'stripe' has Prefix 'Bearer'.
External System 'stripe' has Kind 'rest'.
External System 'stripe' is established in Country Code 'US'.

### github — feature request federation, code-of-record federation

External System 'github' has URL 'https://api.github.com'.
External System 'github' has Header 'Authorization'.
External System 'github' has Prefix 'Bearer'.
External System 'github' has Kind 'rest'.
External System 'github' is established in Country Code 'US'.

### resend — email delivery

External System 'resend' has URL 'https://api.resend.com'.
External System 'resend' has Header 'Authorization'.
External System 'resend' has Prefix 'Bearer'.
External System 'resend' has Kind 'rest'.
External System 'resend' is established in Country Code 'US'.

### clickhouse — analytics and log-entry datastore

External System 'clickhouse' has URL 'https://t7lrlishe6.us-east-1.aws.clickhouse.cloud:8443'.
External System 'clickhouse' has Header 'Authorization'.
External System 'clickhouse' has Prefix 'Basic'.
External System 'clickhouse' has Kind 'clickhouse'.
External System 'clickhouse' is established in Country Code 'US'.

### posthog — product analytics, feature flags, event capture

External System 'posthog' has URL 'https://us.posthog.com'.
External System 'posthog' has Header 'Authorization'.
External System 'posthog' has Prefix 'Bearer'.
External System 'posthog' has Kind 'rest'.
External System 'posthog' is established in Country Code 'US'.

### Domain Metadata

Domain 'connectors' has Access 'public'.
Domain 'connectors' has Description 'External System auth shapes for the third-party services arest-based apps integrate with. Per-app Domain Connection facts carrying Secret References live in gitignored .env files; this file is the auth-shape source of truth.'.
