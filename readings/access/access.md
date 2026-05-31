# AREST Access Control — authorization as substrate facts (server-enforced, no UI)

> Authorization is NOT a view concern. The server enforces permissions with no
> UI at all, so they live here as SUBSTRATE FACTS — a predicate the HATEOAS layer
> projects into action links and an alethic constraint the mutation path rejects
> against. The thin view never sees them.
>
> Two halves, both proven end-to-end (compile_explicit_derivation_tests.rs):
>   * READ  — `User is authorized for Operation on Noun` derives from the user's
>     access-level discriminator joined with that level's permissions. A subtype
>     (Admin ≤ User) is realised relationally as its joinable DISCRIMINATOR
>     (Halpin absorption/separation: an enum / unary flag / FK), here the
>     `Access Level` value type. The HATEOAS CRUDL menu (command::crudl_menu_operations)
>     projects this ∩ `Operation applies in View Context`.
>   * ENFORCE — `performs ⊆ authorized` is an ALETHIC role-SEQUENCE subset
>     constraint (ORM2 tuple subset, per NORMA ConstraintRoleSequenceWithJoinType):
>     a `User performs Operation on Noun` tuple outside `authorized` is a Subset
>     violation → the mutation is rejected (D' = D).
>
> `User` and `Noun` are metamodel entities (readings/core). `Operation` (the CRUDL
> verb) lives HERE — it is a server/REST concept; the iFactr ActionType *decoration*
> (Control Kind / Request Type / Action Type) stays in readings/ui/crudl.md, which
> references this Operation. `View Context` is the HATEOAS resource kind
> (collection / instance / edit), not a UI concept.

## Value Types

Access Level is a value type.

View Context is a value type.
  The possible values of View Context are 'collection', 'instance', 'edit'.

## Entity Types

Operation(.Name) is an entity type.

## Fact Types

User has Access Level.

Access Level permits Operation on Noun.

User is authorized for Operation on Noun. **

User performs Operation on Noun.

Operation applies in View Context.
  Each Operation applies in exactly one View Context.

## Instance Facts

The CRUDL verbs and the HATEOAS resource kind each shows up in. The iFactr
ActionType / Control Kind / Request Type decoration of these same Operations
lives in readings/ui/crudl.md (gated by `ui-readings`); the view-context
applicability is SUBSTRATE and stays here.

Operation 'create' applies in View Context 'collection'.
Operation 'edit' applies in View Context 'instance'.
Operation 'delete' applies in View Context 'instance'.
Operation 'multi-delete' applies in View Context 'collection'.
Operation 'save' applies in View Context 'edit'.
Operation 'cancel' applies in View Context 'edit'.

## Derivation Rules

The permission predicate: a User is authorized for an Operation on a Noun when the
User's access level permits that Operation on that Noun. A non-skolem multi-
antecedent equi-join on the `Access Level` discriminator (the bridge variable).

* User is authorized for Operation on Noun iff User has Access Level and Access Level permits Operation on Noun.

## Subset Constraints

Enforcement: every performed action must be authorized. The attempted-action tuple
`(User, Operation, Noun)` of `performs` must be a subset of `authorized`; a tuple
outside it is an alethic Subset violation (the mutation rejects).

If some User performs some Operation on some Noun then that User is authorized for that Operation on that Noun.
