# AREST Organizations: Access Control

## Entity Types

Organization(.Slug) is an entity type.
App(.Slug) is an entity type.
Domain(.Slug) is an entity type.
User(.id) is an entity type.
External System(.Name) is an entity type.
Generator(.Name) is an entity type.

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
App uses Generator 'ilayer' if some Noun is displayed by some Element and that App contains some Domain and that Noun is defined in that Domain.

## Instance Facts

Domain 'organizations' has Access 'public'.

Noun 'User' is backed by External System 'auth.vin'.
Noun 'User' has URI '/users'.

Noun 'API Product' is backed by External System 'auto.dev'.

Noun 'Stripe Customer' is backed by External System 'stripe'.
Noun 'Stripe Customer' has URI '/customers'.
Noun 'Stripe Subscription' is backed by External System 'stripe'.
Noun 'Stripe Subscription' has URI '/subscriptions'.
Noun 'Stripe Invoice' is backed by External System 'stripe'.
Noun 'Stripe Invoice' has URI '/invoices'.
Noun 'Stripe Charge' is backed by External System 'stripe'.
Noun 'Stripe Charge' has URI '/charges'.
Noun 'Stripe Payment Method' is backed by External System 'stripe'.
Noun 'Stripe Payment Method' has URI '/payment_methods'.
Noun 'Stripe Price' is backed by External System 'stripe'.
Noun 'Stripe Price' has URI '/prices'.
Noun 'Stripe Product' is backed by External System 'stripe'.
Noun 'Stripe Product' has URI '/products'.
Noun 'Stripe Payment Intent' is backed by External System 'stripe'.
Noun 'Stripe Payment Intent' has URI '/payment_intents'.
