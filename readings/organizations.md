# GraphDL Organizations — Access Control

## Entity Types

Organization(.Org Slug) is an entity type.
App(.App Slug) is an entity type.
Domain(.Domain Slug) is an entity type.
User(.Email) is an entity type.

## Value Types

Org Slug is a value type.
App Slug is a value type.
Domain Slug is a value type.
Email is a value type.
Org Role is a value type.
  The possible values of Org Role are 'owner', 'admin', 'member'.
Visibility is a value type.
  The possible values of Visibility are 'private', 'public'.
Label is a value type.
Chat Endpoint is a value type.
App Type is a value type.
  The possible values of App Type are 'standard', 'chat'.

## Readings

### Organization

Organization has Name.
  Each Organization has exactly one Name.

User has Org Role in Organization.
  Each User has at most one Org Role in each Organization.

Organization is owned by User.
  Each Organization is owned by exactly one User.

### App

App has Name.
  Each App has at most one Name.

App has App Type.
  Each App has at most one App Type.

App has Chat Endpoint.
  Each App has at most one Chat Endpoint.

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

If some Organization is owned by some User and that User is deleted then that Organization is also deleted.

## Derivation Rules

If some User authenticates and no Organization is owned by that User then some Organization is owned by that User and that Organization has Name that is that User's Email and that User has Org Role 'owner' in that Organization.

User can access Domain iff User has Org Role in Organization and Domain belongs to that Organization.
User can access Domain if Domain has Visibility 'public'.

User can view Resource in App iff User has Org Role in Organization and App belongs to that Organization and App has navigable Domain and Resource belongs to that Domain.
User can view all Resources in App iff User has Org Role 'owner' in Organization and App belongs to that Organization.
User can view all Resources in App iff User has Org Role 'admin' in Organization and App belongs to that Organization.
User can view only own Resource in App iff User has Org Role 'member' in Organization and App belongs to that Organization and Resource is created by that User.

App shows Domain iff App has navigable Domain and that Domain is that Domain.
App shows Entity Type iff App has navigable Domain and some Noun is defined in that Domain and Entity Type is that Noun.
