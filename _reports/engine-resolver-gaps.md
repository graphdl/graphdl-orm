# Engine Resolver Gaps — auto.dev as Diagnostic Ground Truth

**Corpus under study**: `apps/auto.dev` — 71 check-cli warnings, 0 errors, 0 hints.
**All 71 warnings are antecedent-resolution failures** in derivation rule bodies and deontic constraints where the antecedent clause does not resolve to a declared fact type.

**The readings are correct.** Every referenced predicate is either:
- Declared elsewhere as a fact type or derived fact type (`*` / `+` marker)
- A standard FORML 2 pattern the resolver should recognize

This report categorizes every gap with concrete examples so the engine agent can prioritize fixes.

## Summary of gap categories

| # | Category | Warning count | Blast radius (other corpora) |
|---|----------|--------------:|------------------------------|
| A | Derived fact type as antecedent | 18 | us-law, eu-law, support.auto.dev |
| B | Subtype predicate inheritance | 4 | support.auto.dev, us-law |
| C | Parameter-atom-in-rule-body | 13 | us-law (`Source is 'oem'`), eu-law |
| D | Negation (`is not`, `has no`, `no X is`, `does not`) | 7 | us-law, eu-law |
| E | Aggregate operators beyond count-of / sum-of | 8 | support.auto.dev, us-law |
| F | Range / containment / comparison predicates | 5 | eu-law, us-law |
| G | Relative-clause / subordinate `that`-chains | 12 | eu-law, us-law |
| H | `or`-in-rule-body (mid-clause disjunction) | 4 | eu-law, us-law |
| I | Hyphen-binding role inconsistency in relative clauses | 6 | (mostly auto.dev) |
| J | Cross-file / cross-corpus fact type references | 5 | support.auto.dev |
| K | Symmetric / same-X co-reference | 2 | us-law |

---

## Category A — Derived fact type used as antecedent

**Pattern**: fact type declared with `*` / `+` / `iff` / `if` derivation. Another rule references the same fact type in its antecedent. Checker says unresolved.

**Example 1** — `Customer is in EEA` declared in `eu-compliance.md:41-43`:
```
Customer is in EEA. *
* Customer is in EEA iff Customer has Country Code that is an EEA Country Code.
```
Used as antecedent in 7 rules (all warnings):
```
+ Customer is subject to Regulation 'GDPR (EU 2016/679)' if Customer is in EEA.
* Customer has Data Subject Right Type 'access' iff Customer is in EEA.
```
**All 7 fire unresolved.**

**Example 2** — `Fetcher is proxy-based` declared in `source-routing.md:93`:
```
Fetcher is proxy-based. +
```
Used as antecedent:
```
+ Fetcher is isolated if Fetcher is proxy-based.
```
**Fires unresolved.**

**Example 3** — `Customer is eligible for trial` declared in `website.md:47`:
```
Customer is eligible for trial if Customer has Plan 'Free' or Customer has Plan 'Starter'.
```
Used as antecedent:
```
Checkout Session has Trial Days '14' if Customer is eligible for trial and Checkout Session is for that Customer.
```
**Fires unresolved.**

**Fix**: when resolving an antecedent, include fact types declared elsewhere with `*` / `+` markers (not just elementary ones).

---

## Category B — Subtype predicate inheritance

**Pattern**: entity `X` subtypes entity `Y`. `Y has Z` is declared. A rule body says `X has Z` expecting subtype inheritance.

**Example** — `customer-auth.md:17`:
```
Customer is a subtype of User.
```
`User(.Email)` is declared in `arest/readings/organizations.md:8`, which implicitly means `User has Email`.

Rules in `payload.md:120`, `posthog.md:81`, `stripe.md:151`:
```
Customer has Payload Document if Customer has Email and ...
+ Customer has PostHog Person if ... Customer has Email ...
Customer has Stripe Customer if ... Customer has Email ...
```
**All 3 fire `Customer has Email` unresolved.**

**Fix**: when resolving `Subtype has X`, if `Supertype has X` is declared (either explicitly or via ref scheme), match it.

