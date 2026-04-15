# Scheduling

Exercises: temporal value types, recurrence patterns, availability (ternary),
booking (objectification with spanning UC), conflict detection via constraints,
transitive ring constraints.

## Entity Types

Room(.Room Number) is an entity type.
Meeting(.Meeting Id) is an entity type.
Attendee(.Email) is an entity type.
Booking(.id) is an entity type where Meeting is held in Room.

## Value Types

Room Number is a value type.
Meeting Id is a value type.
Email is a value type.
Meeting Title is a value type.
Start Time is a value type.
End Time is a value type.
Duration is a value type.
Capacity is a value type.
Floor is a value type.
Recurrence is a value type.
  The possible values of Recurrence are 'none', 'daily', 'weekly', 'biweekly', 'monthly'.
Is Required is a value type.

## Readings

### Room

Room has Room Name.
  Each Room has exactly one Room Name.

Room has Capacity.
  Each Room has exactly one Capacity.

Room has Floor.
  Each Room has at most one Floor.

### Meeting

Meeting has Meeting Title.
  Each Meeting has exactly one Meeting Title.

Meeting has Start Time.
  Each Meeting has exactly one Start Time.

Meeting has End Time.
  Each Meeting has exactly one End Time.

Meeting has Recurrence.
  Each Meeting has exactly one Recurrence.

Meeting is organized by Attendee.
  Each Meeting is organized by exactly one Attendee.

Meeting is held in Room.
  Each Meeting is held in at most one Room.
  In each population of Meeting is held in Room, each Meeting, Room combination occurs at most once.

This association with Meeting, Room provides the preferred identification scheme for Booking.

Attendee is invited to Meeting.
  In each population of Attendee is invited to Meeting, each Attendee, Meeting combination occurs at most once.

Attendee is required for Meeting.

### Booking (objectification of "Meeting is held in Room")

Booking is confirmed.

## Constraints

It is obligatory that each Meeting has exactly one Start Time.
It is obligatory that each Meeting has exactly one End Time.

If some Attendee is required for some Meeting then that Attendee is invited to that Meeting.

## Derivation Rules

-- Each premise below is an elementary fact type reading.
-- Prefer this layout over a single prose sentence with multiple "and"s.
* Meeting has Duration iff Meeting has End Time
  and Meeting has Start Time
  and Duration is that End Time minus that Start Time.

-- Subset constraint stated as a single elementary implication.
If some Meeting is organized by some Attendee then that Attendee is invited to that Meeting.

## Instance Facts

Domain 'scheduling' has Visibility 'public'.
