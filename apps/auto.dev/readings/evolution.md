# Auto.dev Domain Change Signals

## Description

Auto.dev-specific signal-originating entities. Feature Request, Support
Request, and Error Pattern each relate directly to a Domain Change as
concrete alternatives to the metamodel's abstract Signal entity (which
carries Signal Source as an enum). An auto.dev Domain Change can be
caused by one concrete signal entity per category; the metamodel's
deontic guard still applies (core/organizations/evolution changes
require a human signal).

## Cross-domain References

Feature Request (from feature-requests)
Support Request (from support)
Error Pattern (from error-monitoring)
Domain Change (from metamodel evolution)

## Fact Types

### Domain Change

Feature Request leads to Domain Change.
Support Request leads to Domain Change.
Error Pattern leads to Domain Change.

## Constraints

Each Domain Change belongs to at most one Feature Request.
Each Domain Change belongs to at most one Support Request.
Each Domain Change belongs to at most one Error Pattern.
