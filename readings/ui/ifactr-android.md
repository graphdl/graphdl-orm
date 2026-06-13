# AREST UI: iFactr.Android / MonoCross + Material Design Layout Grammar

This reading transcribes the canonical iFactr.Android / MonoCross navigation
model and Android Material Design (v1/Holo-to-Material) layout values as FORML
2 fact types and instance facts. It is additive over `readings/ui/ui.md` (the
platform-agnostic view hierarchy), `readings/ui/monoview.md` (the per-app
render surface), and `readings/ui/design.md` (the modern-minimal token layer).

The kernel UI currently hard-codes scattered card geometry. This reading gives
the layout grammar "facts all the way down": the per-component dp values, the
Material type scale (Roboto), the elevation/shadow ladder, the touch-target and
spacing grid, and the iFactr view-type to Android-widget mappings are all
instance facts a renderer can read. No magic numbers in Rust/Slint; the
renderer queries these cells.

Source ground-truth:
  ZebraDevs/iFactr-Android (github.com) — dimensions.xml, styles.xml,
    layout/*.axml, DroidFactory.cs, AndroidDefaults.cs, FragmentHistoryStack.cs
  m1.material.io/layout/metrics-keylines — spacing keylines + component dp
  m1.material.io/style/typography — Roboto type scale sp values + weights
  m1.material.io/material-design/elevation-shadows — elevation resting levels
  m2.material.io/design/layout/spacing-methods — 8dp baseline grid

## Entity Types

Android View Type(.Name) is an entity type.

Android Widget(.Name) is an entity type.

Android Navigation Pane(.Name) is an entity type.

Material Type Style(.Name) is an entity type.

Material Spacing Token(.Name) is an entity type.

Material Elevation Level(.Name) is an entity type.

Material Touch Target(.Name) is an entity type.

Material List Item Size(.Name) is an entity type.

## Value Types

Dp is a value type.

Sp is a value type.

Roboto Weight is a value type.
  The possible values of Roboto Weight are
    'Thin', 'Light', 'Regular', 'Medium', 'Bold', 'Black'.

Roboto Weight Numeric is a value type.

Letter Spacing Em is a value type.

Android Pane Slot is a value type.
  The possible values of Android Pane Slot are
    'master', 'detail', 'popover'.

Form Factor is a value type.
  The possible values of Form Factor are
    'phone-portrait', 'phone-landscape', 'tablet', 'tablet-landscape', 'fullscreen'.

Pane Weight is a value type.

Corner Radius Dp is a value type.

## Fact Types

### Android View Type to Widget binding

Android View Type maps to Android Widget.
  Each Android View Type maps to exactly one Android Widget.

Android View Type has Description.
  Each Android View Type has at most one Description.

Android View Type is hosted in Android Pane Slot.

### Material Type Style

Material Type Style has Sp as font size.
  Each Material Type Style has exactly one Sp as font size.

Material Type Style has Roboto Weight.
  Each Material Type Style has exactly one Roboto Weight.

Material Type Style has Roboto Weight Numeric.
  Each Material Type Style has at most one Roboto Weight Numeric.

Material Type Style has Letter Spacing Em.
  Each Material Type Style has at most one Letter Spacing Em.

Material Type Style has Description.
  Each Material Type Style has at most one Description.

### Material Spacing Token

Material Spacing Token has Dp.
  Each Material Spacing Token has exactly one Dp.

Material Spacing Token has Description.
  Each Material Spacing Token has at most one Description.

### Material Elevation Level

Material Elevation Level has Dp as resting elevation.
  Each Material Elevation Level has exactly one Dp as resting elevation.

Material Elevation Level has Dp as pressed elevation.
  Each Material Elevation Level has at most one Dp as pressed elevation.

Material Elevation Level has Corner Radius Dp.
  Each Material Elevation Level has at most one Corner Radius Dp.

Material Elevation Level has Description.
  Each Material Elevation Level has at most one Description.

### Material Touch Target

Material Touch Target has Dp as minimum width.
  Each Material Touch Target has exactly one Dp as minimum width.

Material Touch Target has Dp as minimum height.
  Each Material Touch Target has exactly one Dp as minimum height.

Material Touch Target has Dp as minimum spacing.
  Each Material Touch Target has at most one Dp as minimum spacing.

### Material List Item Size

Material List Item Size has Dp as row height.
  Each Material List Item Size has exactly one Dp as row height.

Material List Item Size has Description.
  Each Material List Item Size has at most one Description.

### Android Navigation Pane

Android Navigation Pane has Pane Weight in Form Factor.

Android Navigation Pane occupies Android Pane Slot.
  Each Android Navigation Pane occupies exactly one Android Pane Slot.

## Constraints

Each Material Type Style has at most one Sp as font size.
Each Material Type Style has at most one Roboto Weight.
Each Material Touch Target name maps to exactly one Dp as minimum width.
Each Dp value used as resting elevation by a Material Elevation Level is a non-negative integer.
Each Dp value used as row height by a Material List Item Size is a positive multiple of 4.
Each Sp value used as font size by a Material Type Style is a positive integer.

## Deontic Constraints

It is obligatory that each Android View Type maps to some Android Widget.
It is obligatory that each Material Type Style has some Sp as font size.
It is obligatory that each Material Type Style has some Roboto Weight.
It is obligatory that each Material Spacing Token has some Dp.
It is obligatory that each Material Elevation Level has some Dp as resting elevation.
It is obligatory that each Material Touch Target has some Dp as minimum width.
It is obligatory that each Material Touch Target has some Dp as minimum height.
It is obligatory that each Material List Item Size has some Dp as row height.

## Instance Facts

Domain 'ui' has Description 'iFactr.Android + Material Design layout grammar — view-type to widget mappings, Roboto type scale, spacing grid, elevation ladder, touch targets, list-item heights, and pane weights as instance facts so no renderer hard-codes geometry.'.

### Android View Type to Widget mappings
### Source: DroidFactory.cs OnSetDefinitions() Register<T>(typeof(Impl))

Android View Type 'IListView' maps to Android Widget 'ListView'.
Android View Type 'IListView' is hosted in Android Pane Slot 'master'.
Android View Type 'IListView' has Description 'Master list. Wraps android.widget.ListView inside ListViewFragment (a Fragment). Each row is an iFactr ICell rendered by a platform adapter.'.

Android View Type 'IGridView' maps to Android Widget 'GridBase'.
Android View Type 'IGridView' is hosted in Android Pane Slot 'master'.
Android View Type 'IGridView' has Description 'Free-form grid of controls. Wraps a custom GridBase (extends ViewGroup) inside GridFragment, backed by ScrollView + HorizontalScrollView.'.

Android View Type 'ICanvasView' maps to Android Widget 'CanvasFragment'.
Android View Type 'ICanvasView' is hosted in Android Pane Slot 'master'.
Android View Type 'ICanvasView' has Description 'Custom-draw canvas surface. CanvasFragment extends BaseFragment.'.

Android View Type 'IBrowserView' maps to Android Widget 'BrowserFragment'.
Android View Type 'IBrowserView' is hosted in Android Pane Slot 'master'.
Android View Type 'IBrowserView' has Description 'Web content view. BrowserFragment wraps an Android WebView inside a Fragment.'.

Android View Type 'ITabView' maps to Android Widget 'ActionBarAdapter'.
Android View Type 'ITabView' is hosted in Android Pane Slot 'master'.
Android View Type 'ITabView' has Description 'Tab strip rendered as ActionBar tabs (ActionBarAdapter) or as a sliding PagerTabStrip (ActionBarTabView). Each tab drives a separate FragmentHistoryStack on Pane.Master with an incremented ActiveTab index.'.

Android View Type 'IMenu' maps to Android Widget 'Menu'.
Android View Type 'IMenu' is hosted in Android Pane Slot 'master'.
Android View Type 'IMenu' has Description 'Options/overflow menu rendered via the Android ActionBar. IMenuButton items appear as action items or in the overflow drawer.'.

Android View Type 'IToolbar' maps to Android Widget 'Toolbar'.
Android View Type 'IToolbar' is hosted in Android Pane Slot 'master'.
Android View Type 'IToolbar' has Description 'Bottom toolbar strip hosting IToolbarButton and IToolbarSeparator items. Distinct from the ActionBar at the top.'.

Android View Type 'IGridCell' maps to Android Widget 'GridCell'.
Android View Type 'IGridCell' is hosted in Android Pane Slot 'master'.
Android View Type 'IGridCell' has Description 'A cell inside an IGridView containing a layout of controls.'.

Android View Type 'IRichContentCell' maps to Android Widget 'RichText'.
Android View Type 'IRichContentCell' is hosted in Android Pane Slot 'master'.
Android View Type 'IRichContentCell' has Description 'A cell containing rich text (HTML-like spans via Android Spannable).'.

Android View Type 'PopoverActivity' maps to Android Widget 'PopoverActivity'.
Android View Type 'PopoverActivity' is hosted in Android Pane Slot 'popover'.
Android View Type 'PopoverActivity' has Description 'On phones the detail/popover pane is hosted in a separate PopoverActivity. On tablets the detail FrameLayout is in the same iFactrActivity as master.'.

### Android Navigation Pane model
### Source: iFactrActivity.cs, FragmentHistoryStack.cs — Pane.Master / Pane.Detail / Pane.Popover

Android Navigation Pane 'Master' occupies Android Pane Slot 'master'.
Android Navigation Pane 'Detail' occupies Android Pane Slot 'detail'.
Android Navigation Pane 'Popover' occupies Android Pane Slot 'popover'.

### Pane layout_weight values per Form Factor
### Source: iFactr.Droid/Resources/layout*/main.axml
### phone-portrait  : single FrameLayout (master_fragment fills parent; no detail)
### phone-landscape : master weight 1, detail weight 2  (layout-land/main.axml)
### tablet          : master weight 2, detail weight 3  (layout-large/main.axml)
### tablet-landscape: master weight 1, detail weight 2  (layout-large-land/main.axml)