---

## Category C — Parameter-atom-in-rule-body

**Pattern**: an entity has a ref scheme value type. A rule body references the entity by its literal atom ID: `Entity is 'atom'`. The checker doesn't match.

**Example 1** — `Source(.Source Name) is an entity type.` declared in `data-pipeline.md`. Rule in same file:
```
Source has priority over Source if Source is 'oem' and other Source is not 'oem'.
Source has priority over Source if Source is 'edmunds' and other Source is 'kbb'.
```
**All 4 clauses (`Source is 'oem'`, `other Source is not 'oem'`, `Source is 'edmunds'`, `other Source is 'kbb'`) fire unresolved.**

**Example 2** — `Email Template` entity with Name ref scheme. Rules in `website.md:51-55`:
```
Notification is triggered if Customer has usage at 50 percent and Email Template is 'limit-50'.
Notification is triggered if ... and Email Template is 'limit-75'.
Notification is triggered if ... and Email Template is 'limit-90'.
Notification is triggered if ... and Email Template is 'limit-100'.
Notification is triggered if ... and Email Template is 'cancellation-feedback'.
```
**5 `Email Template is '...'` clauses fire unresolved.**

**Fix**: recognize `Entity is 'atom'` as a ref-scheme-value binding (like `Entity has Ref Scheme Value 'atom'`).

---

## Category D — Negation

**Pattern**: `is not X`, `has no Y`, `no X is for Y`, `does not X`.

**Examples**:
- `source-routing.md:150`: `Source Request is for Resource Declaration that has no override- Fetcher` (2 occurrences)
- `data-pipeline.md:105`: `Source Resource is stale iff Source Resource has cached- Timestamp and Source Resource is not fresh.`
- `data-pipeline.md:106`: `Source Resource is missing iff no Source Resource is from that Source for that VIN.`
- `data-pipeline.md:110`: `Source has priority over Source if Source is 'oem' and other Source is not 'oem'.`
- `eu-compliance.md:54`: `Billable Request is GDPR-compliant iff Billable Request does not involve Personal Data.`
- `source-routing.md:152`: `Source Request is for Source Declaration that has no default- Fetcher`

**Fix**: strip negation, match against positive fact type, apply logical negation.

---

## Category E — Aggregate operators beyond count-of / sum-of

**Pattern**: aggregate-style expressions that aren't `count of X where Y` or `sum of X where Y`.

**Examples**:
- `data-pipeline.md:108`: `Vindex Entry is ambiguous iff more than one Vindex Entry has that Squish VIN.` — `more than one X` quantifier
- `service-health.md:105`: `average Response Time is the mean of Log Entry Response Time where ...` — `mean of` operator
- `caching.md`: `expires- Timestamp is cached- Timestamp plus TTL.` — temporal arithmetic `X plus Y`
- `eu-compliance.md`: `response Deadline is submission Date plus 30 days.` — date arithmetic `X plus N days`
- `cost-attribution.md`: `cost per call is Service monthly cost for that Billing Period divided by Meter Event Usage Count ...` — arithmetic `X divided by Y`
- `auth.md`: `Idempotency Key is composed of that Customer, that Meter Endpoint, that billing period start Timestamp, and that Date.` — `composed of` concatenation
- `source-routing.md`: `target- URL is the concatenation of that Base Path and that Resource Path.` — `concatenation of`
- `vehicle-data.md`: `VIN decodes to Year, Make, Model, and Trim via Data Provider 'VPIC'.` — multi-role extraction / `decodes to`

**Fix**: extensible computed-binding grammar for arithmetic (`plus`, `minus`, `divided by`, `concatenation of`), aggregate selectors (`mean of`, `earliest of`, `latest of`, `more than N`), and transforms (`decodes to`).

---

## Category F — Range / containment / comparison predicates

**Pattern**: value comparisons like `within`, `before`, `greater than`, `less than`, `X of N or more`.

