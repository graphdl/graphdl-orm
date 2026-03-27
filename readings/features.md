# Feature Requests

Product feedback loop. Feature requests originate from support requests
or direct submissions. They can lead to domain changes when approved.

## Entity Types

Feature Request(.Feature Request Id) is an entity type.

## Value Types

Feature Request Id is a value type.
Vote Count is a value type.
Priority is a value type.
  The possible values of Priority are 'low', 'medium', 'high', 'urgent'.

## Readings

Support Request leads to Feature Request.
  Each Support Request leads to at most one Feature Request.

Feature Request has Vote Count.
  Each Feature Request has at most one Vote Count.

Feature Request has Priority.
  Each Feature Request has at most one Priority.

User votes on Feature Request.
  Each User, Feature Request combination occurs at most once in the population of User votes on Feature Request.

Feature Request leads to Domain Change.
  Each Feature Request leads to at most one Domain Change.

## Instance Facts

State Machine Definition 'Feature Request' is for Noun 'Feature Request'.
Status 'Proposed' is defined in State Machine Definition 'Feature Request'.
Status 'Approved' is defined in State Machine Definition 'Feature Request'.
Status 'In Progress' is defined in State Machine Definition 'Feature Request'.
Status 'Shipped' is defined in State Machine Definition 'Feature Request'.
Status 'Proposed' is initial.

Transition 'approve' is from Status 'Proposed'.
Transition 'approve' is to Status 'Approved'.
Transition 'approve' is triggered by Event Type 'approve'.

Transition 'startWork' is from Status 'Approved'.
Transition 'startWork' is to Status 'In Progress'.
Transition 'startWork' is triggered by Event Type 'start-work'.

Transition 'deploy' is from Status 'In Progress'.
Transition 'deploy' is to Status 'Shipped'.
Transition 'deploy' is triggered by Event Type 'deploy'.

Domain 'features' has Visibility 'public'.
