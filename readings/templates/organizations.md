# AREST Organizations: Access Control

## Entity Types

Organization(.Slug) is an entity type.
App(.Slug) is an entity type.
Domain(.Slug) is an entity type.
External System(.Name) is an entity type.
Generator(.Name) is an entity type.

<!-- Connector-registry business nouns (API Product, Stripe *) and their
     External-System backings were moved OUT of the baked metamodel into the
     non-metamodel `connectors` library (apps/connectors) on 2026-06-24, so the
     base no longer bleeds business connectors into every app (#23). Apps that
     integrate those services depend on `file:../connectors`. -->

## Value Types

Slug is a value type.
Email is a value type.
Access is a value type.
  The possible values of Access are 'private', 'public'.
Label is a value type.
App Type is a value type.
  The possible values of App Type are 'standard', 'chat'.

## Fact Types

### Organization

Organization has Name.
  Each Organization has exactly one Name.

User owns Organization.
  Each Organization is owned by at most one User.

User administers Organization.

User belongs to Organization.

### User

User has Email.
  Each User has at most one Email.
  For each Email, exactly one User has that Email.

### App

App has Name.
  Each App has at most one Name.

App has App Type.
  Each App has at most one App Type.

App has URI.
  Each App has at most one URI.

App has navigable Domain.
  Each App has some navigable Domain.

App belongs to Organization.
  Each App belongs to at most one Organization.

App uses Generator.

### Domain

Domain has Name.
  Each Domain has at most one Name.

Domain belongs to App.
  Each Domain belongs to at most one App.

Domain belongs to Organization.
  Each Domain belongs to at most one Organization.

Domain has Label.
  Each Domain has at most one Label.

Domain has Access.
  Each Domain has exactly one Access.

### Derived Fact Types

User accesses Domain. +
App navigates Domain. +
App displays Noun. +

App extends App.

Domain depends on Domain.

## Constraints

If some User owns some Organization and that User is deleted then that Organization is also deleted.

Each App, App combination occurs at most once in the population of App extends App.
Each Domain, Domain combination occurs at most once in the population of Domain depends on Domain.

## Ring Constraints

No App extends itself.
No App may cycle back to itself via one or more traversals through extends.

No Domain depends on itself.
No Domain may cycle back to itself via one or more traversals through depends on.

## Derivation Rules

If some User authenticates and that User has some Email and that User does not own any Organization then that User owns some Organization and that Organization has Name that is that Email.

+ User accesses Domain if User owns Organization and App belongs to that Organization and Domain belongs to that App.
+ User accesses Domain if User administers Organization and App belongs to that Organization and Domain belongs to that App.
+ User accesses Domain if User belongs to Organization and App belongs to that Organization and Domain belongs to that App.
+ User accesses Domain if Domain has Access 'public'.

+ App navigates Domain if App has navigable Domain.
<!-- ilayer-join-order (2026-06-23): clauses ordered "App contains Domain" ->
     "Noun is defined in Domain" -> "Noun is displayed by Element" so each join
     shares a key (Domain, then Noun) with the prior clause. The original
     Noun-displayed-first order joined C1(Noun,Element) x C2(App,Domain) -- which
     share NO variable -- as a full cartesian before the C3 filter, materializing
     a multi-GB witness set that OOM-crashed every app compile. -->
App uses Generator 'ilayer' if that App contains some Domain and some Noun is defined in that Domain and that Noun is displayed by some Element.

## Instance Facts

Domain 'organizations' has Access 'public'.
