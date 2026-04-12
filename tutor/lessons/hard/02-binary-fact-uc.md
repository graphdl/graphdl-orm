# Lesson H2: A BINARY FACT WITH A UC

**Goal:** Declare a binary fact type and constrain its multiplicity with natural-language uniqueness.
**Prereqs:** Lesson H1

A binary fact type connects two nouns via a verb — "Order was placed by Customer". By itself it's many-to-many. You turn it into many-to-one / one-to-many / one-to-one by writing a uniqueness constraint **in the reading itself**, not as a separate config.

Three canonical patterns:

- **many-to-one:** `Each Order was placed by at most one Customer.` — every Order has at most one Customer; a Customer may have many Orders.
- **one-to-one:** add a second UC: `Each Customer placed at most one Order.`
- **many-to-many:** no UC on either role, OR the spanning form: `Each Order, Customer combination occurs at most once in the population of Order was placed by Customer.`

## Do it

~~~ compile
Order was placed by Customer.
  Each Order was placed by exactly one Customer.
~~~

## Check

~~~ expect
list Graph\ Schema contains {"id": "Order_was_placed_by_Customer"}
~~~

**NOTE:** "exactly one" = "at most one" + "at least one". The second clause is the mandatory constraint — every Order MUST have a Customer.

**Next:** [Lesson H3: A ternary with a spanning UC](./03-ternary-spanning-uc.md)
