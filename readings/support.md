# Support — Customer Support Domain

## Entity Types

Support Request(.id) is an entity type.
Category(.Name) is an entity type.
Support Response(.id) is an entity type.

## Fact Types

Support Request has Subject.
  Each Support Request has exactly one Subject.
Support Request has Body.
  Each Support Request has exactly one Body.
Support Request is from Customer.
  Each Support Request is from exactly one Customer.
Support Request has Category.
  Each Support Request has at most one Category.
Support Request has Priority.
  Each Support Request has at most one Priority.

Support Response is for Support Request.
  Each Support Response is for exactly one Support Request.
Support Response has Body.
  Each Support Response has exactly one Body.
Support Response is from User.
  Each Support Response is from at most one User.

Category has Description.
  Each Category has at most one Description.

## Constraints

It is obligatory that each Support Response is professional.
It is obligatory that each Support Response addresses the Subject of the Support Request.

## Derivation Rules

Support Request has Category iff
  the Category of the Support Request is derived from the Subject and Body of the Support Request.

## State Machine: Support Request Lifecycle

### Statuses

Received is a Status.
Categorized is a Status.
Responded is a Status.
Escalated is a Status.
Resolved is a Status.

### Transitions

Transition from Received to Categorized is triggered by categorize.
Transition from Categorized to Responded is triggered by respond.
Transition from Categorized to Escalated is triggered by escalate.
Transition from Responded to Received is triggered by reply.
Transition from Responded to Resolved is triggered by resolve.
Transition from Escalated to Responded is triggered by respond.

## Instance Facts

Category 'question' has Description 'General questions about products or services'.
Category 'feature-request' has Description 'Requests for new features or improvements'.
Category 'incident' has Description 'Production issues requiring immediate attention'.
