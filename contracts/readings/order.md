# Order Domain

## Entity Types

Order(.Order Id) is an entity type.
Customer(.Name) is an entity type.
Amount is a value type.

## Fact Types

Order was placed by Customer.
  Each Order was placed by exactly one Customer.

Order has Amount.
  Each Order has at most one Amount.

## State Machines

State Machine Definition 'Order' is for Noun 'Order'.
Status 'In Cart' is initial in State Machine Definition 'Order'.

Transition 'place' is defined in State Machine Definition 'Order'.
Transition 'place' is from Status 'In Cart'.
Transition 'place' is to Status 'Placed'.

Transition 'ship' is defined in State Machine Definition 'Order'.
Transition 'ship' is from Status 'Placed'.
Transition 'ship' is to Status 'Shipped'.

Transition 'archive' is defined in State Machine Definition 'Order'.
Transition 'archive' is from Status 'Shipped'.
Transition 'archive' is to Status 'Archived'.
