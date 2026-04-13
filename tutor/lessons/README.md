# arest-tutor: the interactive tour

The tutor is a hands-on walk through AREST, organized into three tracks and built in the vimtutor spirit: **teach by doing**. You read one lesson, run the embedded blocks, and the tutor flips a ✗ to ✓ when the check passes. Then you move on to the next.

Every lesson is a single markdown file. You can read them directly, but the intended experience runs through the `tutor` MCP tool, which executes the blocks and grades the checks for you.

## Tracks

**[Easy](./easy/)** takes you from zero to a running app in about five minutes. You describe what you want in plain English and watch the app materialize. The lessons use `propose`, `ask`, `synthesize`, and conversational `apply`. There are four lessons in this track.

**[Medium](./medium/)** asks you to drive the engine yourself. The lessons cover tool-call literacy with explicit `apply`, `get`, `query`, `actions`, and `explain`. There are five lessons. The prerequisite is the Easy track, or a working mental model of entities and fact types.

**[Hard](./hard/)** has you author the schema directly. You write FORML2 readings by hand, add constraints, derivation rules, and a state machine, and you finish with self-modification at runtime. There are eight lessons. The prerequisite is the Medium track.

## Start

```
~~~ tutor
{ "command": "start", "track": "easy" }
~~~
```

The tutor loads the first lesson, renders it, and waits. You run the fences, the tutor grades the check, and it hands you a `next` pointer.

## For contributors

See [_format.md](./_format.md) for the lesson file format and the fence grammar.

## Lesson index

### Easy
- [Lesson E1: Describe your app](./easy/01-propose.md)
- [Lesson E2: Ask a question](./easy/02-ask.md)
- [Lesson E3: Get a summary](./easy/03-synthesize.md)
- [Lesson E4: Move an order along](./easy/04-transition-conversationally.md)

### Medium
- [Lesson M1: Create an entity](./medium/01-apply-create.md)
- [Lesson M2: Read it back](./medium/02-get-list-query.md)
- [Lesson M3: Discover what you can do](./medium/03-actions.md)
- [Lesson M4: Fire a transition](./medium/04-apply-transition.md)
- [Lesson M5: Explain what happened](./medium/05-explain.md)

### Hard
- [Lesson H1: Declare a noun](./hard/01-noun.md)
- [Lesson H2: A binary fact with a UC](./hard/02-binary-fact-uc.md)
- [Lesson H3: A ternary with a spanning UC](./hard/03-ternary-spanning-uc.md)
- [Lesson H4: A derivation rule](./hard/04-derivation-rule.md)
- [Lesson H5: A deontic constraint](./hard/05-deontic-constraint.md)
- [Lesson H6: Objectification](./hard/06-objectification.md)
- [Lesson H7: A declared state machine](./hard/07-declared-sm.md)
- [Lesson H8: Self-modification at runtime](./hard/08-self-modification.md)
