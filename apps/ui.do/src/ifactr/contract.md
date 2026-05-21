# iFactr abstract-UI → ui.do ("iFactr.React") contract

A faithful extraction of iFactr's abstract-UI `IInterface` contracts and the
**serialized-object shapes** a renderer consumes. The TypeScript spec lives in
[`contract.ts`](./contract.ts); this file maps each C# interface to its TS type
and to what a React renderer draws, plus notes on the serialization shape and
open questions.

Source (read-only): `iFactr-Android/iFactr.UI/iFactr.UI`.

## The two UI layers

iFactr ships **two parallel UI models**, and a renderer must understand both:

| Layer | Namespace | Examples | Serialized? |
| --- | --- | --- | --- |
| **Modern MonoView** (native) | `iFactr.UI` | `IView`, `IListView`, `IGridView`, `ICell`, `Section`, `IMenu`, `IMenuButton` | No — cells are produced lazily through delegates (`CellRequested`, `ItemIdRequested`). These define field **names/semantics**, not a wire format. |
| **Legacy iFactr.Core** (abstract) | `iFactr.Core.Layers` / `iFactr.Core.Controls` | `iLayer`, `iLayerItem`, `iList`, `iMenu`, `iItem`, `iBlock`, `iPanel`, `Link`, `Button`, `Icon`, `Label` | **Yes** — these carry `[XmlType]`/`[XmlInclude]` and implement `IXmlSerializable`. This is the on-the-wire object graph. |