Android Navigation Pane 'Master' has Pane Weight 1 in Form Factor 'phone-landscape'.
Android Navigation Pane 'Detail' has Pane Weight 2 in Form Factor 'phone-landscape'.

Android Navigation Pane 'Master' has Pane Weight 2 in Form Factor 'tablet'.
Android Navigation Pane 'Detail' has Pane Weight 3 in Form Factor 'tablet'.

Android Navigation Pane 'Master' has Pane Weight 1 in Form Factor 'tablet-landscape'.
Android Navigation Pane 'Detail' has Pane Weight 2 in Form Factor 'tablet-landscape'.

### iFactr Android platform default dimensions
### Source: iFactr.Droid/Resources/values/dimensions.xml + AndroidDefaults.cs

Material Spacing Token 'ifactr-cell-height' has Dp 48.
Material Spacing Token 'ifactr-cell-height' has Description 'CellHeight — default row height for an IListView cell. Matches Material single-line list item 48dp.'.

Material Spacing Token 'ifactr-left-margin' has Dp 16.
Material Spacing Token 'ifactr-left-margin' has Description 'LeftMargin — left horizontal inset for list cell content. Matches Material screen-edge margin 16dp.'.

Material Spacing Token 'ifactr-right-margin' has Dp 16.
Material Spacing Token 'ifactr-right-margin' has Description 'RightMargin — right horizontal inset for list cell content.'.

