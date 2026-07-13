# Resolution Registry Catalog

<!-- The canon side of the Resolution Registry (docs ch. 15): which named
     operations admit a certified per-platform override. An operation's
     reference implementation is the canon DEF carrying its name (or, for a
     verb, the canon pipeline the verb reduces); a platform's fast override
     is held byte-equal to that reference by a parity pin behind the one
     kill switch (AREST_NO_OVERRIDE). The catalog is data so a target can
     enumerate what it is expected to twin and the parity-pin list can
     generate from it. The override bindings themselves are per-platform
     code, never canon data. -->

## Entity Types

Operation(.name) is an entity type.

## Fact Types

Operation is overridable.

## The catalog

<!-- DEF-level: the operation name is the canon DEF the override twins. -->
Operation 'system:ev_cols' is overridable.
Operation 'system:entity_view' is overridable.
Operation 'system:vb_fetch' is overridable.
Operation 'theta:NatJoin' is overridable.
Operation 'theta:append_phi' is overridable.
Operation 'theta:flatten' is overridable.
Operation 'theta:join_combine' is overridable.
Operation 'theta:member' is overridable.
Operation 'theta:dedup' is overridable.

<!-- Verb-level: the operation is a verb whose reference is the canon
     pipeline it reduces; the override is the host's native route. -->
Operation 'query' is overridable.
Operation 'synthesize' is overridable.
Operation 'apps_compile' is overridable.
Operation 'verify' is overridable.
Operation 'validate' is overridable.
Operation 'apply' is overridable.
Operation 'retract' is overridable.
Operation 'get' is overridable.
Operation 'actions' is overridable.
Operation 'schema' is overridable.
Operation 'cells' is overridable.
Operation 'derive' is overridable.
Operation 'nav' is overridable.
Operation 'explain' is overridable.
Operation 'induce' is overridable.
