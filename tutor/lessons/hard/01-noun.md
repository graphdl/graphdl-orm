# Lesson H1: DECLARE A NOUN

**Goal:** Add a new entity type to the running engine with one FORML2 reading.
**Prereqs:** Medium track (tool-call literacy)

A noun is an entity type. Every entity in your system instantiates one. The declaration uses a reference scheme, which is the value or values that uniquely identify an entity of this type. `Customer(.Name)` means "a Customer is identified by its Name."

Three things happen when this reading compiles:
1. The engine emits a fact-type constructor for the reference scheme.
2. `Customer` becomes legal as a role target in every other fact type you declare.
3. The `list:Customer`, `get:Customer`, and `schema:Customer` defs are wired, giving you a REST surface without any additional config.

## Do it

~~~ compile
Customer(.Name) is an entity type.
Customer has Email.
  Each Customer has at most one Email.
~~~

## Check

~~~ expect
list Noun contains {"id": "Customer"}
~~~

**NOTE:** The reference scheme is the PREFERRED identifier. Other uniqueness constraints may exist on the same entity, acting as alternate keys rather than replacements for the preferred scheme.

**Next:** [Lesson H2: A binary fact with a UC](./02-binary-fact-uc.md)
