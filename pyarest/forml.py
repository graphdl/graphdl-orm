"""compile ∘ parse (D3, Cor. closure): a FORML 2 reading, parsed to M-facts and asserted by
`create` with the addressed entity being M ITSELF. There is no compiler subsystem — compiling
a schema is an ordinary command whose cell is one of M's populations, validated by M's own
constraints and committed iff valid (exactly the create path any user command takes).

`parse` is the tokenizer boundary: strings are outside the algebra (spec D5), so text → object
is host code on the enumerable frontier. Everything after parse — the assertion, the validation,
the commit — is `create` over the kernel. Readings outside the fragment R (pronoun-correlated,
nested objectification) are rejected (Def. Fragment).

Scope here is a focused slice — entity/value-type declarations — enough to exhibit the D3
architecture; the remaining reading families (fact types, the constraint families, derivation
and state-machine readings) parse to their own M-facts and are asserted by the same `create`.
"""
import re
from .lam import to_lam, from_lam
from . import ast, system

_OUT_OF_R = (" who ", " that ", " which ")                   # pronoun-correlated clauses (Def. Fragment)


def parse(reading):
    """A FORML reading → ⟨cell, fact⟩: the M population to assert into and the fact to assert.
    Total on the slice it covers; raises on a reading outside the fragment R."""
    text = reading.strip()
    if any(p in f" {text} " for p in _OUT_OF_R):
        raise ValueError(f"reading outside the fragment R (pronoun-correlated): {reading!r}")
    m = re.match(r"^(\w+) is an entity type$", text)
    if m:
        return ("instanceOf", to_lam((m.group(1), "ObjectType")))
    m = re.match(r"^(\w+) is a value type$", text)
    if m:
        return ("instanceOf", to_lam((m.group(1), "ValueType")))
    raise ValueError(f"reading outside the fragment R: {reading!r}")


def compile(reading, D, constraints=()):
    """compile = create over M: parse the reading and assert its fact into M's population via the
    ordinary command transition, validated by M's own `constraints`, committed iff valid. Returns
    ⟨⟨M'', V⟩, D'⟩ — the same shape as any create (Cor. closure: schema change is not special)."""
    cell_name, fact = parse(reading)
    validate = system.validate_of(list(constraints)) if constraints else None
    return ast.run(fact, D, validate_obj=validate, cell_name=cell_name)


def verbalize(cell_name, fact):
    """The inverse of parse: an M-fact → its FORML 2 reading. Total over what parse covers, so
    the round trip is exact on the fragment R (Prop. Spec)."""
    f = fact if isinstance(fact, tuple) else from_lam(fact)
    if cell_name == "instanceOf":
        name, kind = f
        if kind == "ObjectType":
            return f"{name} is an entity type"
        if kind == "ValueType":
            return f"{name} is a value type"
    raise ValueError(f"cannot verbalize {cell_name} fact {f!r}")


def nf(reading):
    """Normal form: nf = verbalize ∘ parse. Idempotent on the fragment R (Prop. Spec) — the
    round trip through the object layer and back is exact."""
    cell_name, fact = parse(reading)
    return verbalize(cell_name, fact)
