# GraphDL UI — Presentation Layer

## Entity Types

Dashboard(.Dashboard Name) is an entity type.
Section(.Section Title) is an entity type.
Widget(.Widget Id) is an entity type.

## Value Types

Dashboard Name is a value type.
Section Title is a value type.
Widget Id is a value type.
Widget Type is a value type.
  The possible values of Widget Type are 'link', 'field', 'status-summary', 'submission', 'streaming', 'remote-control'.
Position is a value type.
Column Count is a value type.
Display Color is a value type.
  The possible values of Display Color are 'green', 'amber', 'red', 'blue', 'violet', 'gray'.

## Readings

### Status
Status has Display Color.
  Each Status has at most one Display Color.


Dashboard has Section.
  Each Section belongs to exactly one Dashboard.
Section has Title.
  Each Section has exactly one Title.
Section has Column Count.
  Each Section has at most one Column Count.
Section has Position.
  Each Section has exactly one Position.
Section has Widget.
  Each Widget belongs to exactly one Section.
Widget has Position.
  Each Widget has exactly one Position.
Widget has Widget Type.
  Each Widget has exactly one Widget Type.
Widget references Entity.
  Each Widget references at most one Entity.
Widget references Field.
  Each Widget references at most one Field.
Widget references Layer.
  Each Widget references at most one Layer.
Widget targets Widget.
  Each Widget targets each Widget at most once.
  No Widget targets itself.

---

# Reactive Entity List

An Entity List is a live view of Resource instances that automatically reflects creation, update, and deletion. It supports server-driven pagination that maps directly to API pagination responses.

## Entity Types

Entity List(.Noun + Domain) is an entity type.
List Item(.Entity List + Resource) is an entity type.
Page(.Entity List + Page Number) is an entity type.

## Value Types

Display Text is a value type.
Display Subtext is a value type.
Display Status is a value type.
Display Image Path is a value type.
Polling Interval is a value type.
Change Type is a value type.
  The possible values of Change Type are 'created', 'updated', 'deleted'.
Scroll Style is a value type.
  The possible values of Scroll Style are 'infinite-scroll', 'paginated', 'load-more'.
Page Size is a value type.
Page Number is a value type.
Total Docs is a value type.
Total Pages is a value type.
Sort Field is a value type.
Sort Direction is a value type.
  The possible values of Sort Direction are 'asc', 'desc'.
Status Filter is a value type.

## Fact Types

### Entity List
Entity List displays Resource instances of Noun.
  Each Entity List displays instances of exactly one Noun.
Entity List belongs to Domain.
  Each Entity List belongs to exactly one Domain.
Entity List has Polling Interval.
  Each Entity List has at most one Polling Interval.
Entity List has Scroll Style.
  Each Entity List has at most one Scroll Style.
Entity List has Page Size.
  Each Entity List has at most one Page Size.
Entity List has Total Docs.
  Each Entity List has at most one Total Docs.
Entity List has Total Pages.
  Each Entity List has at most one Total Pages.
Entity List has Status Filter.
  Each Entity List has at most one Status Filter.
Entity List has Sort Field.
  Each Entity List has at most one Sort Field.
Entity List has Sort Direction.
  Each Entity List has at most one Sort Direction.

### Page
Page belongs to Entity List.
  Each Page belongs to exactly one Entity List.
Page has Page Number.
  Each Page has exactly one Page Number.

### List Item
List Item belongs to Page.
  Each List Item belongs to exactly one Page.
List Item displays Resource.
  Each List Item displays exactly one Resource.
List Item has Display Text.
  Each List Item has at most one Display Text.
List Item has Display Subtext.
  Each List Item has at most one Display Subtext.
List Item has Display Status.
  Each List Item has at most one Display Status.
List Item has Display Image Path.
  Each List Item has at most one Display Image Path.

## Constraints

Each Entity List has at most one List Item per Resource.
Each Entity List has at most one Page per Page Number.

## Derivation Rules

List Item has Display Text iff List Item displays Resource and that Resource has Reference and Display Text is that Reference.
List Item has Display Text iff List Item displays Resource and that Resource has Value and that Resource has no Reference and Display Text is that Value.
List Item has Display Subtext iff List Item displays Resource and that Resource has Reference and that Resource has Value and Display Subtext is that Value.
List Item has Display Status iff List Item displays Resource and some State Machine is for that Resource and that State Machine is currently in some Status and Display Status is that Status.
List Item has Display Image Path iff List Item displays Resource and that Resource has Value and Display Image Path is that Value.

## Deontic Constraints

It is obligatory that each Entity List has some Page Size.
It is obligatory that each Entity List has some Sort Field.
It is obligatory that each Entity List has some Sort Direction.

If some Resource is instance of some Noun and some Entity List displays instances of that Noun and that Resource belongs to some Domain and that Entity List belongs to that Domain then some List Item displays that Resource and that List Item belongs to some Page of that Entity List.
If some List Item displays some Resource and that Resource is deleted then that List Item is removed.
If some Entity List has Scroll Style 'infinite-scroll' then for each Page of that Entity List, that Page appends to the existing List Items.
If some Entity List has Scroll Style 'paginated' then for each Page of that Entity List, that Page replaces the current List Items.

## Instance Facts

Domain 'ui' has Visibility 'public'.
