# GraphDL UI — Presentation Layer

## Entity Types

| Entity | Reference Scheme |
|--------|-----------------|
| Dashboard | Dashboard Name |
| Section | Section Title |
| Widget | Widget Id |

## Value Types

| Value | Type | Constraints |
|-------|------|-------------|
| Widget Type | string | enum: link, field, status-summary, submission, streaming, remote-control |
| Position | number | |
| Column Count | number | |
| Display Color | string | enum: green, amber, red, blue, violet, gray |

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

| Entity | Reference Scheme |
|--------|-----------------|
| Entity List | (Noun + Domain) |
| List Item | (Entity List + Resource) |
| Page | (Entity List + Page Number) |

## Value Types

| Value | Type | Constraints |
|-------|------|-------------|
| Display Text | string | |
| Display Subtext | string | |
| Display Status | string | |
| Display Image Path | string | URL |
| Polling Interval | number | milliseconds, default 5000 |
| Change Type | string | enum: created, updated, deleted |
| Scroll Style | string | enum: infinite-scroll, paginated, load-more |
| Page Size | number | default 20 |
| Page Number | number | starts at 1 |
| Total Docs | number | |
| Total Pages | number | |
| Sort Field | string | |
| Sort Direction | string | enum: asc, desc |
| Status Filter | string | |

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

List Item Display Text := Resource reference or Resource value or Resource id.
List Item Display Subtext := Resource value when Resource has both reference and value.
List Item Display Status := State Machine currentStatus name where State Machine is for Resource.
List Item Display Image Path := Resource imagePath when Resource has imagePath.
Entity List Total Docs := API response totalDocs.
Entity List Total Pages := API response totalPages.
Page Page Number := API response page.

## Deontic Constraints

It is obligatory that Entity List pagination parameters (Page Size, Page Number, Sort Field, Sort Direction) map one-to-one to API query parameters (limit, page, sort).
It is obligatory that when a Resource is created matching Entity List Noun and Domain, a List Item is added to Entity List.
It is obligatory that when a Resource Display Text, Display Subtext, Display Status, or Display Image Path changes, the corresponding List Item properties are updated.
It is obligatory that when a Resource is deleted, the corresponding List Item is removed from Entity List.
It is obligatory that when a List Item is removed from a paginated Entity List and Total Docs exceeds the displayed count, Entity List fetches the next Resource to fill the gap.
It is obligatory that when Scroll Style is 'infinite-scroll', loading the next Page appends List Items to the existing list rather than replacing it.
It is obligatory that when Scroll Style is 'paginated', loading a Page replaces the current List Items.
It is obligatory that when Scroll Style is 'load-more', activating load appends the next Page of List Items.
