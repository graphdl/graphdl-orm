# AREST UI: CRUDL Operations — the iFactr action catalog as facts (crudl-menu-projection)

> The CRUDL operations a user may perform on an entity or collection, modeled
> as an iFactr action menu. Grounded DIRECTLY in iFactr's action vocabulary —
> `iFactr-Android/iFactr.UI/iFactr.UI/Controls/ActionType.cs`
> (`Undefined, Add, Cancel, Edit, Delete, More, Submit, None`) — and its
> serialized controls (`Button`, `SubmitButton`, `CancelButton`; see
> `apps/ui.do/src/ifactr/contract.md`). The menu itself is
> `iLayer.ActionButtons` / `IMenu.Buttons`; each item is a `Button`/`Link`
> carrying an `ActionType` + `RequestType` (+ optional confirmation).
>
> Operation → iFactr ActionType: create=`Add`, edit=`Edit`, delete=`Delete`,
> save=`Submit`, cancel=`Cancel`. "Search" is the `IListView` SearchBox (a
> list affordance, not an action button), so it is NOT an Operation here.
> "Multi-Delete" is `Delete` in a collection/multi-select context.
>
> This reading is the OPERATION CATALOG (the data). The permission gate
> (`User is permitted Operation on Noun`) and the gated menu derivation
> (`ViewElement renders Operation iff Operation applies in View Context and
> User is permitted Operation on Noun`) are follow-on slices — see the
> `crudl-menu-projection` task. Not yet registered in `lib.rs` UI_READINGS:
> register + a full-metamodel compile is the deploy step (check `Operation`
> does not collide with an existing metamodel noun first).

## Value Types

iFactr Action Type is a value type.
  The possible values of iFactr Action Type are
    'Undefined', 'Add', 'Cancel', 'Edit', 'Delete', 'More', 'Submit', 'None'.

View Context is a value type.
  The possible values of View Context are 'collection', 'instance', 'edit'.

Control Kind is a value type.
  The possible values of Control Kind are
    'Button', 'SubmitButton', 'CancelButton', 'Link', 'Icon', 'Label'.

CRUDL Request Type is a value type.
  The possible values of CRUDL Request Type are 'GET', 'POST', 'PUT', 'DELETE'.

## Entity Types

Operation(.Name) is an entity type.

## Fact Types

Operation has iFactr Action Type.
  Each Operation has exactly one iFactr Action Type.

Operation applies in View Context.
  Each Operation applies in exactly one View Context.

Operation has CRUDL Request Type.
  Each Operation has exactly one CRUDL Request Type.

Operation has Control Kind.
  Each Operation has exactly one Control Kind.

Operation requires Confirmation.

## Instance Facts

Operation 'create' has iFactr Action Type 'Add'.
Operation 'create' applies in View Context 'collection'.
Operation 'create' has CRUDL Request Type 'POST'.
Operation 'create' has Control Kind 'Button'.

Operation 'edit' has iFactr Action Type 'Edit'.
Operation 'edit' applies in View Context 'instance'.
Operation 'edit' has CRUDL Request Type 'GET'.
Operation 'edit' has Control Kind 'Button'.

Operation 'delete' has iFactr Action Type 'Delete'.
Operation 'delete' applies in View Context 'instance'.
Operation 'delete' has CRUDL Request Type 'DELETE'.
Operation 'delete' has Control Kind 'Button'.
Operation 'delete' requires Confirmation.

Operation 'multi-delete' has iFactr Action Type 'Delete'.
Operation 'multi-delete' applies in View Context 'collection'.
Operation 'multi-delete' has CRUDL Request Type 'DELETE'.
Operation 'multi-delete' has Control Kind 'Button'.
Operation 'multi-delete' requires Confirmation.

Operation 'save' has iFactr Action Type 'Submit'.
Operation 'save' applies in View Context 'edit'.
Operation 'save' has CRUDL Request Type 'PUT'.
Operation 'save' has Control Kind 'SubmitButton'.

Operation 'cancel' has iFactr Action Type 'Cancel'.
Operation 'cancel' applies in View Context 'edit'.
Operation 'cancel' has CRUDL Request Type 'GET'.
Operation 'cancel' has Control Kind 'CancelButton'.
