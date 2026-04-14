# Auto.dev Domain Change Signals

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

## Instance Facts

Domain 'auto.dev-signals' has Access 'private'.
Domain 'auto.dev-signals' has Description 'Concrete signal-originating entities (Feature Request, Support Request, Error Pattern) each relating directly to a Domain Change, as a concrete alternative to the metamodel abstract Signal entity with Signal Source enum.'.
