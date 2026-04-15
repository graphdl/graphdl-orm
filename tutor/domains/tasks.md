# Task Management

Exercises: assignment (ternary), priority enum, due dates, tags (M:N),
comments (parent-child), subset constraints, ring constraints (blocking),
multiple state machines, Mealy verbs, derivation rules.

## Entity Types

Task(.Task Id) is an entity type.
Project(.Project Code) is an entity type.
Person(.Username) is an entity type.
Comment(.id) is an entity type.
Milestone(.Milestone Name) is an entity type.

## Value Types

Task Id is a value type.
Project Code is a value type.
Username is a value type.
Milestone Name is a value type.
Title is a value type.
Body is a value type.
Priority is a value type.
  The possible values of Priority are 'critical', 'high', 'medium', 'low'.
Effort is a value type.
  The possible values of Effort are '1', '2', '3', '5', '8', '13'.
Due Date is a value type.
Tag is a value type.
Completion Percent is a value type.

## Readings

### Project

Project has Title.
  Each Project has exactly one Title.

Project has Description.
  Each Project has at most one Description.

### Person

Person has Display Name.
  Each Person has exactly one Display Name.

### Task

Task has Title.
  Each Task has exactly one Title.

Task has Body.
  Each Task has at most one Body.

Task has Priority.
  Each Task has exactly one Priority.

Task has Effort.
  Each Task has at most one Effort.

Task has Due Date.
  Each Task has at most one Due Date.

Task belongs to Project.
  Each Task belongs to exactly one Project.

Task is assigned to Person.
  Each Task is assigned to at most one Person.
  It is possible that some Person is assigned more than one Task.

Task has Tag.
  It is possible that some Task has more than one Tag.
  In each population of Task has Tag, each Task, Tag combination occurs at most once.

Task blocks Task.
  In each population of Task blocks Task, each Task, Task combination occurs at most once.
  No Task blocks the same Task. (irreflexive)

Task has parent Task.
  Each Task has at most one parent Task.

### Comment

Comment is on Task.
  Each Comment is on exactly one Task.

Comment has Body.
  Each Comment has exactly one Body.

Comment is by Person.
  Each Comment is by exactly one Person.

Comment replies to Comment.
  Each Comment replies to at most one Comment.

### Milestone

Milestone belongs to Project.
  Each Milestone belongs to exactly one Project.

Milestone has Due Date.
  Each Milestone has at most one Due Date.

Milestone has Completion Percent.
  Each Milestone has at most one Completion Percent.

Task targets Milestone.
  Each Task targets at most one Milestone.

## Constraints

-- Prefer the tagged ring-constraint shorthand to prose reasoning. The
-- shorthand is a single elementary assertion about the fact type.
Task blocks Task is acyclic.

-- Subset constraint between two binary fact types, stated elementally.
If some Task is assigned to some Person then that Task belongs to some Project.

It is obligatory that each Task has exactly one Priority.

## Derivation Rules

-- A compound aggregate stated as a single prose sentence is hard to read.
-- Decompose into named intermediates so each line is an elementary fact.
* Milestone has done Task Count iff done Task Count is the count of Task
  where Task targets that Milestone
  and Task has Status 'Done'.

* Milestone has total Task Count iff total Task Count is the count of Task
  where Task targets that Milestone.

* Milestone has Completion Percent iff Milestone has done Task Count
  and Milestone has total Task Count
  and Completion Percent is that done Task Count divided by that total Task Count.

## Instance Facts

### Task State Machine

State Machine Definition 'Task' is for Noun 'Task'.
Status 'Backlog' is defined in State Machine Definition 'Task'.
Status 'Todo' is defined in State Machine Definition 'Task'.
Status 'In Progress' is defined in State Machine Definition 'Task'.
Status 'In Review' is defined in State Machine Definition 'Task'.
Status 'Done' is defined in State Machine Definition 'Task'.
Status 'Backlog' is initial.

Transition 'prioritize' is from Status 'Backlog'.
Transition 'prioritize' is to Status 'Todo'.
Transition 'prioritize' is triggered by Event Type 'prioritize'.

Transition 'start' is from Status 'Todo'.
Transition 'start' is to Status 'In Progress'.
Transition 'start' is triggered by Event Type 'start'.

Transition 'review' is from Status 'In Progress'.
Transition 'review' is to Status 'In Review'.
Transition 'review' is triggered by Event Type 'submit-for-review'.

Transition 'approve' is from Status 'In Review'.
Transition 'approve' is to Status 'Done'.
Transition 'approve' is triggered by Event Type 'approve'.

Transition 'reject' is from Status 'In Review'.
Transition 'reject' is to Status 'In Progress'.
Transition 'reject' is triggered by Event Type 'request-changes'.

Transition 'reopen' is from Status 'Done'.
Transition 'reopen' is to Status 'Todo'.
Transition 'reopen' is triggered by Event Type 'reopen'.

Domain 'tasks' has Visibility 'public'.
