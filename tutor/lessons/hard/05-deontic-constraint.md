# Lesson H5: A DEONTIC CONSTRAINT

**Goal:** Declare a rule that is morally required but not alethically necessary, then check text against the rule.
**Prereqs:** Lesson H4

Alethic constraints say "this CANNOT be otherwise"; violating them makes the state impossible, so the engine rejects the apply. Deontic constraints say "this SHOULD not be otherwise"; violations are reported but the apply still succeeds. Deontic constraints are the natural home for policy. The reading "Each Order should be placed within 30 days of the quote" can be true today and false tomorrow without the database being broken.

`validate` takes raw text plus a constraint, extracts matching facts via the client LLM (or via a pre-supplied `llm_response`), and reports whether the facts satisfy the constraint. Content moderation, contract review, and policy compliance all run through the same mechanism.

## Do it

~~~ compile
It is obligatory that each Order is placed by a Customer in good standing.
~~~

~~~ validate
{
  "text": "Order #42 was placed by delinquent-account on 2026-04-12.",
  "constraint": "Each Order is placed by a Customer in good standing"
}
~~~

## Check

~~~ expect
violations for apply create Order {"Customer":"delinquent-account"} include standing
~~~

**NOTE:** Under OWA (open-world), the absence of a "good standing" fact is unknown rather than false. Deontic constraints under OWA are sound but not complete: a reported violation is real, but the absence of a violation does not prove compliance.

**Next:** [Lesson H6: Objectification](./06-objectification.md)
