# Lesson H5: A DEONTIC CONSTRAINT

**Goal:** Declare a rule that's morally required but not alethically necessary — and check text against it.
**Prereqs:** Lesson H4

Alethic constraints are "this CANNOT be otherwise" — violating them makes the state impossible, so the engine rejects the apply. Deontic constraints are "this SHOULD not be otherwise" — violations are reported but the apply succeeds. They're the natural home for policy: "Each Order should be placed within 30 days of the quote" can be true today and false tomorrow without the database being broken.

`validate` takes raw text plus a constraint, extracts matching facts via the client LLM (or pre-supplied `llm_response`), and reports whether they satisfy the constraint. Content moderation, contract review, policy compliance — same mechanism.

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

**NOTE:** Under OWA (open-world), the absence of a "good standing" fact is unknown, not false. Deontic + OWA is sound but not complete — a reported violation is real, but no violation doesn't prove compliance.

**Next:** [Lesson H6: Objectification](./06-objectification.md)
