# GraphDL Organizations — Access Control

## Entity Types

Organization(.Slug) is an entity type.
App(.Slug) is an entity type.
Domain(.Slug) is an entity type.
User(.Email) is an entity type.
External System(.Name) is an entity type.

## Value Types

Slug is a value type.
Email is a value type.
Visibility is a value type.
  The possible values of Visibility are 'private', 'public'.
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

### Domain

Domain has Name.
  Each Domain has at most one Name.

Domain belongs to App.
  Each Domain belongs to at most one App.

Domain belongs to Organization.
  Each Domain belongs to at most one Organization.

Domain has Label.
  Each Domain has at most one Label.

Domain has Visibility.
  Each Domain has exactly one Visibility.

### Derived Fact Types

User accesses Domain.
App navigates Domain.
App displays Noun.

## Constraints

If some User owns some Organization and that User is deleted then that Organization is also deleted.

## Derivation Rules

If some User authenticates and that User does not own any Organization then that User owns some Organization and that Organization has Name that is that User's Email.

User accesses Domain if User owns Organization and App belongs to that Organization and Domain belongs to that App.
User accesses Domain if User administers Organization and App belongs to that Organization and Domain belongs to that App.
User accesses Domain if User belongs to Organization and App belongs to that Organization and Domain belongs to that App.
User accesses Domain if Domain has Visibility 'public'.

App navigates Domain if App has navigable Domain.
App displays Noun if App has navigable Domain and Noun is defined in that Domain.

## Instance Facts

Domain 'organizations' has Visibility 'public'.

Noun 'User' is backed by External System 'auth.vin'.
Noun 'User' has URI '/users'.

Noun 'API Product' is backed by External System 'auto.dev'.