Material Spacing Token 'ifactr-top-margin' has Dp 10.
Material Spacing Token 'ifactr-top-margin' has Description 'TopMargin — top inset within a list cell.'.

Material Spacing Token 'ifactr-bottom-margin' has Dp 10.
Material Spacing Token 'ifactr-bottom-margin' has Description 'BottomMargin — bottom inset within a list cell.'.

Material Spacing Token 'ifactr-small-h-spacing' has Dp 7.
Material Spacing Token 'ifactr-small-h-spacing' has Description 'SmallHorizontalSpacing — spacing between sibling controls in a cell.'.

Material Spacing Token 'ifactr-large-h-spacing' has Dp 10.
Material Spacing Token 'ifactr-large-h-spacing' has Description 'LargeHorizontalSpacing — spacing between groups of controls in a cell.'.

Material Spacing Token 'ifactr-small-v-spacing' has Dp 7.
Material Spacing Token 'ifactr-small-v-spacing' has Description 'SmallVerticalSpacing — vertical spacing between rows of controls.'.

Material Spacing Token 'ifactr-large-v-spacing' has Dp 10.
Material Spacing Token 'ifactr-large-v-spacing' has Description 'LargeVerticalSpacing — vertical spacing between groups of rows.'.

### Material Design 1 Android spacing grid
### Source: m1.material.io/layout/metrics-keylines + m2.material.io/design/layout/spacing-methods

