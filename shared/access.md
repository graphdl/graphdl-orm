# Access — the standard authorization module

User is not core metamodel: this module is ordinary readings an application ingests or
does not (composition is the tree-shaking). Authorization is a DERIVED fact type with
full rule power; enforcement is one membership check in create, and an engine without
this module ingested proceeds ungoverned (graceful absence, one closed-default
declaration away).

User(.Id) is an entity type.
Role(.Name) is an entity type.
Operation(.Name) is an entity type.
Resource(.Name) is an entity type.

User has Role.
Role grants Operation on Resource.

User1 is authorized for Operation2 on Resource3 if User1 has Role4 and Role4 grants Operation2 on Resource3.
