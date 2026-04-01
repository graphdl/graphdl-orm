# GraphDL Organizations — Access Control

## Entity Types

Organization(.Slug) is an entity type.
App(.Slug) is an entity type.
Domain(.Slug) is an entity type.
User(.Email) is an entity type.

## Value Types

Slug is a value type.
Email is a value type.
Org Role is a value type.
  The possible values of Org Role are 'owner', 'admin', 'member'.
Visibility is a value type.
  The possible values of Visibility are 'private', 'public'.
Label is a value type.
App Type is a value type.
  The possible values of App Type are 'standard', 'chat'.

## Readings

### Organization

Organization has Name.
  Each Organization has exactly one Name.

User has Org Role in Organization.
  Each User has at most one Org Role in each Organization.

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

## Constraints

If some User has Org Role 'owner' in some Organization and that User is deleted then that Organization is also deleted.

## Derivation Rules

If some User authenticates and that User has no Org Role in any Organization then that User has Org Role 'owner' in some Organization and that Organization has Name that is that User's Email.

User accesses Domain if User has Org Role in Organization and Domain belongs to that Organization.
User accesses Domain if Domain has Visibility 'public'.

User views Resource in App if User has Org Role in Organization and App belongs to that Organization and App has navigable Domain and Resource belongs to that Domain.
User views all Resources in App if User has Org Role 'owner' in Organization and App belongs to that Organization.
User views all Resources in App if User has Org Role 'admin' in Organization and App belongs to that Organization.
User views only own Resource in App if User has Org Role 'member' in Organization and App belongs to that Organization and Resource is created by that User.

App navigates Domain if App has navigable Domain.
App displays Noun if App has navigable Domain and Noun is defined in that Domain.

## Instance Facts

Domain 'organizations' has Visibility 'public'.
