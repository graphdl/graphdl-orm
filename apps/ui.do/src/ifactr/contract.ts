/**
 * iFactr abstract-UI contract — TypeScript spec for ui.do ("iFactr.React").
 *
 * This file is a *faithful extraction* of the iFactr abstract-UI `IInterface`
 * contracts and the on-the-wire serialized-object shapes that a renderer
 * consumes. It is a spec only — no runtime logic. ui.do (a HATEOAS thin client)
 * targets these shapes so a serialized iFactr object deserializes into them.
 *
 * Source reference (read-only): `iFactr-Android/iFactr.UI/iFactr.UI`.
 * Per-interface doc-comments cite the originating `.cs` file.
 *
 * --- The two iFactr UI layers -------------------------------------------------
 *
 * iFactr ships two parallel UI models:
 *
 *  1. The MODERN MonoView model (`IView`/`IListView`/`IGridView`/`ICell`/
 *     `Section`/`IMenu`). These are *native, runtime* contracts: cells are
 *     produced lazily through delegates (`CellRequested`, `ItemIdRequested`)
 *     rather than being serialized. They define field names/semantics but are
 *     NOT themselves a wire format.
 *
 *  2. The LEGACY iFactr.Core abstract-UI model (`iLayer`/`iLayerItem`/`iList`/
 *     `iMenu`/`iItem`/`iBlock`/`iPanel` + `Link`/`Button`/`Icon`/`Label`).
 *     These ARE the serialized objects — they carry `[XmlType]`/`[XmlInclude]`
 *     attributes and implement `IXmlSerializable`. This is what crosses the wire
 *     from an iFactr server/controller to a binding (renderer).
 *
 * --- Serialization shape (see contract.md for detail) -------------------------
 *
 * iFactr serializes with `System.Xml.XmlSerializer`, NOT DataContract/JSON.
 * Polymorphic collections (`ItemsCollection<T>`, `iList`, `iMenu`, `iBlock`)
 * implement `IXmlSerializable` and write each element under an element name
 * equal to the .NET type's `FullName` (e.g. `iFactr.Core.Layers.SubtextItem`),
 * or the assembly-qualified name with `, ` replaced by `___` for cross-assembly
 * types. There is therefore an explicit **type discriminator carried by the
 * element/type name** — mirrored here as the `$type` discriminant field on each
 * serialized node so a JSON/HATEOAS projection of the same object graph stays
 * faithful to the original XML type-tagging.
 *
 * Field names below match the C# property names exactly (PascalCase) so a
 * direct (case-preserving) deserialization round-trips.
 */

/* eslint-disable @typescript-eslint/no-empty-object-type */

// ============================================================================
// Shared value types
// ============================================================================

/**
 * `iFactr.UI.Color` — serialized as ARGB components plus a hex string.
 * Source: referenced throughout (e.g. `MonoView/Views/Interfaces/IView.cs`).
 * Renderer: maps to a CSS color (`#RRGGBB` / `rgba(...)`).
 */
export interface Color {
  A?: number;
  R?: number;
  G?: number;
  B?: number;
  /** Hex form, e.g. "#FF0000". iFactr exposes this as `HexCode`. */
  HexCode?: string;
}

/**
 * Serializable string→string map. iFactr uses `SerializableDictionary<string,string>`
 * for `Link.Parameters` and layer `Parameters`. Renderer: query/body params on navigate.
 */
export type SerializableDictionary = Record<string, string>;

/**
 * Discriminator carried by the XML element/type name during serialization.
 * Set to the originating .NET type `FullName` (e.g. "iFactr.Core.Layers.SubtextItem").
 * Present on every polymorphic serialized node.
 */
export interface ITyped {
  /** .NET type FullName used as the XML element/type tag. */
  $type?: string;
}

// ============================================================================
// Enums (extracted verbatim from the C# enums)
// ============================================================================

/**
 * `iFactr.Core.Controls.ActionType` — Source: `Controls/ActionType.cs`
 * (`[XmlType("CoreActionType", Namespace = "Core")]`). The semantic intent of a
 * link/button action. `Button` additionally defines an identical `ButtonActionType`
 * (`Controls/Button.cs`), kept aligned here.
 * Renderer: drives default label/icon/affordance (Add/Edit/Delete/Submit/...).
 */