**Examples**:
- `service-health.md:103,105,107`: `Log Entry has Timestamp within that Interval` (3 occurrences) — range containment
- `data-pipeline.md:104`: `Source Resource is fresh iff Source Resource has Fresh Until and now is before that Fresh Until.` — `now is before X`
- `service-health.md:124`: `HTTP Status of 500 or more` — bare comparison without explicit subject
- `database-routing.md:49`: `Query should retry if Query Route has Max Retry Count greater than 0 and attempt count is less than Max Retry Count.` — `greater than N` + `less than X`

**Fix**: recognize `within`, `before`, `after`, `greater than`, `less than`, `at least`, `at most`, `or more`, `or less` as built-in comparison predicates.

---

## Category G — Relative-clause / subordinate `that`-chains

**Pattern**: `X has Y that <predicate>` — a relative clause that constrains `Y`. Layer-2 doesn't descend into the `that`-subclause.

**Examples**:
- `api-products.md`: `some Data Provider that sources data for that API Product serves that Vehicle Make` — double-nested relative: `Data Provider that sources data for API Product` + `Data Provider serves Vehicle Make`
- `source-routing.md`: `Source Request is for Source Declaration that has Base Path` — relative `Source Declaration that has Base Path`
- `eu-compliance.md`: `Billable Request is for Customer that is in EEA` — relative with derived predicate
- `eu-compliance.md`: `Billable Request is for Meter Endpoint that has Identifier Sensitivity that is 'vehicle-identifier' or 'plate-identifier'` — nested relative + disjunctive value
- `eu-compliance.md`: `Billable Request is for Meter Endpoint that is provided by External System that is established in Country Code that is not an EEA Country Code` — triple-nested relative + negation
- `eu-compliance.md`: `Log Entry has Endpoint that is Meter Endpoint that has Identifier Sensitivity 'vehicle-identifier' or 'plate-identifier'` — similar
- `eu-compliance.md`: `Customer has Country Code that is an EEA Country Code` — relative with value-type co-reference
- `payload.md`: `Customer has Email Address that is that Email` — relative tying two value types together

**Fix**: parse `that <subject> <predicate>` as a sub-clause; resolve against declared fact types; bind the role variable through.

---

## Category H — `or`-in-rule-body (mid-clause disjunction)

**Pattern**: `X or Y` appears mid-clause (not as top-level rule disjunction).

**Examples**:
- `website.md:47`: `Customer is eligible for trial if Customer has Plan 'Free' or Customer has Plan 'Starter'.` — top-level `or` in antecedent
- `api-request-context.md`: `Request Context has Trust Score iff Request Context belongs to Customer and that Customer is authenticated and Trust Score is Authenticated Trust Score, or Request Context belongs to Customer and not Customer is authenticated and Trust Score is the Cloudflare Bot Management Score for that Request Context.` — comma-or mid-iff
- Inside relatives: `'vehicle-identifier' or 'plate-identifier'` — value disjunction in a relative clause

**Fix**: split `A or B` at the top level of an antecedent (or within a single relative clause) into separate rule bodies or treat as logical OR.

---

## Category I — Hyphen-binding role inconsistency in relative clauses

**Pattern**: fact type declared with hyphen-bound role (`X has override- Fetcher`). Rule body references it in a relative clause (`Resource Declaration that has override- Fetcher`). Layer-2 handles hyphen-binding in flat references but not in relative-clause form.

**Examples** (all `source-routing.md`):
- `Source Request is for Resource Declaration that has override- Fetcher` (declared: `Resource Declaration has override- Fetcher`)
- `Source Request is routed via that override- Fetcher`
- `Source Request is for Resource Declaration that has no override- Fetcher` (also adds negation — Category D)
- `Source Request is for Source Declaration that has default- Fetcher`
- `Source Request is routed via that default- Fetcher`
- `Source Request is for Source Declaration that has no default- Fetcher`

**Fix**: in Category G's relative-clause parsing, preserve hyphen-bound role names when matching against declared fact types.

---

## Category J — Cross-file / cross-corpus fact type references

**Pattern**: fact type declared in one file; rule in another file references it. `check-cli --include-metamodel` already handles metamodel references, but intra-app cross-file and cross-corpus references still fail sometimes.

