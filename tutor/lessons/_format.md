# Lesson format (contributor spec)

Every lesson is a single markdown file named `NN-kebab-title.md`. Lessons are consumed by the `tutor` MCP tool and by humans reading directly, so the format is readable both ways.

## Required sections

```markdown
# Lesson <track>.<num>: <TITLE>

**Goal:** <one-sentence outcome stated as a fact in the running app>
**Prereqs:** <comma-separated prior lessons, or "none">

<narrative — 2–4 short paragraphs. What the learner is about to do and why. Teach by doing; keep theory to a NOTE callout.>

## Do it

<Optionally one or more runnable fences — see "Runnable fences" below.>

## Check

~~~ expect
<a single predicate the tutor tool evaluates against current D; see "Expect grammar">
~~~

**Next:** [Lesson <next>](../<track>/<next-file>.md)
```

Optional sections that may appear between `## Do it` and `## Check`:

- `## Why` — one paragraph of theory. Always optional. Cite the whitepaper by theorem or equation when relevant.
- `## NOTE` — a single callout, italicized or quoted. Matches the vimtutor convention.

## Runnable fences

Each fence is one MCP tool call. The `tutor` tool reads the fence tag, builds the call, executes it, and shows the result inline. Fence bodies are plain text passed verbatim as the tool's `input` (for `compile`/`query`) or parsed as JSON (for `apply`/`propose`/`ask`/`synthesize`/`validate`).

| Tag | Tool | Body | Notes |
|-----|------|------|-------|
| `compile` | `compile` | FORML2 readings | Engine self-modifies; new D is persisted. |
| `apply` | `apply` | JSON args | `operation`, `noun`, `id?`, `fields?`, `event?`. |
| `query` | `query` | JSON `{fact_type, filter?}` | Returns JSON array. |
| `get` | `get` | JSON `{noun, id?}` | Omit id to list. |
| `actions` | `actions` | JSON `{noun, id}` | HATEOAS discovery. |
| `ask` | `ask` | `{question, noun?}` | Exercises client sampling. |
| `synthesize` | `synthesize` | `{noun, id?}` | Prose. |
| `validate` | `validate` | `{text, constraint}` | OWA deontic check. |
| `propose` | `propose` | `{rationale, target_domain, readings?, nouns?, constraints?, verbs?}` | Enters review workflow. |
| `explain` | `explain` | `{id, noun?, fact?}` | Derivation trace + audit. |

## Expect grammar

One predicate per lesson. The tutor tool flips ✗→✓ when it passes. Grammar (all forms evaluate against the live D):

```
expect ::= <query-expect> | <get-expect> | <status-expect> | <violation-expect>

query-expect    ::= query <fact_type> contains <json-object>
                  | query <fact_type> count <op> <int>
get-expect      ::= get <noun> <id> equals <json-object>
                  | list <noun> contains <json-object>
                  | list <noun> count <op> <int>
status-expect   ::= status <noun> <id> is <status-name>
violation-expect::= violations for apply <op> <noun> <json> include <constraint-id>

op ::= == | >= | <= | > | <
```

Example:

```
list Order contains {"id": "o1", "total": "100"}
```

## Track conventions

- **Easy** — LLM-driven. Most fences are `propose`, `ask`, `synthesize`, or conversational `apply`. The learner edits prose, not FORML2.
- **Medium** — Tool-call literacy. Fences are explicit `apply`, `get`, `query`, `actions`, `explain`.
- **Hard** — FORML2 authorship. Fences are `compile` blocks the learner writes from scratch.

## Non-goals

- No state persisted between lessons by default. Each lesson's ✓ check should be self-contained against facts it declares. When a lesson does assume prior state, mark it explicitly with `**Prereqs:** Lesson <x.y>`.
- No branching. Lessons are a linear walk per track.
- No keyboard shortcuts. The tutor is driven by MCP tool calls, not terminal input.
