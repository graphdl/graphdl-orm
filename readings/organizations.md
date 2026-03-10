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

## Readings

Organization has Name.
  Each Organization has at most one Name.
User has OrgRole in Organization — UC(User, Organization).
Each Organization has at most one owner User.
Domain belongs to Organization.
  Each Domain belongs to at most one Organization.