export enum ActionType {
  Undefined = "Undefined",
  Add = "Add",
  Cancel = "Cancel",
  Edit = "Edit",
  Delete = "Delete",
  More = "More",
  Submit = "Submit",
  None = "None",
}

/**
 * `iFactr.UI.RequestType` — Source: `Link.cs`. How a navigation should be performed.
 * Renderer: async fetch vs. media open vs. history reset vs. new tab/window.
 */
export enum RequestType {
  Async = "Async",
  Media = "Media",
  ClearPaneHistory = "ClearPaneHistory",
  NewWindow = "NewWindow",
}

/**
 * `iFactr.Core.Controls.Link.Rev` — Source: `Controls/Link.cs` (legacy/obsolete;
 * superseded by `RequestType`). Retained for faithful deserialization of older payloads.
 */
export enum LinkRev {
  Async = "Async",
  Media = "Media",
  None = "None",
}

/**
 * `iFactr.Core.Controls.Button.Position` — Source: `Controls/Button.cs`
 * (obsolete/"no longer honored", retained for fidelity).
 */
export enum ButtonPosition {
  NotSpecified = "NotSpecified",
  TopLeft = "TopLeft",
  TopRight = "TopRight",
  InLine = "InLine",
}

/**
 * `iFactr.UI.ListViewStyle` — Source: `MonoView/Enums/ListViewStyle.cs` (byte enum).
 * Renderer: flat list vs. grouped sections.
 */
export enum ListViewStyle {
  Default = "Default",
  Grouped = "Grouped",
}

/**
 * `iFactr.UI.ColumnMode` — Source: `MonoView/Enums/ColumnMode.cs` (byte enum).
 * Renderer: single vs. two-column list layout.
 */
export enum ColumnMode {
  OneColumn = "OneColumn",
  TwoColumns = "TwoColumns",
}

/**
 * `iFactr.Core.Layers.LayerLayout` — Source: `Views/Enums/LayerLayout.cs`.
 */
export enum LayerLayout {
  Rounded = "Rounded",
  EdgetoEdge = "EdgetoEdge",
}

/**
 * `iFactr.Core.Layers.iList.StyleTypes` — Source: `Views/Items/iList.cs`.
 * Renderer: per-row presentation hint for a serialized `iList`.
 */
export enum ListStyleTypes {
  Simple = "Simple",
  SimpleWrap = "SimpleWrap",
  SubtextBelow = "SubtextBelow",
  SubtextBeside = "SubtextBeside",
  Content = "Content",
  HeaderContent = "HeaderContent",
  HeaderWrapContent = "HeaderWrapContent",
  Store = "Store",
}

// ============================================================================
// MonoCross base view contract
// ============================================================================

/**
 * `MonoCross.Navigation.IMXView` — Source: `MonoCross/Navigation/MXView.cs`.
 * The root marker for "this object is a View". Carries a model and a model type.
 * On the wire the model is the layer/controller payload; `ModelType` is the
 * .NET type name of that model.
 * Renderer: top-level dispatch — pick a view component by view kind, bind `Model`.
 */
export interface IMXView extends ITyped {
  /** .NET type name of the model displayed by this view. */
  ModelType?: string;
  /** The model payload the view renders. */
  Model?: unknown;
}

/**
 * Discriminated kinds of view a renderer may receive.
 * Mirrors the `IView` subtype hierarchy in `MonoView/Views/Interfaces/`.
 */
export type ViewKind =
  | "ListView"
  | "GridView"
  | "TabView"
  | "BrowserView"
  | "CanvasView";

/**
 * `iFactr.UI.IView` — Source: `MonoView/Views/Interfaces/IView.cs`.
 * Base interface for all native views (`IListView`, `IGridView`, `ITabView`,
 * `IBrowserView`, `ICanvasView`). Common chrome: title, header/title colors,
 * size, orientation, background, arbitrary metadata.
 * Renderer: draws the page/screen frame (header bar + title) and hosts content.
 */
export interface IView extends IMXView {
  /** Discriminator for which concrete view component to render. */
  ViewKind?: ViewKind;
  /** Title shown in the header bar. */
  Title?: string;
  /** Header bar color, if any. */
  HeaderColor?: Color;
  /** Color used to draw the title. */
  TitleColor?: Color;
  /** Native current height (read-only at runtime). */
  Height?: number;
  /** Native current width (read-only at runtime). */
  Width?: number;
  /** Orientation preference (`PreferredOrientation`). */
  PreferredOrientations?: string;
  /** Arbitrary string metadata bag (`MetadataCollection`). */
  Metadata?: Record<string, string>;
  /** Background color (set via `SetBackground(Color)`). */
  BackgroundColor?: Color;
  /** Background image path (set via `SetBackground(string, ContentStretch)`). */
  BackgroundImagePath?: string;
}

