# GraphDL Organizations — Access Control

## Entity Types

| Entity | Reference Scheme |
|--------|-----------------|
| Organization | OrgSlug |

## Value Types

| Value | Type | Constraints |
|-------|------|-------------|
| OrgSlug | string | unique |
| OrgRole | string | enum: owner, member |

## Readings

Organization has Name.
  Each Organization has at most one Name.
User has OrgRole in Organization — UC(User, Organization).
Domain belongs to Organization.
  Each Domain belongs to at most one Organization.