The directive ("serialized iFactr object compatibility — same `IInterface`
contracts") is satisfied by mirroring the **legacy serialized graph** with
faithful field names while also expressing the **modern view contracts** that a
renderer dispatches on. `contract.ts` covers both.

## Serialization shape (the wire format)

iFactr serializes with **`System.Xml.Serialization.XmlSerializer`** — *not*
DataContract and *not* JSON. Key mechanics (Source: `Views/Items/iLayerItem.cs`,
`ItemsCollection.cs`):

- Polymorphic collections (`ItemsCollection<T>`, and `IXmlSerializable` types
  `iList`/`iMenu`/`iBlock`/`iPanel`) write **each element under an element name
  equal to the .NET type's `FullName`** (e.g. `iFactr.Core.Layers.SubtextItem`).
- For **cross-assembly** element types, the element name becomes the
  assembly-qualified name (minus version) with `, ` replaced by `___`; on read
  it is reversed (`Replace("___", ", ")`) before `Type.GetType(...)`.
- The xml declaration and the `xsi`/`xsd` namespace decls are stripped from each
  fragment before being written into the parent stream.
- Controls use `[XmlType]` aliases: `ActionType` → `CoreActionType` (ns `Core`),
  `Button.ActionType` → `ButtonActionType` (ns `Button`), `Link` → `CoreLink`
  (ns `Core`). `Link` declares `[XmlInclude]` for `Button`, `SubmitButton`,
  `CancelButton` (its serialized subtypes).
- `[XmlIgnore]` members are NOT on the wire — notably `iLayer.ID`,
  `iLayer.MapUri`, `iLayer.Parameters`/`ActionParameters`, `iLayer.NavContext`,
  `iLayer.ValidationErrors`, `iLayer.View`, and `Button.Action`
  (the `ButtonActionType` overload is `[XmlIgnore]`; the underlying
  `Link.Action` `CoreActionType` is what serializes).

**Implication for the type discriminator.** Because the **element/type name is
the discriminator**, `contract.ts` adds a `$type` field (the .NET `FullName`) on
every polymorphic node via the `ITyped` base. A JSON/HATEOAS projection of the
same object graph should carry that discriminator so deserialization stays
unambiguous. The modern view kinds add a parallel `ViewKind` discriminant for
top-level dispatch.

## Mapping table

### Views

| iFactr C# (source) | TS type | React renderer draws |
| --- | --- | --- |
| `MonoCross.Navigation.IMXView` (`MXView.cs`) | `IMXView` | Root dispatch: pick a view by kind, bind `Model`. |
| `iFactr.UI.IView` (`Views/Interfaces/IView.cs`) | `IView` | Page frame: header bar + `Title`/`TitleColor`/`HeaderColor`, background, hosts content. |
| `iFactr.UI.IListView` (`Views/Interfaces/IListView.cs`) | `IListView` | Scrolling list of `Sections`→cells, optional `SearchBox` + `Menu`, column/separator styling. |
| `iFactr.UI.IGridView` (`Views/Interfaces/IGridView.cs`) | `IGridView` | CSS-grid surface hosting positioned `Children`, scroll toggles, `Menu`. |
| `iFactr.UI.ITabView` (`Views/Interfaces/ITabView.cs`) | `ITabView` | Tab bar switching `TabItems`. |
| `iFactr.UI.IBrowserView` (`Views/Interfaces/IBrowserView.cs`) | `IBrowserView` | Embedded HTML host (`Url`/`Html`) + optional back/fwd controls. |
| `iFactr.UI.IHistoryEntry` (mixin) | `IHistoryEntry` | Back button + stack/breadcrumb behavior. |

### List content (modern)

| iFactr C# (source) | TS type | React renderer draws |
| --- | --- | --- |
| `iFactr.UI.Section` / `SectionCollection` (`Cells and Tiles/Section.cs`) | `Section` | A list section: header + N cells + footer. |
| `iFactr.UI.ICell` (`Cells and Tiles/Interfaces/ICell.cs`) | `ICell` | One row/tile (sizing + background + metadata). |
| `iFactr.UI.ContentCell` (`Cells and Tiles/ContentCell.cs`) | `IContentCell` | Image + title + subtitle + trailing value; tap → `NavigationLink`. |

### Menus & toolbars

| iFactr C# (source) | TS type | React renderer draws |
| --- | --- | --- |
| `iFactr.UI.IMenu` (`Menus and Toolbars/Interfaces/IMenu.cs`) | `IMenu` | A view's overflow/action menu (`Buttons`). |
| `iFactr.UI.IMenuButton` (`Menus and Toolbars/Interfaces/IMenuButton.cs`) | `IMenuButton` | A single menu/toolbar action; navigates `NavigationLink`. |

### Controls (serialized)

| iFactr C# (source) | TS type | React renderer draws |
| --- | --- | --- |
| `iFactr.Core.Controls.IBlockPanelItem` (`Controls/IBlockPanelItem.cs`) | `IBlockPanelItem` | Marker base for block/panel content. |
| `iFactr.Core.Controls.Link` (`Controls/Link.cs`) | `Link` | Anchor/pressable → navigate to `Address` (honoring `RequestType`, confirmation). |
| `iFactr.Core.Controls.Button` (`Controls/Button.cs`) | `Button` | Button; label/affordance defaults from `Action` when `Text` empty. |
| `iFactr.Core.Controls.SubmitButton` (`Controls/SubmitButton.cs`) | `SubmitButton` | Submit button (gathers + posts the form). |
| `iFactr.Core.Controls.CancelButton` (`Controls/CancelButton.cs`) | `CancelButton` | Cancel button. |
| `iFactr.Core.Controls.Icon` (`Controls/Icon.cs`) | `Icon` | Sized/aligned `<img>`. |
| `iFactr.Core.Controls.Label` (`Controls/Label.cs`) | `Label` + `LabelStyle` | Text run with heading level + bold/italic emphasis. |
| `iFactr.Core.Controls.ActionType` (`Controls/ActionType.cs`) | `ActionType` | Default label/icon/affordance per action. |

### Legacy serialized layer graph

| iFactr C# (source) | TS type | React renderer draws |
| --- | --- | --- |
| `iFactr.Core.Layers.iLayer` (`Views/Layers/iLayer.cs`) | `iLayer` | The page model: header (`Title`) + `ActionButtons`, then `Items` top→bottom. |
| `iFactr.Core.Layers.iLayerItem` (`Views/Items/iLayerItem.cs`) | `iLayerItem` | Abstract base for layer content (`Header`/`Footer`). |
| `iFactr.Core.Layers.iCollection<iItem>` (`Views/Items/iCollection.cs`) | `iCollection` | A named, header/footer-bearing group of rows. |
| `iFactr.Core.Layers.iList` (`Views/Items/iList.cs`) | `iList` | A list/section; `DisplayStyle` selects the row template. |
| `iFactr.Core.Layers.iMenu` (legacy) (`Views/Items/iMenu.cs`) | `iMenuLegacy` | An action/overflow menu of rows. |
| `iFactr.Core.Layers.iItem` (`CollectionItems/iItem.cs`) | `iItem` | A list row: text/subtext/icon + tap → `Link` (+ optional `Button`). |
| `iFactr.Core.Layers.iBlock` (`Views/Items/iBlock.cs`) | `iBlock` | A rich-text/HTML block of `PanelItem`s. |
| `iFactr.Core.Layers.iPanel` (`Views/Items/iPanel.cs`) | `iPanel` | A styled rich content panel. |

### Enums

| iFactr C# (source) | TS type |
| --- | --- |
| `iFactr.Core.Controls.ActionType` (`Controls/ActionType.cs`) | `ActionType` |
| `iFactr.UI.RequestType` (`Link.cs`) | `RequestType` |
| `iFactr.Core.Controls.Link.Rev` (`Controls/Link.cs`) | `LinkRev` |
| `iFactr.Core.Controls.Button.Position` (`Controls/Button.cs`) | `ButtonPosition` |
| `iFactr.UI.ListViewStyle` (`MonoView/Enums/ListViewStyle.cs`) | `ListViewStyle` |
| `iFactr.UI.ColumnMode` (`MonoView/Enums/ColumnMode.cs`) | `ColumnMode` |
| `iFactr.Core.Layers.LayerLayout` (`Views/Enums/LayerLayout.cs`) | `LayerLayout` |
| `iFactr.Core.Layers.iList.StyleTypes` (`Views/Items/iList.cs`) | `ListStyleTypes` |

## Gaps & assumptions (for the orchestrator/user to resolve)

1. **XML, not JSON.** The native iFactr wire format is `XmlSerializer` XML with
   element-name type tags. ui.do is a JSON/HATEOAS thin client, so a transform
   (XML→JSON) is required somewhere. The spec models the **logical object graph**
   and adds `$type` to preserve the discriminator. *Decision needed:* does the
   AREST worker emit JSON projections of these objects, or raw iFactr XML that
   ui.do parses? `$type` naming (full .NET `FullName` vs. a short alias) should
   match whatever the producer emits.

2. **Delegates don't serialize.** Modern `IListView`/`Section` produce cells via
   `CellRequested(section, index, recycledCell)` and `ItemIdRequested` — runtime
   delegates with no wire representation. The spec assumes a **realized**
   projection: `Section.ItemCount` + an inlined `Cells: ICell[]`. The producer
   must materialize cells before sending (the legacy `iList`/`iItem` graph is the
   already-materialized equivalent and is the safer compatibility target).

3. **Two `iMenu`/menu concepts.** Modern `iFactr.UI.IMenu` (view support menu)
   and legacy `iFactr.Core.Layers.iMenu` (an `iItem` collection on a layer) are
   distinct; modeled as `IMenu` and `iMenuLegacy` respectively. Confirm which the
   renderer will actually receive.

4. **`Color` shape.** `iFactr.UI.Color` was not read in full; modeled as ARGB +
   `HexCode` (the `HexCode`/`A` accessors are used in `iLayerItem.GetLayerStyle`).
   Verify exact serialized member names if colors come over the wire.

5. **`Button.Action` `[XmlIgnore]` subtlety.** The strongly-typed
   `ButtonActionType` overload is `[XmlIgnore]`; the serialized value is the base
   `Link.Action` (`CoreActionType`). The TS `Button.Action` reuses `ActionType`
   (member names are identical), so this is transparent for a JSON projection but
   matters if reading raw XML.

6. **HTML projection of blocks.** `iBlock`/`iPanel`/`Label`/`Icon`/`Link` expose
   `GetHtml()`; the HTML is regenerated from data fields and is **not** part of
   the serialized state. The renderer should rebuild presentation from the typed
   fields rather than relying on a pre-rendered HTML string.

7. **Scope of cell subtypes.** Only `ContentCell` (modern) and `iItem`/
   `SubtextItem` (legacy) are modeled concretely. Other `CollectionItems/*`
   (MultiLineSubtextItem, RightSubtextItem, PhotoItem, ShopItem, MessageItem,
   VariableContentItem, ContentItem) and modern `RichContentCell`/`GridCell`
   share the `iItem`/`ICell` bases and are distinguished by `$type`; add explicit
   interfaces if the renderer needs per-subtype layout fidelity.
