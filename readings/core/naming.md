# Naming

## Description
Convention-based name projection — pluralization, slug, table name —
declared as data so the rule set lives in the readings, not in Rust.

The English pluralization cascade in `crates/arest/src/naming.rs` is
lifted (#895) to a `PluralizationRuleTable` whose rows are read at
runtime from the parallel enum value types `Pluralization Pattern` and
`Pluralization Replacement` declared below. Boot mirrors the legacy
suffix cascade so behavior round-trips on every historical input.

Pattern dialect (interpreted by `PluralizationRuleTable::pluralize`):
  - `^WORD$` matches the entire lowercased word; replacement returned
    with the leading character's case lifted from the input.
  - `SUFFIX$` matches the lowercased word's tail; the matched suffix
    is stripped from the original word and the replacement appended
    (so the prefix's case survives — `Match` + `ch$ → ches` yields
    `Matches`).
  - `$` matches any word as a zero-length suffix → trailing default,
    appended verbatim.

Order matters — the cascade is first-match-wins. Vowel-y patterns must
precede the consonant-y catchall, specific es-suffixes must precede
bare `s$`, and the empty-pattern default must be last.

## Entity Types

Pluralization Rule(.id) is an entity type.

## Value Types

Pluralization Pattern is a value type.
  The possible values of Pluralization Pattern are '^child$', '^person$', 'ay$', 'ey$', 'oy$', 'uy$', 'iy$', 'ss$', 'sh$', 'ch$', 'x$', 's$', 'z$', 'y$', '$'.
Pluralization Replacement is a value type.
  The possible values of Pluralization Replacement are 'children', 'people', 'ays', 'eys', 'oys', 'uys', 'iys', 'sses', 'shes', 'ches', 'xes', 'ses', 'zzes', 'ies', 's'.

## Fact Types

Pluralization Rule has Pattern.
  Each Pluralization Rule has exactly one Pattern.
Pluralization Rule has Replacement.
  Each Pluralization Rule has exactly one Replacement.

## Instance Facts

The parallel `Pluralization Pattern` / `Pluralization Replacement`
enum-value declarations above are the authority that
`PluralizationRuleTable::from_grammar_state` reads; the per-row
instance facts below mirror them as Pluralization Rule entities for
human-readability and tooling that walks the rule set as named
records (e.g. UI surfaces, diagnostics).

'rule-child'   has Pattern '^child$'.   'rule-child'   has Replacement 'children'.
'rule-person'  has Pattern '^person$'.  'rule-person'  has Replacement 'people'.
'rule-ay'      has Pattern 'ay$'.       'rule-ay'      has Replacement 'ays'.
'rule-ey'      has Pattern 'ey$'.       'rule-ey'      has Replacement 'eys'.
'rule-oy'      has Pattern 'oy$'.       'rule-oy'      has Replacement 'oys'.
'rule-uy'      has Pattern 'uy$'.       'rule-uy'      has Replacement 'uys'.
'rule-iy'      has Pattern 'iy$'.       'rule-iy'      has Replacement 'iys'.
'rule-ss'      has Pattern 'ss$'.       'rule-ss'      has Replacement 'sses'.
'rule-sh'      has Pattern 'sh$'.       'rule-sh'      has Replacement 'shes'.
'rule-ch'      has Pattern 'ch$'.       'rule-ch'      has Replacement 'ches'.
'rule-x'       has Pattern 'x$'.        'rule-x'       has Replacement 'xes'.
'rule-s'       has Pattern 's$'.        'rule-s'       has Replacement 'ses'.
'rule-z'       has Pattern 'z$'.        'rule-z'       has Replacement 'zzes'.
'rule-ies'     has Pattern 'y$'.        'rule-ies'     has Replacement 'ies'.
'rule-default' has Pattern '$'.         'rule-default' has Replacement 's'.

Domain 'naming' has Access 'public'.
Domain 'naming' has Description 'Convention-based name projection (pluralization rules) declared as data per the Sweep-1 dispatch-to-data lift recipe (#895). PluralizationRuleTable reads the parallel Pluralization Pattern / Pluralization Replacement enum values; boot mirrors the legacy cascade so behavior round-trips.'.
