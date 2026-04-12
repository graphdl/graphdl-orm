# Lesson H6: OBJECTIFICATION

**Goal:** Promote a fact type into an entity type — legally, with a spanning UC — and attach facts to the relationship itself.
**Prereqs:** Lesson H5

Objectification turns a fact into a thing. `CurrentMarriage` isn't just "Person married Person"; it's an entity with its own facts (date, location, officiant). You objectify a fact type when you want to say things ABOUT the relationship, not just that it holds.

The rule: you may only objectify a fact type if it has a UC spanning all its roles. Without a spanning UC, the objectified entity has no coherent identity — there's no way to know which instance you're talking about. If your fact type doesn't have a spanning UC, either add one or flatten into binaries.

## Do it

~~~ compile
Person is married to Person.
  Each Person, Person combination occurs at most once in the population of Person is married to Person.

Marriage is Person is married to Person.
Marriage has Date. Each Marriage has exactly one Date.
~~~

## Check

~~~ expect
list Noun contains {"id": "Marriage"}
~~~

**NOTE:** A better pattern is often an independent reference scheme: `Marriage(.CertificateNumber)`. That avoids the sub-conceptual choice of "which person identifies this marriage." Use the spanning-UC scheme when no natural external id exists.

**Next:** [Lesson H7: A declared state machine](./07-declared-sm.md)
