# Lesson H3: A TERNARY WITH A SPANNING UC

**Goal:** Write a ternary fact type, with a uniqueness constraint that spans n−1 roles.
**Prereqs:** Lesson H2

A ternary binds three nouns. The arity decomposition rule: a ternary must have a UC spanning at least two of its three roles. If it doesn't, the fact is compound — split it into binaries.

The FORML2 form uses `For each ... and ...`: "For each Plan and Interval that Plan has that Interval at most one Price" means "(Plan, Interval) → Price is many-to-one." You cannot use the `Each X` shorthand for ternaries — that'd be a single-role UC on a ternary, which violates the rule.

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
list Fact\ Schema contains {"id": "Plan_has_Price_per_Interval"}
~~~

**NOTE:** If you objectify a ternary — promote it to an entity type — it must have a SPANNING UC (across all three roles) or an independent reference scheme. We'll see that in Lesson H6.

**Next:** [Lesson H4: A derivation rule](./04-derivation-rule.md)
