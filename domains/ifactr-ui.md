# iFactr UI

## Entity Types

| Entity | Reference Scheme | Notes |
|--------|-----------------|-------|
| Layer | LayerName | Base screen/view. FormLayer and NavigationLayer are subtypes |
| FormLayer | LayerName | Layer subtype for user input |
| Fieldset | FieldsetName | Grouped form fields with header/footer |
| Field | FieldId | Base input control. Subtypes: TextField, BoolField, DateField, etc. |
| TextField | FieldId | Single-line text input |
| NumericField | FieldId | Numeric-only text input |
| MultiLineTextField | FieldId | Multi-line text input |
| EmailField | FieldId | Email text input |
| BoolField | FieldId | Boolean switch |
| DateField | FieldId | Date/time picker |
| SliderField | FieldId | Range slider |
| SelectListField | FieldId | Dropdown/picker |
| SelectListFieldItem | ItemKey | Option within SelectListField |
| LabelField | FieldId | Read-only display |
| ButtonField | FieldId | Navigation button as form field |
| NavigationField | FieldId | Navigation link as form field |
| ImagePickerField | FieldId | Image selection |
| DrawingField | FieldId | Drawing canvas |
| LayerItem | ItemName | Base UI element on a Layer |
| ItemList | ItemName | Scrollable list of navigation items |
| Menu | ItemName | Menu of navigation items |
| Item | ItemText | Navigation element within ItemList or Menu |
| Block | BlockName | Rich text/HTML content container |
| Panel | PanelName | Rich text/HTML content container with header |
| PanelItem | PanelItemId | Element within Block or Panel |
| Link | LinkAddress | Navigation/action control |
| Button | ButtonId | Clickable action control. Extends Link |
| SubmitButton | ButtonId | Form submission button |
| CancelButton | ButtonId | Form cancel button |
| Icon | IconSource | Image reference |

## Value Types

| Value | Type | Constraints |
|-------|------|------------|
| LayerName | string | |
| LayerTitle | string | |
| FieldId | string | |
| FieldLabel | string | |
| FieldPlaceholder | string | |
| FieldText | string | |
| FieldsetName | string | |
| FieldsetHeader | string | |
| FieldsetFooter | string | |
| ItemName | string | |
| ItemText | string | |
| ItemSubtext | string | |
| BlockName | string | |
| PanelName | string | |
| PanelItemId | string | |
| ButtonId | string | |
| ButtonText | string | |
| LinkAddress | string | format: uri |
| LinkText | string | |
| ConfirmationText | string | |
| IconSource | string | |
| ItemKey | string | |
| ItemValue | string | |
| HtmlContent | string | |
| Expression | string | |
| LoadIndicatorTitle | string | |
| LoadIndicatorDelay | number | minimum: 0 |
| MinValue | number | |
| MaxValue | number | |
| StepSize | number | |
| SliderValue | number | |
| RowCount | integer | minimum: 1 |
| MinuteInterval | integer | minimum: 1 |
| SelectedIndex | integer | minimum: 0 |
| IsScrollable | boolean | |
| IsPassword | boolean | |
| IsFullSize | boolean | |
| IsFocused | boolean | |
| BoolValue | boolean | |
| LayerLayout | string | enum: Rounded, EdgeToEdge |
| CompositeLayout | string | enum: OneColumn, TwoColumns |
| ActionType | string | enum: Undefined, Add, Cancel, Edit, Delete, More, Submit, None |
| KeyboardType | string | enum: AlphaNumeric, PIN, Symbolic, Email |
| TextCompletion | string | enum: Disabled, OfferSuggestions, AutoCapitalize |
| DateType | string | enum: Date, Time, DateTime |
| FieldsetLayout | string | enum: List, Simple |
| RequestType | string | enum: Async, ClearPaneHistory, NewWindow, Media |
| PopoverPresentationStyle | string | enum: Normal, FullScreen |
| DisplayStyle | string | enum: Simple, SimpleWrap, SubtextBelow, SubtextBeside, Content, HeaderContent, HeaderWrapContent, Store |
| ButtonPosition | string | enum: NotSpecified, TopLeft, TopRight, InLine |

## Readings