Material Spacing Token 'baseline-grid' has Dp 8.
Material Spacing Token 'baseline-grid' has Description 'All components align to the 8dp square baseline grid for mobile, tablet, desktop.'.

Material Spacing Token 'icon-grid' has Dp 4.
Material Spacing Token 'icon-grid' has Description 'Iconography and fine-grained typography align to the 4dp sub-grid.'.

Material Spacing Token 'screen-margin-phone' has Dp 16.
Material Spacing Token 'screen-margin-phone' has Description 'Left/right screen-edge margin on phone. Content keyline at 72dp from left edge.'.

Material Spacing Token 'screen-margin-tablet' has Dp 24.
Material Spacing Token 'screen-margin-tablet' has Description 'Left/right screen-edge margin on tablet/desktop. Content keyline at 80dp.'.

Material Spacing Token 'content-keyline-phone' has Dp 72.
Material Spacing Token 'content-keyline-phone' has Description 'Left edge of text content when paired with a 40dp icon or avatar on phone (16 + 40 + 16 = 72dp).'.

Material Spacing Token 'content-keyline-tablet' has Dp 80.
Material Spacing Token 'content-keyline-tablet' has Description 'Left edge of text content when paired with a 40dp icon or avatar on tablet (24 + 40 + 16 = 80dp).'.

Material Spacing Token 'status-bar' has Dp 24.
Material Spacing Token 'status-bar' has Description 'Android status bar height.'.

Material Spacing Token 'app-bar-portrait' has Dp 56.
Material Spacing Token 'app-bar-portrait' has Description 'ActionBar / Toolbar height on phone portrait. iFactr popover.axml uses 48dp header; standard Material is 56dp.'.

Material Spacing Token 'app-bar-landscape' has Dp 48.
Material Spacing Token 'app-bar-landscape' has Description 'ActionBar / Toolbar height on phone landscape. iFactr popover.axml root height is 48dp — the landscape app-bar value.'.

Material Spacing Token 'app-bar-tablet' has Dp 64.
Material Spacing Token 'app-bar-tablet' has Description 'ActionBar / Toolbar height on tablet/desktop.'.

Material Spacing Token 'pane-divider' has Dp 1.
Material Spacing Token 'pane-divider' has Description '1dp divider View between master and detail panes in iFactr side-by-side layouts.'.

### Material Design 1 list-item row heights
### Source: m1.material.io/layout/metrics-keylines component heights table

Material List Item Size 'single-line' has Dp 48 as row height.
Material List Item Size 'single-line' has Description 'Single-line list item, no icon. Matches iFactr CellHeight 48dp.'.

Material List Item Size 'single-line-icon' has Dp 48 as row height.
Material List Item Size 'single-line-icon' has Description 'Single-line list item with an icon; same 48dp row height.'.

Material List Item Size 'two-line' has Dp 64 as row height.
Material List Item Size 'two-line' has Description 'Two-line list item (primary + secondary text), no avatar.'.

Material List Item Size 'two-line-avatar' has Dp 72 as row height.
Material List Item Size 'two-line-avatar' has Description 'Two-line list item with 40dp avatar or image tile. Most common Material list row.'.

Material List Item Size 'three-line' has Dp 88 as row height.
Material List Item Size 'three-line' has Description 'Three-line list item (primary + two secondary/body lines).'.

### Touch target
### Source: m1.material.io/layout/metrics-keylines

Material Touch Target 'standard' has Dp 48 as minimum width.
Material Touch Target 'standard' has Dp 48 as minimum height.
Material Touch Target 'standard' has Dp 8 as minimum spacing.
Material Touch Target 'standard' has Description 'Android minimum interactive touch target: 48dp x 48dp (approx 9mm at 160dpi baseline) with 8dp or more between adjacent targets. iFactr CellHeight 48dp satisfies this.'.

