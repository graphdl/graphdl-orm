# AREST UI: View Projection — worked draft (Support Request)

> **Status: DRAFT companion to `view-projection-design.md`.** A single
> Noun (`Support Request`) carried end-to-end through the three view
> projections, with the derivation rules instantiated against its
> actual facts. This is illustrative — it shows what the §4 rule shapes
> *produce*, so the follow-up reading and the user have a concrete
> target. Rule blocks are not yet checker-clean (see the design doc's
> negation/cardinality caveats).

## The example Noun (the data the projection reads)

A minimal `Support Request` modeled the AREST way — a Noun, its Fact
Types (typed by value type), constraints, a reference scheme, and a
state machine. (Authored against the `readings/core/core.md` and
`readings/core/state.md` metamodel fact types.)

```forml2
Support Request(.Ticket Id) is an entity type.        -- reference scheme = Ticket Id

-- value types
Ticket Id is a value type.
Subject is a value type.
Body is a value type.
Priority is a value type.
  The possible values of Priority are 'low', 'normal', 'high', 'urgent'.
Reported At is a value type.
  Reported At has Format 'date'.
Is Escalated is a value type.
  Is Escalated has Format 'boolean'.

-- fact types
Support Request has Ticket Id.
  Each Support Request has exactly one Ticket Id.        -- mandatory + identifying
Support Request has Subject.
  Each Support Request has exactly one Subject.           -- mandatory
Support Request has Body.
  Each Support Request has at most one Body.              -- optional, multiline-ish
Support Request has Priority.
  Each Support Request has exactly one Priority.          -- mandatory enum
Support Request has Reported At.
  Each Support Request has at most one Reported At.       -- optional date
Support Request is escalated.                             -- boolean fact
Support Request is reported by Customer.                  -- entity-valued fact (FK)
  Each Support Request is reported by exactly one Customer.

-- uniqueness
No two Support Requests have the same Ticket Id.

-- state machine
State Machine Definition 'SupportRequest' is for Noun 'Support Request'.
Status 'Open' is initial in State Machine Definition 'SupportRequest'.
Transition 'triage'   is from Status 'Open'        is to Status 'In Progress'.
Transition 'resolve'  is from Status 'In Progress' is to Status 'Resolved'.
Transition 'reopen'   is from Status 'Resolved'    is to Status 'Open'.
Transition 'close'    is from Status 'Resolved'    is to Status 'Closed'.
Status 'Closed' is terminal in State Machine Definition 'SupportRequest'.
```

## (1) Collection view — `get({noun:"Support Request"})`

The projection reads the **reference scheme** (`Ticket Id`) for the row
caption and a summary fact (`Subject`) for the subtitle; the `Status` is
the trailing badge; a creational button is projected from the SM's
initial-status entry.

Derived `View` + `ViewElement`s:

```forml2
View 'SupportRequest.collection' is for Noun 'Support Request'.
View 'SupportRequest.collection' has View Kind 'collection'.

ViewElement 'sr.list' belongs to View 'SupportRequest.collection'.
ViewElement 'sr.list' has Component Role 'list'.            -- §4.6
  -- row caption  = Ticket Id   (reference scheme)
  -- row subtitle = Subject     (summary fact, design Q3)
  -- row badge    = current Status
  -- row tap      = NavigationLink → /support-requests/{Ticket Id}

ViewElement 'sr.new' belongs to View 'SupportRequest.collection'.
ViewElement 'sr.new' has Component Role 'button'.           -- creational
ViewElement 'sr.new' has display- Title 'New Support Request'.
```

Serialized to the iFactr modern wire shape (`contract.ts`):

```jsonc
{
  "$type": "iFactr.UI.IListView", "ViewKind": "ListView",
  "Title": "Support Requests",
  "Sections": [{
    "Cells": [
      { "$type": "iFactr.UI.ContentCell",
        "TextLabel": "SR-1042",              // Ticket Id (ref scheme)
        "SubtextLabel": "Login page 500s",   // Subject (summary)
        "ValueLabel": "In Progress",         // Status badge
        "NavigationLink": { "Address": "/support-requests/SR-1042",
                            "RequestType": "Async" } }
    ]
  }],
  "Menu": { "$type": "iFactr.UI.IMenu",
            "Buttons": [{ "Title": "New",
                          "NavigationLink": { "Address": "/support-requests/new" } }] }
}
```

## (2) Instance / detail+form view — `get({noun:"Support Request", id:"SR-1042"})`

One `ViewElement` per Fact Type, the **value type** picking the
Component Role (§4.2), the **constraints** setting required (§4.3), the
reading/Title giving the caption (§4.4).