/**
 * `iFactr.UI.IHistoryEntry` — mixin implemented by `IListView`, `IGridView`,
 * `IBrowserView` (Source: `MonoView/Views/Interfaces/IHistoryEntry.cs`,
 * referenced by those views). Carries back-stack chrome.
 * Renderer: back button + breadcrumb behavior.
 */
export interface IHistoryEntry {
  /** Back button override; `null` uses the platform default. */
  BackLink?: Link | null;
  /** Stack identifier / pane id, when present in the payload. */
  StackID?: string;
}

// ============================================================================
// Controls (serialized: iFactr.Core.Controls.*)
// ============================================================================

/**
 * `iFactr.Core.Controls.IBlockPanelItem` — Source: `Controls/IBlockPanelItem.cs`.
 * Marker for anything insertable into an `iBlock`/`iPanel`. Every concrete panel
 * item exposes an HTML projection via `GetHtml()`; on the wire only its data
 * fields survive (the HTML is regenerated by the renderer).
 */
export interface IBlockPanelItem extends ITyped {}

/**
 * `iFactr.Core.Controls.Icon` — Source: `Controls/Icon.cs` (a `PanelItem`).
 * Renderer: an `<img>` (or icon) sized by Width/Height, aligned by Align.
 */
export interface Icon extends IBlockPanelItem {
  /** Alignment hint. */
  Align?: string;
  /** Display height (string, e.g. "32px"). */
  Height?: string;
  /** Display width (string, e.g. "100%"). */
  Width?: string;
  /** Image URI / file path. */
  Location?: string;
  /** Title/name of the image (rendered as title attr). */
  Name?: string;
}

/**
 * `iFactr.Core.Controls.Label.LabelStyle` placeholder — Source: `Controls/Label.cs`
 * (style lives in `iFactr.Core.Styles.LabelStyle`). Captures the fields the
 * renderer actually consumes for text formatting.
 * Renderer: heading level + bold/italic emphasis + alignment.
 */
export interface LabelStyle {
  /** 0 = body text; 1..6 = h1..h6. */
  HeaderLevel?: number;
  /** "Normal" | "Bold" | "Italic" | "BoldItalic" (LabelStyle.Format). */
  TextFormat?: string;
  /** "Left" | "Center" | "Right" (LabelStyle.Align). */
  TextAlign?: string;
}

/**
 * `iFactr.Core.Controls.Label` — Source: `Controls/Label.cs` (a `PanelItem`).
 * Renderer: a (possibly heading/emphasized) text run.
 */
export interface Label extends IBlockPanelItem {
  Name?: string;
  Text?: string;
  Style?: LabelStyle;
}

/**
 * `iFactr.Core.Controls.Link` — Source: `Controls/Link.cs`
 * (`[XmlType("CoreLink", Namespace="Core")]`, `[XmlInclude]` of Button/SubmitButton/
 * CancelButton — i.e. Link is the serialized base of the button family).
 * A navigable control pointing at a URL `Address` with optional `Parameters`.
 * Renderer: an anchor / pressable that navigates (honoring RequestType + confirmation).
 */
export interface Link extends IBlockPanelItem {
  /** Semantic action intent. */
  Action?: ActionType;
  /** Target URL/URI to navigate to. */
  Address?: string;
  /** Confirmation prompt shown before navigating, if set. */
  ConfirmationText?: string;
  /** Optional image displayed with the link. */
  Image?: Icon;
  /** ms before showing a load indicator; <0 disables. */
  LoadIndicatorDelay?: number;
  /** Title for the load indicator. */
  LoadIndicatorTitle?: string;
  /** Navigation parameters (≈ hidden form fields). */
  Parameters?: SerializableDictionary;
  /** How the request is performed. */
  RequestType?: RequestType;
  /** Display text. */
  Text?: string;
  /** @deprecated legacy alias of RequestType === NewWindow. */
  NewWindow?: boolean;
  /** @deprecated legacy alias of RequestType. */
  RevSetting?: LinkRev;
}