### Material Design 1 component elevation resting levels
### Source: m1.material.io/material-design/elevation-shadows

Material Elevation Level 'flat' has Dp 0 as resting elevation.
Material Elevation Level 'flat' has Description 'Flat/text button and non-interactive surfaces. No shadow.'.

Material Elevation Level 'switch-thumb' has Dp 1 as resting elevation.
Material Elevation Level 'switch-thumb' has Description 'Switch thumb resting elevation.'.

Material Elevation Level 'card' has Dp 2 as resting elevation.
Material Elevation Level 'card' has Dp 8 as pressed elevation.
Material Elevation Level 'card' has Corner Radius Dp 2.
Material Elevation Level 'card' has Description 'Card at rest 2dp with 2dp rounded corners; raised to 8dp when dragged. The kernel scattered-card layout should source elevation and corner-radius from this entry.'.

Material Elevation Level 'raised-button' has Dp 2 as resting elevation.
Material Elevation Level 'raised-button' has Dp 8 as pressed elevation.
Material Elevation Level 'raised-button' has Description 'Contained/raised button resting at 2dp, pressed to 8dp.'.

Material Elevation Level 'refresh-indicator' has Dp 3 as resting elevation.
Material Elevation Level 'refresh-indicator' has Description 'SwipeRefreshLayout progress circle.'.

Material Elevation Level 'app-bar' has Dp 4 as resting elevation.
Material Elevation Level 'app-bar' has Description 'ActionBar / Toolbar. iFactr ActionBar/Toolbar sits at this elevation.'.

Material Elevation Level 'floating-action-button' has Dp 6 as resting elevation.
Material Elevation Level 'floating-action-button' has Dp 12 as pressed elevation.
Material Elevation Level 'floating-action-button' has Description 'FAB resting at 6dp, raised to 12dp when pressed.'.

Material Elevation Level 'snackbar' has Dp 6 as resting elevation.
Material Elevation Level 'snackbar' has Description 'Snackbar / toast-replacement surface.'.

Material Elevation Level 'menu' has Dp 8 as resting elevation.
Material Elevation Level 'menu' has Description 'Dropdown, context, and popup menus.'.

Material Elevation Level 'navigation-drawer' has Dp 16 as resting elevation.
Material Elevation Level 'navigation-drawer' has Description 'Side navigation drawer (DrawerLayout) and modal bottom sheet.'.

Material Elevation Level 'modal-bottom-sheet' has Dp 16 as resting elevation.
Material Elevation Level 'modal-bottom-sheet' has Description 'Modal bottom sheet; same resting elevation as navigation drawer.'.

Material Elevation Level 'dialog' has Dp 24 as resting elevation.
Material Elevation Level 'dialog' has Description 'Modal dialog. Highest in-app surface elevation.'.

### Material Design 1 / Android Roboto type scale
### Source: m1.material.io/style/typography — 13-style MD1 scale
### sp sizes are the device defaults (desktop is -1sp each)
### Roboto weights: Thin=100 Light=300 Regular=400 Medium=500 Bold=700 Black=900
### Letter-spacing in em: official MD1 spec device values

Material Type Style 'Display4' has Sp 112 as font size.
Material Type Style 'Display4' has Roboto Weight 'Light'.
Material Type Style 'Display4' has Roboto Weight Numeric 300.
Material Type Style 'Display4' has Letter Spacing Em -0.04.
Material Type Style 'Display4' has Description 'Largest display style. 112sp / Light / -0.04em. Hero numerals and giant headings.'.

Material Type Style 'Display3' has Sp 56 as font size.
Material Type Style 'Display3' has Roboto Weight 'Regular'.
Material Type Style 'Display3' has Roboto Weight Numeric 400.
Material Type Style 'Display3' has Letter Spacing Em -0.02.
Material Type Style 'Display3' has Description '56sp / Regular / -0.02em.'.

Material Type Style 'Display2' has Sp 45 as font size.
Material Type Style 'Display2' has Roboto Weight 'Regular'.
Material Type Style 'Display2' has Roboto Weight Numeric 400.
Material Type Style 'Display2' has Letter Spacing Em 0.0.
Material Type Style 'Display2' has Description '45sp / Regular / 0em.'.

