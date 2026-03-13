# GraphDL UI — Presentation Layer

## Entity Types

| Entity | Reference Scheme |
|--------|-----------------|
| Dashboard | DashboardName |
| Section | SectionTitle |
| Widget | WidgetId |

## Value Types

| Value | Type | Constraints |
|-------|------|-------------|
| WidgetType | string | enum: link, field, status-summary, submission, streaming, remote-control |
| Position | number | |
| ColumnCount | number | |
| DisplayColor | string | enum: green, amber, red, blue, violet, gray |

## Readings

### Status
Status has DisplayColor.
  Each Status has at most one DisplayColor.


Dashboard has Section.
  Each Section belongs to exactly one Dashboard.
Section has Title.
  Each Section has exactly one Title.
Section has ColumnCount.
  Each Section has at most one ColumnCount.
Section has Position.
  Each Section has exactly one Position.
Section has Widget.
  Each Widget belongs to exactly one Section.
Widget has Position.
  Each Widget has exactly one Position.
Widget has WidgetType.
  Each Widget has exactly one WidgetType.
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

An EntityList is a live view of Resource instances that automatically reflects creation, update, and deletion. It supports server-driven pagination that maps directly to API pagination responses.

## Entity Types

| Entity | Reference Scheme |
|--------|-----------------|
| EntityList | (Noun + Domain) |
| ListItem | (EntityList + Resource) |
| Page | (EntityList + PageNumber) |

## Value Types

| Value | Type | Constraints |
|-------|------|-------------|
| DisplayText | string | |
| DisplaySubtext | string | |
| DisplayStatus | string | |
| DisplayImagePath | string | URL |
| PollingInterval | number | milliseconds, default 5000 |
| ChangeType | string | enum: created, updated, deleted |
| ScrollStyle | string | enum: infinite-scroll, paginated, load-more |
| PageSize | number | default 20 |
| PageNumber | number | starts at 1 |
| TotalDocs | number | |
| TotalPages | number | |
| SortField | string | |
| SortDirection | string | enum: asc, desc |
| StatusFilter | string | |

## Fact Types

### EntityList
EntityList displays Resource instances of Noun.
  Each EntityList displays instances of exactly one Noun.
EntityList belongs to Domain.
  Each EntityList belongs to exactly one Domain.
EntityList has PollingInterval.
  Each EntityList has at most one PollingInterval.
EntityList has ScrollStyle.
  Each EntityList has at most one ScrollStyle.
EntityList has PageSize.
  Each EntityList has at most one PageSize.
EntityList has TotalDocs.
  Each EntityList has at most one TotalDocs.
EntityList has TotalPages.
  Each EntityList has at most one TotalPages.
EntityList has StatusFilter.
  Each EntityList has at most one StatusFilter.
EntityList has SortField.
  Each EntityList has at most one SortField.
EntityList has SortDirection.
  Each EntityList has at most one SortDirection.

### Page
Page belongs to EntityList.
  Each Page belongs to exactly one EntityList.
Page has PageNumber.
  Each Page has exactly one PageNumber.

### ListItem
ListItem belongs to Page.
  Each ListItem belongs to exactly one Page.
ListItem displays Resource.
  Each ListItem displays exactly one Resource.
ListItem has DisplayText.
  Each ListItem has at most one DisplayText.
ListItem has DisplaySubtext.
  Each ListItem has at most one DisplaySubtext.
ListItem has DisplayStatus.
  Each ListItem has at most one DisplayStatus.
ListItem has DisplayImagePath.
  Each ListItem has at most one DisplayImagePath.

## Constraints

Each EntityList has at most one ListItem per Resource.
Each EntityList has at most one Page per PageNumber.

## Derivation Rules

ListItem DisplayText := Resource reference or Resource value or Resource id.
ListItem DisplaySubtext := Resource value when Resource has both reference and value.
ListItem DisplayStatus := StateMachine currentStatus name where StateMachine is for Resource.
ListItem DisplayImagePath := Resource imagePath when Resource has imagePath.
EntityList TotalDocs := API response totalDocs.
EntityList TotalPages := API response totalPages.
Page PageNumber := API response page.

## Deontic Constraints

It is obligatory that EntityList pagination parameters (PageSize, PageNumber, SortField, SortDirection) map one-to-one to API query parameters (limit, page, sort).
It is obligatory that when a Resource is created matching EntityList Noun and Domain, a ListItem is added to EntityList.
It is obligatory that when a Resource DisplayText, DisplaySubtext, DisplayStatus, or DisplayImagePath changes, the corresponding ListItem properties are updated.
It is obligatory that when a Resource is deleted, the corresponding ListItem is removed from EntityList.
It is obligatory that when a ListItem is removed from a paginated EntityList and TotalDocs exceeds the displayed count, EntityList fetches the next Resource to fill the gap.
It is obligatory that when ScrollStyle is 'infinite-scroll', loading the next Page appends ListItems to the existing list rather than replacing it.
It is obligatory that when ScrollStyle is 'paginated', loading a Page replaces the current ListItems.
It is obligatory that when ScrollStyle is 'load-more', activating load appends the next Page of ListItems.
