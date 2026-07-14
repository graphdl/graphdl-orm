# The paper's own example (AREST.tex §1, the small declaration), verbatim
# modulo the two-column line wraps. G3 conformance app: the paper's example
# must compile on every host, same bytes.

Order(.OrderId) is an entity type.
Customer(.Name) is an entity type.
Order is placed by Customer.
Each Order is placed by exactly one Customer.
Customer ships Order.
State Machine Definition 'Order' is for Noun 'Order'.
Status 'In Cart' is initial in State Machine Definition 'Order'.
Transition 'place' is from Status 'In Cart'.
Transition 'place' is to Status 'Placed'.
Transition 'place' is triggered by Fact Type 'Customer places Order'.
Transition 'ship' is from Status 'Placed'.
Transition 'ship' is to Status 'Shipped'.
Transition 'ship' is triggered by Fact Type 'Customer ships Order'.
