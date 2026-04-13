# AREST UI: Presentation Layer

## Description

Platform-agnostic view hierarchy, navigation, and controls. Custom renderers registered per platform via the factory pattern.

## Entity Types

### Views
View(.id) is an entity type.
  List View is a subtype of View.
  Browser View is a subtype of View.
  Grid View is a subtype of View.
  Canvas View is a subtype of View.
  Tab View is a subtype of View.
  {List View, Browser View, Grid View, Canvas View, Tab View} are mutually exclusive subtypes of View.

### Navigation
History Stack(.id) is an entity type.
Tab Item(.id) is an entity type.

### Cells and Content
Cell(.id) is an entity type.
  Grid Cell is a subtype of Cell.
  Rich Content Cell is a subtype of Cell.
  {Grid Cell, Rich Content Cell} are mutually exclusive subtypes of Cell.

Section(.id) is an entity type.
Section Header(.id) is an entity type.
Section Footer(.id) is an entity type.

### Controls
Control(.id) is an entity type.
  Button is a subtype of Control.
  Text Box is a subtype of Control.
  Text Area is a subtype of Control.
  Password Box is a subtype of Control.
  Label is a subtype of Control.
  Date Picker is a subtype of Control.
  Time Picker is a subtype of Control.
  Slider is a subtype of Control.
  Switch is a subtype of Control.
  Select List is a subtype of Control.
  Image is a subtype of Control.
  Checkbox is a subtype of Control.
  {Button, Text Box, Text Area, Password Box, Label, Date Picker, Time Picker, Slider, Switch, Select List, Image, Checkbox} are mutually exclusive subtypes of Control.

### Menus and Toolbars
Menu(.id) is an entity type.
Menu Button(.id) is an entity type.
Toolbar(.id) is an entity type.
Toolbar Item(.id) is an entity type.
  Toolbar Button is a subtype of Toolbar Item.
  Toolbar Separator is a subtype of Toolbar Item.
  {Toolbar Button, Toolbar Separator} are mutually exclusive subtypes of Toolbar Item.

Search Box(.id) is an entity type.

### Layout
Element(.id) is an entity type.

View is a subtype of Element.
Control is a subtype of Element.
Cell is a subtype of Element.
Menu is a subtype of Element.
Toolbar is a subtype of Element.
Search Box is a subtype of Element.

Dashboard(.Name) is an entity type.
Widget(.Widget Id) is an entity type.
Entity List(.Noun + Domain) is an entity type.
List Item(.Entity List + Resource) is an entity type.
Page(.Entity List + Page Number) is an entity type.

### Platform Registration
View Renderer(.id) is an entity type.

## Value Types

Title is a value type.
Color is a value type.
Font is a value type.
Content Stretch is a value type.
  The possible values of Content Stretch are 'none', 'fill', 'uniform', 'uniform-to-fill'.
Preferred Orientation is a value type.
  The possible values of Preferred Orientation are 'portrait', 'landscape', 'portrait-or-landscape'.
Column Mode is a value type.
List View Style is a value type.
Selection Style is a value type.
Pane is a value type.
  The possible values of Pane are 'master', 'detail', 'popover'.
Horizontal Alignment is a value type.
  The possible values of Horizontal Alignment are 'left', 'center', 'right', 'stretch'.
Vertical Alignment is a value type.
  The possible values of Vertical Alignment are 'top', 'center', 'bottom', 'stretch'.
Visibility is a value type.
  The possible values of Visibility are 'visible', 'hidden', 'collapsed'.
Keyboard Type is a value type.
  The possible values of Keyboard Type are 'default', 'number', 'decimal', 'phone', 'email', 'url'.
Text Alignment is a value type.
  The possible values of Text Alignment are 'left', 'center', 'right'.
Text Completion is a value type.
  The possible values of Text Completion are 'none', 'offer-suggestions', 'auto-correct'.
Widget Id is a value type.
Widget Type is a value type.
  The possible values of Widget Type are 'link', 'field', 'status-summary', 'submission', 'streaming', 'remote-control'.
Display Color is a value type.
  The possible values of Display Color are 'green', 'amber', 'red', 'blue', 'violet', 'gray'.
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
Column Count is a value type.
Position is a value type.
Image Path is a value type.
Placeholder is a value type.
Expression is a value type.
Submit Key is a value type.
Min Value is a value type.
Max Value is a value type.
Badge Value is a value type.
Platform is a value type.
  The possible values of Platform are 'web', 'ios', 'android', 'macos', 'windows', 'linux', 'terminal', 'wearable', 'tv', 'embedded'.

## Fact Types

### View (base properties)
View has Title.
  Each View has at most one Title.
View has Color as header color.
  Each View has at most one header Color.
View has Color as title color.
  Each View has at most one title Color.
View has Preferred Orientation.
  Each View has at most one Preferred Orientation.
Noun is displayed by Element.
  Each Noun is displayed by at most one Element.

### List View
List View has Column Mode.
  Each List View has at most one Column Mode.
List View has List View Style.
  Each List View has at most one List View Style.
List View has Menu.
  Each List View has at most one Menu.
List View has Search Box.
  Each List View has at most one Search Box.
List View has Section.

