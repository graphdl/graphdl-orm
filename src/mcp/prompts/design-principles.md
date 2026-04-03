# Design Principles

## Encapsulation

Never import another project's internal source files. Use published interfaces: REST APIs, CLI commands, SDK packages. A framework that exposes HTTP endpoints already HAS a public API.

## Separation of Concerns

Each module has one job. Domain readings belong with domain files. Generator code belongs with the generator. Metamodel facts (backed-by, URI patterns) go in graphdl-orm/readings/. Business instance data goes in support.auto.dev/readings/.

## Idempotency

Every operation must produce the same result when called multiple times. Use find-or-create patterns. Seeds, migrations, webhooks, handlers.
