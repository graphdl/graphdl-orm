# Lesson H3: A TERNARY WITH A SPANNING UC

**Goal:** Write a ternary fact type, with a uniqueness constraint that spans n−1 roles.
**Prereqs:** Lesson H2

A ternary binds three nouns. The arity decomposition rule requires that a ternary have a UC spanning at least two of its three roles. If the ternary lacks such a UC, the fact is compound and should be split into binaries.

The FORML2 form uses `For each ... and ...`. For example, "For each Plan and Interval that Plan has that Interval at most one Price" means "(Plan, Interval) → Price is many-to-one." You cannot use the `Each X` shorthand for ternaries, because that would be a single-role UC on a ternary, which violates the rule.

## Do it

~~~ compile
Plan(.id) is an entity type.
Interval(.id) is an entity type.
Price is a value type.

Plan has Price per Interval.
  For each Plan and Interval that Plan has that Interval at most one Price.
~~~

## Check

~~~ expect
list Fact\ Type contains {"id": "Plan_has_Price_per_Interval"}
~~~

**NOTE:** If you objectify a ternary (promote it to an entity type), it must have a SPANNING UC (across all three roles) or an independent reference scheme. Lesson H6 shows how that works.

**Next:** [Lesson H4: A derivation rule](./04-derivation-rule.md)