/**
 * `iFactr.Core.Controls.Button` — Source: `Controls/Button.cs` (extends `Link`).
 * Defaults `Action` to `Submit`; when `Text` is null the framework derives it
 * from `Action` (Add→"Add", Edit→"Edit", ...). `Action` here is the
 * `ButtonActionType` overload (same members as `ActionType`).
 * Renderer: a button; its label/affordance defaults from Action when Text is empty.
 */
export interface Button extends Link {
  /** Stable id for the button. */
  ID?: string;
  /** @deprecated "no longer honored". */
  ButtonPosition?: ButtonPosition;
}

/**
 * `iFactr.Core.Controls.SubmitButton` — Source: `Controls/SubmitButton.cs`
 * (extends `Button`; `[Obsolete]`). `Action` is sealed to `Submit`.
 * Renderer: a submit button that gathers + posts the surrounding form.
 */
export interface SubmitButton extends Button {
  Action?: ActionType.Submit;
}

/**
 * `iFactr.Core.Controls.CancelButton` — Source: `Controls/CancelButton.cs`
 * (extends `Button`; included via `Link`'s `[XmlInclude]`). Cancels an operation.
 */
export interface CancelButton extends Button {
  Action?: ActionType.Cancel;
}

// ============================================================================
// Legacy serialized layer model (the on-the-wire object graph)
// ============================================================================

/**
 * `iFactr.Core.Layers.iLayerItem` — Source: `Views/Items/iLayerItem.cs`.
 * Abstract base of everything placed on a layer (lists, menus, blocks, panels,
 * fieldsets). Carries `Header`/`Footer` and custom `IXmlSerializable` read/write.
 */
export interface iLayerItem extends ITyped {
  Header?: string;
  Footer?: string;
}

/**
 * `iFactr.Core.Layers.iItem` — Source: `Views/Items/CollectionItems/iItem.cs`.
 * A navigation row within an `iList`/`iMenu`: primary `Text`, secondary `Subtext`,
 * an `Icon`, a `Link` (selection target) and an optional secondary `Button`.
 * Subtypes (SubtextItem, MultiLineSubtextItem, RightSubtextItem, PhotoItem,
 * ShopItem, ContentItem, MessageItem, ...) live in `CollectionItems/` and are
 * tagged by `$type`. Renderer: a list cell (text/subtext/icon + tap → Link).
 */
export interface iItem extends ITyped {
  Text?: string;
  Subtext?: string;
  Icon?: Icon;
  Link?: Link;
  Button?: Button;
}

/**
 * `iFactr.Core.Layers.iCollection<iItem>` — Source: `Views/Items/iCollection.cs`.
 * Shared base for `iList` and `iMenu`: a named, header/footer-bearing collection
 * of `iItem`s (`Items`). Renderer: a grouped section of rows.
 */
export interface iCollection extends iLayerItem {
  Name?: string;
  Items?: iItem[];
}

/**
 * `iFactr.Core.Layers.iList` — Source: `Views/Items/iList.cs`
 * (`IXmlSerializable`). A serialized list section of `iItem`s with a `DisplayStyle`.
 * Renderer: a list/section; `DisplayStyle` selects the row template.
 */
export interface iList extends iCollection {
  DisplayStyle?: ListStyleTypes;
}

/**
 * `iFactr.Core.Layers.iMenu` (legacy) — Source: `Views/Items/iMenu.cs`
 * (`IXmlSerializable`). A serialized menu of `iItem`s with `ID`/`Style`.
 * NOTE: distinct from the modern `iFactr.UI.IMenu` below.
 * Renderer: an action/overflow menu of rows.
 */
export interface iMenuLegacy extends iCollection {
  ID?: string;
  Style?: string;
}

/**
 * `iFactr.Core.Layers.iBlock` — Source: `Views/Items/iBlock.cs` (`IXmlSerializable`,
 * `IHtmlText`). A rich-text/HTML container of `PanelItem`s (Label/Icon/Link/...).
 * `Text` is the concatenation of raw text + each child's `GetHtml()`.
 * Renderer: a rich content block.
 */
export interface iBlock extends iLayerItem {
  Name?: string;
  /** Raw text/HTML; on read, equals text + children HTML. */
  Text?: string;
  /** Expand to the pane's far edge regardless of content size. */
  FullSize?: boolean;
  /** Inline panel items (Label, Icon, Link, ...). */
  Items?: IBlockPanelItem[];
  HtmlSource?: string;
}