Material Type Style 'Display1' has Sp 34 as font size.
Material Type Style 'Display1' has Roboto Weight 'Regular'.
Material Type Style 'Display1' has Roboto Weight Numeric 400.
Material Type Style 'Display1' has Letter Spacing Em 0.0.
Material Type Style 'Display1' has Description '34sp / Regular / 0em. Base of the display cluster; large in-page headings.'.

Material Type Style 'Headline' has Sp 24 as font size.
Material Type Style 'Headline' has Roboto Weight 'Regular'.
Material Type Style 'Headline' has Roboto Weight Numeric 400.
Material Type Style 'Headline' has Letter Spacing Em 0.0.
Material Type Style 'Headline' has Description '24sp / Regular. Primary section heading.'.

Material Type Style 'Title' has Sp 20 as font size.
Material Type Style 'Title' has Roboto Weight 'Medium'.
Material Type Style 'Title' has Roboto Weight Numeric 500.
Material Type Style 'Title' has Letter Spacing Em 0.005.
Material Type Style 'Title' has Description '20sp / Medium / +0.005em. ActionBar/Toolbar title. iFactr popover.axml Layer Title TextView uses textAppearanceLarge (approx 20sp).'.

Material Type Style 'Subhead' has Sp 16 as font size.
Material Type Style 'Subhead' has Roboto Weight 'Regular'.
Material Type Style 'Subhead' has Roboto Weight Numeric 400.
Material Type Style 'Subhead' has Letter Spacing Em 0.01.
Material Type Style 'Subhead' has Description '16sp / Regular / +0.01em. List item primary text. iFactr MediumFont is 18sp; Subhead at 16sp is the standard Material list primary text size.'.

Material Type Style 'Body2' has Sp 14 as font size.
Material Type Style 'Body2' has Roboto Weight 'Medium'.
Material Type Style 'Body2' has Roboto Weight Numeric 500.
Material Type Style 'Body2' has Letter Spacing Em 0.01.
Material Type Style 'Body2' has Description '14sp / Medium / +0.01em. Emphasized body; secondary content in a two-line list row.'.

Material Type Style 'Body1' has Sp 14 as font size.
Material Type Style 'Body1' has Roboto Weight 'Regular'.
Material Type Style 'Body1' has Roboto Weight Numeric 400.
Material Type Style 'Body1' has Letter Spacing Em 0.01.
Material Type Style 'Body1' has Description '14sp / Regular / +0.01em. Default body copy. iFactr AndroidDefaults NormalFont = Roboto 14sp.'.

Material Type Style 'Caption' has Sp 12 as font size.
Material Type Style 'Caption' has Roboto Weight 'Regular'.
Material Type Style 'Caption' has Roboto Weight Numeric 400.
Material Type Style 'Caption' has Letter Spacing Em 0.04.
Material Type Style 'Caption' has Description '12sp / Regular / +0.04em. Secondary annotations, timestamps, sub-labels.'.

Material Type Style 'Button' has Sp 14 as font size.
Material Type Style 'Button' has Roboto Weight 'Medium'.
Material Type Style 'Button' has Roboto Weight Numeric 500.
Material Type Style 'Button' has Letter Spacing Em 0.08.
Material Type Style 'Button' has Description '14sp / Medium / +0.08em / ALL CAPS. iFactr popover.axml action button uses textSize 14sp + textAllCaps=true.'.

Material Type Style 'Overline' has Sp 12 as font size.
Material Type Style 'Overline' has Roboto Weight 'Regular'.
Material Type Style 'Overline' has Roboto Weight Numeric 400.
Material Type Style 'Overline' has Letter Spacing Em 0.17.
Material Type Style 'Overline' has Description '12sp / Regular / +0.17em / ALL CAPS. Section labels and category overlines.'.

Material Type Style 'Menu' has Sp 14 as font size.
Material Type Style 'Menu' has Roboto Weight 'Regular'.
Material Type Style 'Menu' has Roboto Weight Numeric 400.
Material Type Style 'Menu' has Letter Spacing Em 0.01.
Material Type Style 'Menu' has Description '14sp / Regular. ActionBar overflow and context menu items.'.
