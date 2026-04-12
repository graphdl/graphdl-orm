# Lesson E2: ASK A QUESTION

**Goal:** Answer an English question from the live population without writing a query.
**Prereqs:** Lesson E1

Once an app is running you can interrogate it the same way you described it — in English. `ask` translates the question into a projection over the facts in D, executes it, and returns the matching rows. If your MCP client supports sampling the round-trip is seamless; if not, the tutor will hand you the prompt to run manually and accept the answer via `llm_response`.

This is the "Sherlock" moment the paper points at: the engine evaluates the logical chain; you read the conclusion.

## Do it

~~~ apply
{ "operation": "create", "noun": "Order", "id": "acme-1", "fields": { "Customer": "acme", "Amount": "250" } }
~~~

~~~ ask
{ "question": "How many orders does acme have?" }
~~~

## Check

~~~ expect
query Order_was_placed_by_Customer count >= 1
~~~

**Next:** [Lesson E3: Get a summary](./03-synthesize.md)