**Examples**:
- `plans-subscriptions.md`: `some Plan Change grants Trial Days to that Customer` — `Plan Change grants Trial Days to Customer` may be declared elsewhere
- `listings.md`: `VDP is sourced from Listing Source iff some Listing has that VDP and that Listing is sourced from that Listing Source.` — `Listing has VDP` declared elsewhere
- `ingest-services.md`: `Scrape Job requires Captcha Task if Scrape Job is for Scrape Target and Proxy Request is for that Scrape Target` — `Proxy Request is for Scrape Target` declared elsewhere
- `vehicle-data.md`: `VIN decodes to Option via OEM Data Provider for that Vehicle Make` — complex cross-file reference
- `ingest-services.md`: `extraction produces normalized data for that Scrape Job` — likely cross-file

**Fix**: check-cli already auto-includes metamodel. Also follow App-extends-App chain or scan-all-dirs-in-app to collect all declarations before resolving antecedents.

---

## Category K — Symmetric / same-X co-reference

**Pattern**: `same X`, `the same Y`, `X of that Z` where X is an implicit co-reference between two role fillers.

**Examples**:
- `service-health.md:126`: `Noun population is resolved from alternate External System if primary External System has Service Health Status 'degraded' and alternate External System serves same Noun.` — `same Noun` means the Noun bound earlier in the rule
- `api-request-context.md`: `Customer has Country Code that is an EEA Country Code` — the value-type `Country Code` is implicitly constrained against the `EEA Country Code` population

**Fix**: `same X` should bind to the earlier X variable in the rule scope.

---

## Priority ordering (by total blast radius across corpora)

1. **Category A — Derived fact type as antecedent** (~18 in auto.dev; also dominant in eu-law, us-law). Largest single category. Fix would let us stop inlining derivation bodies.
2. **Category G — Relative-clause / `that`-chains** (~12 in auto.dev; ~40+ in eu-law, us-law). Second largest. Also unlocks natural legal-style verbalization.
3. **Category C — Parameter-atom-in-rule-body** (~13 in auto.dev; widespread elsewhere). Simple pattern, mechanical fix, high yield.
4. **Category E — Aggregate operators beyond count-of / sum-of** (~8 in auto.dev; many in us-law for tax rules). Needs a computed-binding grammar.
5. **Category D — Negation** (~7 in auto.dev; many in eu-law deontic constraints).
6. **Category I — Hyphen-binding in relatives** (~6 in auto.dev; rare elsewhere).
7. **Category F — Range / comparison** (~5 in auto.dev; common in eu-law time-bound rules).
8. **Category J — Cross-file / cross-corpus** (~5 in auto.dev; common in extension chains).
9. **Category H — `or`-in-rule-body** (~4 in auto.dev; scattered).
10. **Category B — Subtype inheritance** (~4 in auto.dev; scattered).
11. **Category K — same-X co-reference** (~2 in auto.dev; rare elsewhere).

## Cross-cutting notes

- **Rules often hit multiple categories at once.** `Source Request is for Resource Declaration that has no override- Fetcher` is Category D (negation) + G (relative) + I (hyphen-binding). Fixing one category closes some warnings fully but leaves compound-gap rules half-resolved.
- **Deontic constraints suffer the same gaps as derivations.** `It is obligatory that API Product 'build' returns a response only if some Data Provider that sources data for that API Product serves that Vehicle Make.` — the antecedent clause is Category G.
- **Reference corpora already 0**: arest-metamodel, law-core, spd-1, sherlock, robocall-service, tutor-domains, support.auto.dev. auto.dev is the richest remaining gap set.
- **No readings-side workaround is acceptable here.** The user rejected converting derivation bodies to `###` commentary. The 71 warnings are signals the engine must eventually handle.

## File anchors for reproduction

All examples reference files under `C:\Users\lippe\Repos\apps\auto.dev\`. To reproduce the full list:
```
C:\Users\lippe\Repos\arest\crates\arest\target\release\check-cli.exe C:\Users\lippe\Repos\apps\auto.dev
```