| Fact Type | value type facts | rule | Component Role | required? |
| --- | --- | --- | --- | --- |
| has Ticket Id | value, text | §4.2 text | `text-input` (read-only id) | yes (mandatory) |
| has Subject | value, text | §4.2 text | `text-input` | yes (mandatory) |
| has Body | value, text (long) | §4.2 text | `text-input` (multiline) | no |
| has Priority | enum (`Enum Values`) | §4.2 enum | `combo-box` (Q1) | yes (mandatory) |
| has Reported At | `Format 'date'` | §4.2 date | `date-picker` | no |
| is escalated | `Format 'boolean'` | §4.2 bool | `checkbox` | no |
| is reported by Customer | entity-valued (Noun) | §6 Q2 | `combo-box` / drill-in `card` | yes (mandatory) |

Derived elements (abbreviated):

```forml2
View 'SupportRequest.instance' is for Noun 'Support Request'.
View 'SupportRequest.instance' has View Kind 'instance'.

ViewElement 'sr.subject'  renders Fact Type 'Support Request has Subject'.
ViewElement 'sr.subject'  has Component Role 'text-input'.
ViewElement 'sr.subject'  is required.                       -- §4.3 (mandatory)
ViewElement 'sr.subject'  has display- Title 'Subject'.      -- §4.4 (FT reading)

ViewElement 'sr.priority' renders Fact Type 'Support Request has Priority'.
ViewElement 'sr.priority' has Component Role 'combo-box'.    -- §4.2 enum → combo-box
ViewElement 'sr.priority' is required.
  -- combo-box items = {'low','normal','high','urgent'} from the value type's Enum Values

ViewElement 'sr.reportedAt' renders Fact Type 'Support Request has Reported At'.
ViewElement 'sr.reportedAt' has Component Role 'date-picker'.

ViewElement 'sr.escalated' renders Fact Type 'Support Request is escalated'.
ViewElement 'sr.escalated' has Component Role 'checkbox'.

ViewElement 'sr.customer'  renders Fact Type 'Support Request is reported by Customer'.
ViewElement 'sr.customer'  has Component Role 'combo-box'.   -- §6 Q2 (edit) + NavigationLink (view)
ViewElement 'sr.customer'  is required.

ViewElement 'sr.submit'    has Component Role 'button'.      -- SubmitButton
ViewElement 'sr.submit'    has display- Title 'Save'.
```

The form's **inverse projection** (the MT.D `Fetch()` analogue): on
`Save`, each element's edited value becomes a fact in an `apply`/`create`
call. A uniqueness conflict on `Ticket Id` or a missing mandatory comes
back as `IListView.ValidationErrors` keyed by the originating Fact Type
(design §6 Q6).

This is the MT.D `Populate` of a `[Section]`-grouped object — except
required-ness, the enum value set, the uniqueness check, and the
identifying caption all came from facts MT.D has no way to express.

## (3) Menu / actions view — Theorem 4 (current Status = `In Progress`)

The SM fold (state.md) says the transitions whose `from` Status is
`In Progress` are exactly `{resolve}`. The menu projection (§4.5) yields
one button per available transition — identical to the `_links` block
the `actions` MCP verb already returns.

```forml2
View 'SupportRequest.menu' is for Noun 'Support Request'.
View 'SupportRequest.menu' has View Kind 'menu'.

ViewElement 'sr.act.resolve' renders Transition 'resolve'.   -- from 'In Progress'
ViewElement 'sr.act.resolve' has Component Role 'button'.
ViewElement 'sr.act.resolve' has display- Title 'Resolve'.
  -- (triage is NOT projected here — its from-Status is 'Open', not the current one)
```

Serialized (the menu = Theorem 4 `_links`, two fidelities of one
projection):

```jsonc
// HATEOAS _links (existing, from the SM fold):
{ "id": "SR-1042", "status": "In Progress",
  "_links": { "resolve": { "href": "/support-requests/SR-1042/transition",
                           "method": "POST", "event": "resolve" } } }

// the same projection as an iFactr IMenu (the menu View):
{ "$type": "iFactr.UI.IMenu",
  "Buttons": [{ "$type": "iFactr.UI.IMenuButton", "Title": "Resolve",
    "NavigationLink": { "Address": "/support-requests/SR-1042/transition",
                        "RequestType": "Async", "Parameters": { "event": "resolve" } } }] }
```

After the POST, the SM advances to `Resolved`; re-projecting the menu
yields `{reopen, close}` — no view code changed, the projection just
re-ran over the new Status. That round-trip *is* the MXController
binding: the button's `clicked` Event (components.md) carries the
transition name; the generic controller POSTs and re-fetches; the
re-projected view reflects the new state. One controller, every Noun.

## Override demo (the MT.D-attribute analogue)

To make `Priority` render as a radio set instead of the default
combo-box for *this* Noun only, a domain reading asserts the override —
the derived rule's `no … asserted` guard then yields:

```forml2
ViewElement 'sr.priority' has Component Role 'list'.   -- asserted override (radio-style set)
```

To hide `Body` from the collection summary (MT.D `[Skip]` analogue):

```forml2
Fact Type 'Support Request has Body' is not projected into View 'SupportRequest.collection'.
```

No rule edits, no per-Noun view code — the projection defaults hold for
every other Noun, and the override is a fact, exactly like asserting a
`PanePreference` over a default in `monoview.md`.
