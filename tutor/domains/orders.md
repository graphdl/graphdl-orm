# Order Management

Exercises: state machines, transitions, guards, event types, deontic constraints,
derivation rules, subset constraints with autofill, ternary fact types,
fact-driven events, Mealy/Moore verb semantics.

## Entity Types

Order(.Order Number) is an entity type.
Customer(.Email) is an entity type.
LineItem(.id) is an entity type.
Shipment(.Tracking Number) is an entity type.
Payment(.Payment Id) is an entity type.
Coupon(.Coupon Code) is an entity type.

## Value Types

Order Number is a value type.
Email is a value type.
Tracking Number is a value type.
Payment Id is a value type.
Coupon Code is a value type.
Quantity is a value type.
Amount is a value type.
Discount Percent is a value type.
Shipping Address is a value type.
Billing Address is a value type.
Payment Method is a value type.
  The possible values of Payment Method are 'credit_card', 'debit_card', 'paypal', 'bank_transfer', 'crypto'.
Order Note is a value type.
Cancellation Reason is a value type.

## Readings

### Customer

Customer has Name.
  Each Customer has exactly one Name.

Customer has Shipping Address.
  Each Customer has at most one Shipping Address.

Customer has Billing Address.
  Each Customer has at most one Billing Address.

### Order

Order is placed by Customer.
  Each Order is placed by exactly one Customer.

Order has Amount.
  Each Order has exactly one Amount.

Order has Shipping Address.
  Each Order has exactly one Shipping Address.

Order has Order Note.
  Each Order has at most one Order Note.

Order has Cancellation Reason.
  Each Order has at most one Cancellation Reason.

### Line Item

LineItem belongs to Order.
  Each LineItem belongs to exactly one Order.

LineItem is for Product.
  Each LineItem is for exactly one Product.

LineItem has Quantity.
  Each LineItem has exactly one Quantity.

LineItem has Amount.
  Each LineItem has exactly one Amount.

### Coupon

Coupon has Discount Percent.
  Each Coupon has exactly one Discount Percent.

Coupon is applied to Order.
  Each Coupon is applied to at most one Order.
  Each Order has at most one Coupon.

### Payment

Payment is for Order.
  Each Payment is for exactly one Order.

Payment has Amount.
  Each Payment has exactly one Amount.

Payment has Payment Method.
  Each Payment has exactly one Payment Method.

### Shipment

Shipment is for Order.
  Each Shipment is for exactly one Order.

Shipment has Shipping Address.
  Each Shipment has exactly one Shipping Address.

## Constraints

It is obligatory that each Order has at least one LineItem.
It is obligatory that each LineItem has Quantity greater than 0.
It is forbidden that a Coupon has Discount Percent greater than 100.
It is forbidden that a Coupon has Discount Percent less than 0.

If some Customer places some Order then that Order has Shipping Address that is that Customer's Shipping Address.

## Derivation Rules

* Order has Amount iff Amount is the sum of LineItem Amount where some LineItem belongs to that Order.

If some Coupon is applied to some Order and that Coupon has Discount Percent then that Order has Amount that is reduced by that Discount Percent.

## Instance Facts

### Order State Machine

State Machine Definition 'Order' is for Noun 'Order'.
Status 'Draft' is defined in State Machine Definition 'Order'.
Status 'Placed' is defined in State Machine Definition 'Order'.
Status 'Paid' is defined in State Machine Definition 'Order'.
Status 'Shipped' is defined in State Machine Definition 'Order'.
Status 'Delivered' is defined in State Machine Definition 'Order'.
Status 'Cancelled' is defined in State Machine Definition 'Order'.
Status 'Refunded' is defined in State Machine Definition 'Order'.
Status 'Draft' is initial.

Transition 'place' is from Status 'Draft'.
Transition 'place' is to Status 'Placed'.
Transition 'place' is triggered by Event Type 'place'.

Transition 'pay' is from Status 'Placed'.
Transition 'pay' is to Status 'Paid'.
Transition 'pay' is triggered by Event Type 'pay'.

Transition 'ship' is from Status 'Paid'.
Transition 'ship' is to Status 'Shipped'.
Transition 'ship' is triggered by Event Type 'ship'.

Transition 'deliver' is from Status 'Shipped'.
Transition 'deliver' is to Status 'Delivered'.
Transition 'deliver' is triggered by Event Type 'deliver'.

Transition 'cancel-draft' is from Status 'Draft'.
Transition 'cancel-draft' is to Status 'Cancelled'.
Transition 'cancel-draft' is triggered by Event Type 'cancel'.

Transition 'cancel-placed' is from Status 'Placed'.
Transition 'cancel-placed' is to Status 'Cancelled'.
Transition 'cancel-placed' is triggered by Event Type 'cancel'.

Transition 'refund' is from Status 'Delivered'.
Transition 'refund' is to Status 'Refunded'.
Transition 'refund' is triggered by Event Type 'refund'.

### Payment State Machine

State Machine Definition 'Payment' is for Noun 'Payment'.
Status 'Pending' is defined in State Machine Definition 'Payment'.
Status 'Authorized' is defined in State Machine Definition 'Payment'.
Status 'Captured' is defined in State Machine Definition 'Payment'.
Status 'Failed' is defined in State Machine Definition 'Payment'.
Status 'Pending' is initial.

Transition 'authorize' is from Status 'Pending'.
Transition 'authorize' is to Status 'Authorized'.
Transition 'authorize' is triggered by Event Type 'authorize'.

Transition 'capture' is from Status 'Authorized'.
Transition 'capture' is to Status 'Captured'.
Transition 'capture' is triggered by Event Type 'capture'.

Transition 'fail' is from Status 'Pending'.
Transition 'fail' is to Status 'Failed'.
Transition 'fail' is triggered by Event Type 'fail'.

Domain 'orders' has Visibility 'public'.
