# Lesson H2: A BINARY FACT WITH A UC

**Goal:** Declare a binary fact type and constrain its multiplicity with natural-language uniqueness.
**Prereqs:** Lesson H1

A binary fact type connects two nouns via a verb, for example "Order was placed by Customer". By itself it is many-to-many. You turn it into many-to-one, one-to-many, or one-to-one by writing a uniqueness constraint **in the reading itself** rather than as a separate config.

Three canonical patterns exist:

- **many-to-one:** `Each Order was placed by at most one Customer.` In this form every Order has at most one Customer, while a Customer may have many Orders.
- **one-to-one:** add a second UC: `Each Customer placed at most one Order.`
- **many-to-many:** use no UC on either role, or the spanning form `Each Order, Customer combination occurs at most once in the population of Order was placed by Customer.`

## Do it

~~~ compile
Order was placed by Customer.
  Each Order was placed by exactly one Customer.
~~~

## Check

~~~ expect
list Fact\ Type contains {"id": "Order_was_placed_by_Customer"}
~~~

**NOTE:** "exactly one" equals "at most one" plus "at least one". The second clause is the mandatory constraint, which means every Order MUST have a Customer.

**Next:** [Lesson H3: A ternary with a spanning UC](./03-ternary-spanning-uc.md)