/**
 * `iFactr.Core.Layers.iPanel` — Source: `Views/Items/iPanel.cs` (`IXmlSerializable`,
 * `IHtmlText`). Like `iBlock` with an extra `Style` and (obsolete) `Image`.
 * Renderer: a styled rich content panel.
 */
export interface iPanel extends iLayerItem {
  Text?: string;
  Style?: string;
  FullSize?: boolean;
  Items?: IBlockPanelItem[];
  /** @deprecated use InsertImage. */
  Image?: Icon;
}

/**
 * `iFactr.Core.Layers.iLayer` — Source: `Views/Layers/iLayer.cs` (`IMXController`).
 * The serialized screen/controller: the top-level model carried by a view. Holds
 * `Items` (the iLayerItem graph rendered top→bottom), `ActionButtons`, a
 * `BackButton`, layout flags, detail/composite links, and submission `Parameters`.
 * Renderer: the page model — render header (Title) + ActionButtons, then Items.
 */
export interface iLayer extends ITyped {
  /** Layer name; layer equality is by Name (history identity). */
  Name?: string;
  Title?: string;
  /** Action buttons (popup/menu depending on platform). */
  ActionButtons?: Button[];
  /** The ordered UI element graph to render. */
  Items?: iLayerItem[];
  /** Item to scroll to on present. */
  FocusedItem?: unknown;
  Layout?: LayerLayout;
  IsScrollable?: boolean;
  /** Back button; Action===None hides it; null uses default. */
  BackButton?: Button | null;
  /** Composite/master-detail action button (large form factors). */
  CompositeActionButton?: Button | null;
  /** Next layer link for composite layout. */
  CompositeLayerLink?: Link | null;
  /** Detail-pane auto-navigation link. */
  DetailLink?: Link | null;
  /** Submission/hidden parameters (a.k.a. ActionParameters). */
  Parameters?: SerializableDictionary;
}

// ============================================================================
// Modern MonoView cells & sections (IListView / IGridView content)
// ============================================================================

/**
 * `iFactr.UI.ICell` — Source: `MonoView/Cells and Tiles/Interfaces/ICell.cs`.
 * Base of `IGridCell` and `IRichContentCell`. A renderable list entry with sizing
 * + background + metadata. Concrete cells are produced at runtime via delegates,
 * so a serialized projection tags each by `$type` (ContentCell/GridCell/...).
 * Renderer: one row/tile within a section.
 */
export interface ICell extends ITyped {
  BackgroundColor?: Color;
  MinHeight?: number;
  MaxHeight?: number;
  Metadata?: Record<string, string>;
}

/**
 * `iFactr.UI.ContentCell` — Source: `MonoView/Cells and Tiles/ContentCell.cs`
 * (a `GridCell`). The standard text/subtext/value + image cell.
 * Renderer: avatar/icon + title + subtitle + trailing value, tap → Link/selection.
 */
export interface IContentCell extends ICell {
  /** Primary text label. */
  TextLabel?: string;
  /** Secondary text label (typically below the title). */
  SubtextLabel?: string;
  /** Value label (typically right-aligned). */
  ValueLabel?: string;
  /** Image file path/URI. */
  ImagePath?: string;
  /** Navigation target when the cell is selected. */
  NavigationLink?: Link;
  /** Accessory/affordance hint (e.g. disclosure). */
  AccessoryLink?: Link;
}

/**
 * `iFactr.UI.SectionHeader` / `SectionFooter` — Source: `MonoView/Cells and Tiles/Section.cs`
 * (referenced). Header/footer chrome for a section.
 */
export interface SectionHeaderFooter {
  Text?: string;
  TextColor?: Color;
  BackgroundColor?: Color;
}

/**
 * `iFactr.UI.Section` — Source: `MonoView/Cells and Tiles/Section.cs`.
 * A group of cells with optional `Header`/`Footer`. At runtime `ItemCount` +
 * `CellRequested` produce cells lazily; a serialized projection inlines the
 * realized cells as `Cells`.
 * Renderer: a list section (header + N cells + footer).
 */
export interface Section {
  Header?: SectionHeaderFooter;
  Footer?: SectionHeaderFooter;
  /** Total number of cells in the section. */
  ItemCount?: number;
  /** Realized cells (serialized projection of CellRequested output). */
  Cells?: ICell[];
}

// ============================================================================
// Modern menus & toolbars
// ============================================================================