| Reading | Multiplicity |
|---------|-------------|
| Layer has LayerTitle | \*:1 |
| Layer has LayerLayout | \*:1 |
| Layer has IsScrollable | \*:1 |
| Layer has PopoverPresentationStyle | \*:1 |
| Layer has CompositeLayout | \*:1 |
| Layer has BackButton | \*:1 |
| Layer has DetailLink | \*:1 |
| Layer has CompositeLayerLink | \*:1 |
| Layer has CompositeActionButton | \*:1 |
| Layer has CompositeParent | \*:1 |
| Layer contains LayerItem | 1:\* |
| Layer contains Button as ActionButton | 1:\* |
| FormLayer is a subtype of Layer | |
| FormLayer contains Fieldset | 1:\* |
| FormLayer has SubmitButton as ActionButton | \*:1 |
| Fieldset has FieldsetHeader | \*:1 |
| Fieldset has FieldsetFooter | \*:1 |
| Fieldset has FieldsetLayout | \*:1 |
| Fieldset contains Field | 1:\* |
| Field has FieldLabel | \*:1 |
| Field has FieldText | \*:1 |
| Field has FieldPlaceholder | \*:1 |
| Field has IsFocused | \*:1 |
| TextField is a subtype of Field | |
| TextField has Expression | \*:1 |
| TextField has IsPassword | \*:1 |
| TextField has KeyboardType | \*:1 |
| TextField has TextCompletion | \*:1 |
| NumericField is a subtype of TextField | |
| MultiLineTextField is a subtype of TextField | |
| MultiLineTextField has RowCount | \*:1 |
| EmailField is a subtype of TextField | |
| BoolField is a subtype of Field | |
| BoolField has BoolValue | \*:1 |
| DateField is a subtype of Field | |
| DateField has DateType | \*:1 |
| DateField has MinuteInterval | \*:1 |
| SliderField is a subtype of Field | |
| SliderField has SliderValue | \*:1 |
| SliderField has MinValue | \*:1 |
| SliderField has MaxValue | \*:1 |
| SliderField has StepSize | \*:1 |
| SelectListField is a subtype of Field | |
| SelectListField has SelectedIndex | \*:1 |
| SelectListField contains SelectListFieldItem | 1:\* |
| SelectListFieldItem has ItemValue | \*:1 |
| LabelField is a subtype of Field | |
| ButtonField is a subtype of Field | |
| ButtonField has Button as Link | \*:1 |
| ButtonField has ConfirmationText | \*:1 |
| ButtonField has RequestType | \*:1 |
| ButtonField has ActionType | \*:1 |
| NavigationField is a subtype of Field | |
| NavigationField has Link | \*:1 |
| NavigationField has ConfirmationText | \*:1 |
| NavigationField has RequestType | \*:1 |
| NavigationField has ActionType | \*:1 |
| ImagePickerField is a subtype of Field | |
| DrawingField is a subtype of Field | |
| LayerItem has FieldsetHeader as Header | \*:1 |
| LayerItem has FieldsetFooter as Footer | \*:1 |
| ItemList is a subtype of LayerItem | |
| ItemList has DisplayStyle | \*:1 |
| ItemList contains Item | 1:\* |
| Menu is a subtype of LayerItem | |
| Menu contains Item | 1:\* |
| Item has ItemSubtext | \*:1 |
| Item has Icon | \*:1 |
| Item has Link | \*:1 |
| Item has Button | \*:1 |
| Block is a subtype of LayerItem | |
| Block has HtmlContent | \*:1 |
| Block has IsFullSize | \*:1 |
| Block contains PanelItem | 1:\* |
| Panel is a subtype of LayerItem | |
| Panel has HtmlContent | \*:1 |
| Panel has IsFullSize | \*:1 |
| Panel contains PanelItem | 1:\* |
| Link has LinkText | \*:1 |
| Link has ActionType | \*:1 |
| Link has ConfirmationText | \*:1 |
| Link has Icon | \*:1 |
| Link has LoadIndicatorDelay | \*:1 |
| Link has LoadIndicatorTitle | \*:1 |
| Link has RequestType | \*:1 |
| Button is a subtype of Link | |
| Button has ButtonText | \*:1 |
| Button has ButtonPosition | \*:1 |
| SubmitButton is a subtype of Button | |
| CancelButton is a subtype of Button | |
| Icon has IconSource | \*:1 |
