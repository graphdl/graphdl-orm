# 02 · Writing Readings

A reading is a single FORML 2 sentence. Readings are grouped into markdown files under `readings/` and compiled together. Each file may declare entity types, value types, fact types, constraints, state machines, derivation rules, and instance facts. The order within a file is flexible, since the parser accumulates declarations and resolves cross-references at compile time.

## Minimal file structure

```markdown
# Orders

## Entity Types

Order(.Order Id) is an entity type.
Customer(.Name) is an entity type.

## Fact Types

Order was placed by Customer.
```

The level-1 heading is the domain name (optional, informational). The level-2 headings (`Entity Types`, `Fact Types`, `Constraints`, `State Machines`, `Derivation Rules`, `Instance Facts`) are informational groupings. The parser reads the whole file; it does not require the headings.

## Entity types

An entity is a thing with an identity. Declare one with a reference scheme in parentheses, prefixed by a dot:

```forml2
Order(.Order Id) is an entity type.
Customer(.Name) is an entity type.
Product(.SKU) is an entity type.
```

The ref scheme names the value that identifies instances. `.Order Id` means "Order is identified by its Order Id." If you omit the ref scheme, it defaults to `.id`:

```forml2
Session is an entity type.    -- identified by .id
```

### Compound reference schemes

Some entities are identified by more than one value. Declare several components separated by commas:

```forml2
Resource(.Domain, .Slug) is an entity type.
```

When you refer to instances, join the components with a hyphen:

```forml2
Resource 'myapp-orders' has Title 'Order management'.
```

The compiler will split `'myapp-orders'` into `Domain='myapp'` and `Slug='orders'` and push both as component facts so joins and constraints work on either part.

## Value types

A value is an atom that participates in fact types but has no independent identity. Names, numbers, enums, timestamps:

```forml2
Order Id is a value type.
Name is a value type.
Amount is a value type.
```

Value types with a fixed domain get a declaration:

```forml2
Severity is a value type.
  The possible values of Severity are 'error', 'warning', 'info'.
```

The compiler generates a VC (Value Comparison) constraint that rejects any other value.

## Fact types

A fact type is a named predicate over one or more object types. Binary fact types are the most common:

```forml2
Order has Amount.
Order was placed by Customer.
Employee reports to Employee.
```

Unary fact types express properties:

```forml2
Order is completed.
User is authenticated.
```

Ternary or higher fact types are legal but rarer; they typically get their own table after RMAP:

```forml2
Employee earns Salary in Year.
```

### Readings and verbs

Every fact type has one primary reading with one verb. You can add alternate readings for different directions:

```forml2
Customer purchases Product.
Product is bought by Customer.
```

Both readings refer to the same fact type (same roles, same tuple). The Verb entity (`purchase`, `buy`) determines the reading's orientation. A fact asserted via one reading is visible through the other; they are synonyms.

For a ring fact type (both roles played by the same noun), give the reading a clear direction:

```forml2
Employee reports to Employee.
```

The first role is `reports to`'s source (the reporter); the second is its target (the manager). Ring constraints below take these positions into account.

## Instance facts

Once you have fact types, you can declare instances. Put them under `## Instance Facts`:

```forml2
## Instance Facts

Order 'ord-1' was placed by Customer 'acme'.
Order 'ord-1' has Amount '42.00'.
Customer 'acme' has Email 'billing@acme.com'.
```

Quoted strings identify specific instances. If the noun uses a compound ref scheme, the component parts are inferred:

```forml2
Resource 'myapp-orders' has Title 'Order management'.
-- → Resource.Domain = 'myapp', Resource.Slug = 'orders'
```

Instance facts accumulate in the population `P`. At create time via the MCP API, they reach `P` by the same path; ultimately, `assert` pushes facts into the FILE cell.

## Subtyping

A subtype inherits all the fact types of its supertype:

```forml2
Employee is a subtype of Person.
Manager is a subtype of Employee.
```

Every Employee is a Person; every Manager is both. Subtype hierarchies can be deep. Mutually-exclusive subtypes are declared with a set notation:

```forml2
{Individual, Organization} are mutually exclusive subtypes of Party.
```

Disjunctive completeness (every Party must be one of the subtypes):

```forml2
Each Party is an Individual or an Organization.
```

RMAP chooses between partitioned tables (each subtype gets its own) and single-table inheritance depending on whether the subtype has fact types of its own.

## What's next

You now have entity types, fact types, and the reading grammar. The next chapter, [Constraints](03-constraints.md), covers all 17 kinds and shows how to use them to keep your data honest.