/**
 * `iFactr.UI.IMenuButton` — Source: `MonoView/Menus and Toolbars/Interfaces/IMenuButton.cs`.
 * A pressable item in an `IMenu`: `Title`, optional `ImagePath`, and a
 * `NavigationLink` used when there is no Clicked handler.
 * Renderer: a menu/toolbar action.
 */
export interface IMenuButton extends ITyped {
  Title?: string;
  ImagePath?: string;
  NavigationLink?: Link;
}

/**
 * `iFactr.UI.IMenu` — Source: `MonoView/Menus and Toolbars/Interfaces/IMenu.cs`.
 * The support-action menu attached to a list/grid/browser view. Carries colors,
 * an activator `Title`/`ImagePath`, and its `Buttons` (`ButtonCount` + `GetButton`).
 * Renderer: the view's overflow / action menu.
 */
export interface IMenu extends ITyped {
  BackgroundColor?: Color;
  ForegroundColor?: Color;
  SelectionColor?: Color;
  /** Title of the button that activates the menu (where applicable). */
  Title?: string;
  /** Image of the menu activator button. */
  ImagePath?: string;
  /** Number of buttons (`ButtonCount`). */
  ButtonCount?: number;
  /** The menu buttons (`GetButton(index)` realized). */
  Buttons?: IMenuButton[];
}

// ============================================================================
// View kinds (the concrete views a renderer dispatches on)
// ============================================================================

/**
 * `iFactr.UI.IListView` — Source: `MonoView/Views/Interfaces/IListView.cs`.
 * The most common view: sections of cells, an optional `Menu` and `SearchBox`,
 * column/separator styling, validation errors, and form submission. Cells are
 * runtime-delegate-produced (`CellRequested`); the serialized projection inlines
 * `Sections[].Cells`.
 * Renderer: a (optionally grouped/multi-column) scrolling list with header chrome.
 */
export interface IListView extends IView, IHistoryEntry {
  ViewKind?: "ListView";
  ColumnMode?: ColumnMode;
  SeparatorColor?: Color;
  Style?: ListViewStyle;
  Menu?: IMenu;
  /** Search box config (`ISearchBox`) when present. */
  SearchBox?: { Placeholder?: string };
  Sections?: Section[];
  /** Control validation errors (`ValidationErrorCollection`). */
  ValidationErrors?: Record<string, string[]>;
}

/**
 * `iFactr.UI.IGridView` — Source: `MonoView/Views/Interfaces/IGridView.cs`.
 * A grid layout surface (`IGridBase`: rows/columns/children) with scrolling
 * toggles, an optional `Menu`, validation, and submission.
 * Renderer: a CSS-grid container hosting positioned child controls.
 */
export interface IGridView extends IView, IHistoryEntry {
  ViewKind?: "GridView";
  HorizontalScrollingEnabled?: boolean;
  VerticalScrollingEnabled?: boolean;
  Menu?: IMenu;
  /** `IGridBase` row definitions. */
  Rows?: unknown[];
  /** `IGridBase` column definitions. */
  Columns?: unknown[];
  /** Positioned child controls (`IGridBase.Children`). */
  Children?: ICell[];
  ValidationErrors?: Record<string, string[]>;
}

/**
 * `iFactr.UI.ITabView` — Source: `MonoView/Views/Interfaces/ITabView.cs`.
 * Top-level tab container. Renderer: a tab bar that switches sub-views.
 */
export interface ITabView extends IView {
  ViewKind?: "TabView";
  SelectedIndex?: number;
  SelectionColor?: Color;
  /** `ITabItem` collection. */
  TabItems?: Array<{ Title?: string; ImagePath?: string; NavigationLink?: Link }>;
}

/**
 * `iFactr.UI.IBrowserView` — Source: `MonoView/Views/Interfaces/IBrowserView.cs`.
 * An embedded HTML/web view. Renderer: an iframe / html host with optional
 * default Back/Forward controls and a Menu.
 */
export interface IBrowserView extends IView, IHistoryEntry {
  ViewKind?: "BrowserView";
  EnableDefaultControls?: boolean;
  Menu?: IMenu;
  /** URL to load (`Load(url)`). */
  Url?: string;
  /** Inline HTML to load (`LoadFromString(html)`). */
  Html?: string;
}

/**
 * Any view a renderer may receive, discriminated by `ViewKind`.
 */
export type AnyView = IListView | IGridView | ITabView | IBrowserView;
