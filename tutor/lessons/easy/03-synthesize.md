# Lesson E3: GET A SUMMARY

**Goal:** Turn an entity's facts into readable prose.
**Prereqs:** Lesson E2

`synthesize` runs the full pipeline (resolve + derive to LFP + validate) so every implicit fact is materialized, then hands the fact bag to the client LLM with a prompt that says "verbalize these, don't invent." The engine guarantees content correctness; the LLM only shapes wording.

This is how you'd build an "AI summary" panel on a dashboard without any hallucination budget — everything in the prose is grounded in a fact that passed constraints.

## Do it

~~~ synthesize
{ "noun": "Order", "id": "acme-1" }
~~~

## Check

~~~ expect
get Order acme-1 equals {"id": "acme-1", "Customer": "acme", "Amount": "250"}
~~~

**NOTE:** If the client doesn't support MCP sampling, `synthesize` returns `mode: "prompt-only"` with the prompt and the fact bag. Run the prompt anywhere, pass the prose back as `llm_response`, and the tool completes.

**Next:** [Lesson E4: Move an order along](./04-transition-conversationally.md)
