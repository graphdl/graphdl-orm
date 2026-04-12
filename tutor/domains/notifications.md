# Notifications

Exercises: event-driven fact creation, channels (OR constraint — at least one),
delivery tracking state machine, user preferences, deontic permissions,
open world assumption.

## Entity Types

Notification(.Notification Id) is an entity type.
Channel(.Channel Name) is an entity type.
Preference(.id) is an entity type.

## Value Types

Notification Id is a value type.
Channel Name is a value type.
Subject is a value type.
Message Body is a value type.
Recipient Email is a value type.
Sent At is a value type.
Read At is a value type.
Urgency is a value type.
  The possible values of Urgency are 'low', 'normal', 'high', 'critical'.

## Readings

### Notification

Notification has Subject.
  Each Notification has exactly one Subject.

Notification has Message Body.
  Each Notification has exactly one Message Body.

Notification has Recipient Email.
  Each Notification has exactly one Recipient Email.

Notification has Urgency.
  Each Notification has exactly one Urgency.

Notification has Sent At.
  Each Notification has at most one Sent At.

Notification has Read At.
  Each Notification has at most one Read At.

Notification is delivered via Channel.
  Each Notification is delivered via at least one Channel.
  In each population of Notification is delivered via Channel, each Notification, Channel combination occurs at most once.

### Channel

Channel has Description.
  Each Channel has at most one Description.

### Preference

Preference is for Recipient Email.
  Each Preference is for exactly one Recipient Email.

Preference enables Channel.
  Each Preference enables at least one Channel.

Preference has Urgency threshold.
  Each Preference has at most one Urgency threshold.

## Constraints

Each Notification is delivered via some Channel.

It is permitted that a Notification is delivered via more than one Channel.

It is forbidden that a Notification is sent to a Recipient Email that has no Preference.

## Instance Facts

Channel 'email' has Channel Name 'Email'.
Channel 'email' has Description 'Standard email delivery'.

Channel 'sms' has Channel Name 'SMS'.
Channel 'sms' has Description 'Text message delivery'.

Channel 'push' has Channel Name 'Push'.
Channel 'push' has Description 'Mobile push notification'.

Channel 'webhook' has Channel Name 'Webhook'.
Channel 'webhook' has Description 'HTTP webhook callback'.

### Notification State Machine

State Machine Definition 'Notification' is for Noun 'Notification'.
Status 'Queued' is defined in State Machine Definition 'Notification'.
Status 'Sending' is defined in State Machine Definition 'Notification'.
Status 'Sent' is defined in State Machine Definition 'Notification'.
Status 'Delivered' is defined in State Machine Definition 'Notification'.
Status 'Read' is defined in State Machine Definition 'Notification'.
Status 'Failed' is defined in State Machine Definition 'Notification'.
Status 'Queued' is initial.

Transition 'send' is from Status 'Queued'.
Transition 'send' is to Status 'Sending'.
Transition 'send' is triggered by Event Type 'send'.

Transition 'confirm-sent' is from Status 'Sending'.
Transition 'confirm-sent' is to Status 'Sent'.
Transition 'confirm-sent' is triggered by Event Type 'confirm-sent'.

Transition 'confirm-delivery' is from Status 'Sent'.
Transition 'confirm-delivery' is to Status 'Delivered'.
Transition 'confirm-delivery' is triggered by Event Type 'confirm-delivery'.

Transition 'mark-read' is from Status 'Delivered'.
Transition 'mark-read' is to Status 'Read'.
Transition 'mark-read' is triggered by Event Type 'mark-read'.

Transition 'fail-sending' is from Status 'Sending'.
Transition 'fail-sending' is to Status 'Failed'.
Transition 'fail-sending' is triggered by Event Type 'fail'.

Transition 'retry' is from Status 'Failed'.
Transition 'retry' is to Status 'Queued'.
Transition 'retry' is triggered by Event Type 'retry'.

Domain 'notifications' has Visibility 'public'.
