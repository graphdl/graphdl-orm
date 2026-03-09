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

Dashboard has Section (1:*)
Section has Title (*:1)
Section has ColumnCount (*:1)
Section has Position (*:1)
Section has Widget (1:*)
Widget has Position (*:1)
Widget has WidgetType (*:1)
Widget references Entity (*:1)
Widget references Field (*:1)
Widget references Layer (*:1)
Widget targets Widget (*:*)
