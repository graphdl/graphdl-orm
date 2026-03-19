# GraphDL Organizations — Access Control

## Entity Types

| Entity | Reference Scheme |
|--------|-----------------|
| Organization | Org Slug |
| App | App Slug |

## Value Types

| Value | Type | Constraints |
|-------|------|-------------|
| Org Slug | string | unique |
| App Slug | string | unique |
| Org Role | string | enum: owner, admin, member |
| Visibility | string | enum: private, public |
| Label | string | |
| Chat Endpoint | string | |
| App Type | string | enum: standard, chat |

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

Domain belongs to App.
  Each Domain belongs to at most one App.

Domain belongs to Organization.
  Each Domain belongs to at most one Organization.

Domain has Label.
  Each Domain has at most one Label.

Domain has Visibility.
  Each Domain has exactly one Visibility.

## Constraints

Deleting the owner User of an Organization deletes the Organization.

## Derivation Rules

User can access Domain where User has Org Role in Organization and Domain belongs to that Organization.
Any User can access Domain where Domain has Visibility 'public'.
