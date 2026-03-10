# GraphDL Organizations — Access Control

## Entity Types

| Entity | Reference Scheme |
|--------|-----------------|
| Organization | OrgSlug |

## Value Types

| Value | Type | Constraints |
|-------|------|-------------|
| OrgSlug | string | unique |
| OrgRole | string | enum: owner, admin, member |
| Visibility | string | enum: private, public |

## Readings

Organization has Name.
  Each Organization has exactly one Name.

User has OrgRole in Organization.
  Each User has at most one OrgRole in each Organization.

Organization is owned by User.
  Each Organization is owned by exactly one User.

Domain belongs to Organization.
  Each Domain belongs to at most one Organization.

Domain has Visibility.
  Each Domain has exactly one Visibility.

## Constraints

Deleting the owner User of an Organization deletes the Organization.

## Derivation Rules

User can access Domain where User has OrgRole in Organization and Domain belongs to that Organization.
Any User can access Domain where Domain has Visibility 'public'.
