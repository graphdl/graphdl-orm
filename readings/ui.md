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

## Readings

Dashboard has Section.
  Each Section belongs to at most one Dashboard.
Section has Title.
  Each Section has at most one Title.
Section has ColumnCount.
  Each Section has at most one ColumnCount.
Section has Position.
  Each Section has at most one Position.
Section has Widget.
  Each Widget belongs to at most one Section.
Widget has Position.
  Each Widget has at most one Position.
Widget has WidgetType.
  Each Widget has at most one WidgetType.
Widget references Entity.
  Each Widget references at most one Entity.
Widget references Field.
  Each Widget references at most one Field.
Widget references Layer.
  Each Widget references at most one Layer.
Widget targets Widget.
  Each Widget targets each Widget at most once.
