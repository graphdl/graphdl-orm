# The polyglot stdio binding

What a host IS comes from the paper, not from this file: an implementation of the
reduction over the object algebra plus whatever it registers into DEFS (Def. 5's
definition tuples; the Platform binding paragraph: "a browser registers rendering
functions, a server registers httpFetch and upsert, and one SYSTEM serves browser,
server, and storage by varying DEFS rather than the logic"). The theory never
mentions processes or JSON. This file documents ONE platform binding, the carriage
this repo uses between its Python and Rust hosts: JSON over stdio. It sits at the
same level as httpFetch, a named binding, not spec.

Within that binding, everything above the lambda kernel is a value, so it travels:
the compiled definitions, the store D, the machines, the constraints, and M itself
cross as data. Only the base stratum (the Backus primitives and the Scott machinery)
is implemented per host, under the differential's equality contract. This file
records the carriage between python/pyarest/polyglot.py and rust/src/main.rs so a
further host can adopt it from shared/ without reading either, or supply its own
carriage and still be a host by passing the differential.

## Value encoding

Sequences encode as JSON arrays, recursively. Leaves encode as JSON scalars: strings,
integers, and floats are DISTINCT ORM value types (NATEQ compares type first, then
value; 1 and 1.0 are not equal). Bottom (the paper's ⊥) has no request encoding and
answers as JSON `null` in results. The application sentinel tag is reserved to the
kernels and never crosses the wire.

## One-shot scenario (stdin to stdout)

One JSON object on stdin:

    {"d": <D>,                     the store, a value
     "overrides": 0 | 1,           1 registers the host's FAST twins (cleared first);
                                   0 clears them, pure canonical evaluation
     "process": [[name, obj], …],  the compiled definitions, canonical objects as data
     "cases": [{"f": obj, "x": obj, "fuel": int}, …]}

The kernel evaluates each case as f applied to x within a DEFS step frame over d and
process, and answers ONE JSON line per case; `null` marks ⊥. `fuel` 0 or absent means
unbounded; positive bounds the reduction budget.

## Serve mode (--serve, resident)

One JSON object per request line, one JSON line (an array of case results) per reply.
Request keys, all optional:

* `d`, `process`, `overrides` — set or replace the RETAINED store, as in one-shot.
  A request carrying only these (with `cases: []`) is the set_store call.
* `cases` — as in one-shot, with two additions per case:
  * `xd` in place of `x`: the operand becomes the pair ⟨xd, D_retained⟩ without
    re-serializing D. This is the machine-step shape.
  * `retain`: 1 commits the case result's D′ into the retained store (the owner
    instance evolves; a refused step retains nothing).
* `engine`: `"native"` selects the native carrier (the deepest override, same
  protocol, certified equal three ways by the differential); absent means the Scott
  closures.
* `dump`: any value; the reply is prefixed with the retained store's serialization
  (the round-trip check).

## The contract

The Python Scott mu is ground truth. The differential (tests/test_polyglot.py)
asserts agreement across the Python kernel, the Rust Scott kernel, the Rust FAST
overrides, and the native carrier, on the theta/constraint battery and the machine
scenarios. A new host joins by implementing the base stratum and passing the same
differential; nothing above the base needs porting, because it arrives as data
(`process`, `d`). The stdio JSON carriage documented here is the convenient way to
run the differential against a new kernel, not a requirement of hosthood.