### Browser View
Browser View has Menu.
  Each Browser View has at most one Menu.

### Grid View
Grid View has Menu.
  Each Grid View has at most one Menu.

### Canvas View
Canvas View has Toolbar.
  Each Canvas View has at most one Toolbar.
Canvas View has Color as stroke color.
  Each Canvas View has at most one stroke Color.

### Tab View
Tab View has Tab Item.
Tab View has Color as selection color.
  Each Tab View has at most one selection Color.

### History Stack (navigation)
View is in History Stack.
  Each View is in at most one History Stack.
History Stack has Pane.
  Each History Stack has at most one Pane.

### Tab Item
Tab Item has Title.
  Each Tab Item has exactly one Title.
Tab Item has Image Path.
  Each Tab Item has at most one Image Path.
Tab Item has Badge Value.
  Each Tab Item has at most one Badge Value.

### Section
Section belongs to List View.
  Each Section belongs to exactly one List View.
Section has Section Header.
  Each Section has at most one Section Header.
Section has Section Footer.
  Each Section has at most one Section Footer.
Section has Cell.

### Cell
Cell has Color as background color.
  Each Cell has at most one background Color.
Grid Cell has Selection Style.
  Each Grid Cell has at most one Selection Style.

### Control (base)
Control has Submit Key.
  Each Control has at most one Submit Key.
Control is enabled.

### Button
Button has Title.
  Each Button has exactly one Title.
Button has Image Path.
  Each Button has at most one Image Path.

### Text Box
Text Box has Placeholder.
  Each Text Box has at most one Placeholder.
Text Box has Expression.
  Each Text Box has at most one Expression.
Text Box has Keyboard Type.
  Each Text Box has at most one Keyboard Type.
Text Box has Text Alignment.
  Each Text Box has at most one Text Alignment.
Text Box has Text Completion.
  Each Text Box has at most one Text Completion.

### Date Picker
Date Picker has date- Format.
  Each Date Picker has at most one date- Format.

### Time Picker
Time Picker has time- Format.
  Each Time Picker has at most one time- Format.

### Slider
Slider has Min Value.
  Each Slider has exactly one Min Value.
Slider has Max Value.
  Each Slider has exactly one Max Value.

### Select List
Select List has items.

### Menu
Menu has Title.
  Each Menu has at most one Title.
Menu has Image Path.
  Each Menu has at most one Image Path.
Menu has Menu Button.

### Menu Button
Menu Button has Title.
  Each Menu Button has exactly one Title.
Menu Button has Image Path.
  Each Menu Button has at most one Image Path.

### Toolbar
Toolbar has Toolbar Item as primary.
Toolbar has Toolbar Item as secondary.

### Toolbar Button
Toolbar Button has Title.
  Each Toolbar Button has at most one Title.
Toolbar Button has Image Path.
  Each Toolbar Button has at most one Image Path.

### Search Box
Search Box has Placeholder.
  Each Search Box has at most one Placeholder.
Search Box has Text Completion.
  Each Search Box has at most one Text Completion.

### Element (grid positioning)
Element has Position as column index.
Element has Position as row index.
Element has Horizontal Alignment.
  Each Element has at most one Horizontal Alignment.
Element has Vertical Alignment.
  Each Element has at most one Vertical Alignment.
Element has Visibility.
  Each Element has at most one Visibility.

### View Renderer (custom view registration)
View Renderer is for View.
  Each View Renderer is for exactly one View.
View Renderer is on Platform.
  Each View Renderer is on exactly one Platform.
View Renderer has component Name.
  Each View Renderer has exactly one component Name.

### Status display
Status has Display Color.
  Each Status has at most one Display Color.

### Dashboard layout
Dashboard has Widget.
  Each Widget belongs to exactly one Dashboard.
Widget has Position.
  Each Widget has exactly one Position.
Widget has Widget Type.
  Each Widget has exactly one Widget Type.
Widget references Noun.
  Each Widget references at most one Noun.
Widget has Column Count.
  Each Widget has at most one Column Count.

### Entity List (reactive live view)
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
List Item has display- Text.
  Each List Item has at most one display- Text.
List Item has display-sub- Text.
  Each List Item has at most one display-sub- Text.
List Item has Display Status.
  Each List Item has at most one Display Status.
List Item has Display Image Path.
  Each List Item has at most one Display Image Path.

## Constraints

Each Entity List has at most one List Item per Resource.
Each Entity List has at most one Page per Page Number.
No View Renderer is for the same View on the same Platform more than once.

## Derivation Rules

List Item has display- Text if List Item displays Resource and that Resource has Reference and display- Text is that Reference.
List Item has display- Text if List Item displays Resource and that Resource has Value and that Resource has no Reference and display- Text is that Value.
List Item has display-sub- Text if List Item displays Resource and that Resource has Reference and that Resource has Value and display-sub- Text is that Value.
List Item has Display Status if List Item displays Resource and some State Machine is for that Resource and that State Machine is currently in some Status and Display Status is that Status.

## Deontic Constraints

It is obligatory that each Entity List has some Page Size.
It is obligatory that each Entity List has some Sort Field.
It is obligatory that each Entity List has some Sort Direction.

## Instance Facts

Domain 'ui' has Access 'public'.
