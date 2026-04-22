# Func AOT-Compile on First Apply (#318)

The interpreter dispatch in `ast::apply_nonbottom` is an enum match
over every `Func` variant, repeated on every apply call of every
sub-tree. For rule-firing hot paths (forward chain, RMAP joins,
check.rs), the same Func tree is applied thousands of times across a
stable domain — the outer match, tree walk, and sub-apply recursion
dominate the cost profile even though the per-variant work is small.

`#318` is the step that closes that loop: compile each `Func` once on
first apply into a closure that has sub-closures baked in, cache the
result keyed by the Func's identity, and on subsequent applies dispatch
through the cached closure directly. Interpreter dispatch collapses
into a pointer call tree.

## What's already in place

- `ast::apply_nonbottom` is the pure dispatcher — no hidden state, so
  turning it into a one-shot compilation is mechanical.
- `wasm_lower::lower_to_wasm` already lowers a Func to a WASM module
  for the engine-as-runtime path (#152). A slice that routes WASM
  through wasmtime is the "AOT-via-WASM" variant of this task.
- `Func` has a canonical `Eq + Hash` representation via
  `func_to_object` + `freeze` — an Object byte-level hash is a
  cache key that survives AST rewrites.

## Compilation shape

```rust
type ApplyFn = Arc<dyn Fn(&Object, &Object) -> Object + Send + Sync>;

fn compile(f: &Func) -> ApplyFn {
    match f {
        Func::Id            => Arc::new(|x, _d| x.clone()),
        Func::Selector(n)   => { let n = *n; Arc::new(move |x, _| apply_selector(x, n)) }
        Func::Compose(g, h) => {
            let gc = compile(g);
            let hc = compile(h);
            Arc::new(move |x, d| { let mid = hc(x, d); gc(&mid, d) })
        }
        Func::ApplyToAll(f) => {
            let inner = compile(f);
            Arc::new(move |x, d| match x.as_seq() {
                Some(items) => Object::Seq(items.iter().map(|it| inner(it, d)).collect()),
                None => Object::Bottom,
            })
        }
        // Filter / Insert / Condition: same shape, sub-closure captured
        // at compile time so no re-match at dispatch time.

        // Leaves that need D or cell access stay as the interpreter
        // would handle them — compilation wraps but doesn't replace.
        leaf => {
            let leaf = leaf.clone();
            Arc::new(move |x, d| apply_nonbottom(&leaf, x, d))
        }
    }
}
```

The recursive forms (`Compose`, `Construction`, `Condition`,
`ApplyToAll`, `Filter`, `Insert`, `While`, `FoldL`, `BU`) each
specialize — the interesting property is that the sub-Funcs' closures
are captured at compile time so the tree walk happens once, not per
apply. Leaf variants (arithmetic, comparisons, `Fetch`, `Platform`)
fall back to `apply_nonbottom`.

## Cache layout

```rust
struct AotCache {
    // Keyed by freeze-hash of the Func — stable across process restarts
    // so the baked metamodel path (#185 freeze + kernel boot) can
    // skip compilation entirely.
    entries: DashMap<u64, ApplyFn>,
}
```

Per-tenant or global — per-tenant isolates memory pressure, global
shares work across tenants that derive the same rules. Start with
global; switch to per-tenant if one tenant's huge derivation graph
starves others.

## Wiring

`apply` gains a fast path:

```rust
pub fn apply(func: &Func, x: &Object, d: &Object) -> Object {
    if !consume_fuel() { return Object::Bottom; }
    if x.is_bottom() { return Object::Bottom; }

    let key = func_hash(func);
    let compiled = AOT_CACHE.get_or_insert(key, || compile(func));
    compiled(x, d)
}
```

`func_hash` is `freeze(&func_to_object(f))` → FNV-1a. The first apply
is O(|Func|) to compile; subsequent applies are O(1) plus the
closure's native cost.

## Acceptance

- `compile(f)(x, d) == apply(f, x, d)` for every Func/Object/State
  triple in an enumerated test matrix (Compose, Construction,
  Condition, ApplyToAll, Filter, Insert, BU, While, FoldL +
  primitive leaves).
- Benchmark: 10× speedup on the bundled-metamodel fixpoint when the
  same Func is applied ≥ 100 times against a ≥ 100-element
  population. Target: the `forward_chain_defs_state` benchmark
  added in #297 drops from its current ms-scale number per full
  pass to sub-ms.
- No regression: existing lib + properties suites stay green.

## Staging

1. Land the `compile` primitive as a pure function on `Func` with a
   small test matrix. No caching, no wiring into `apply` yet.
2. Add the cache, gated behind a `aot-compile` feature flag so the
   interpreter path remains the default while the closure tree is
   measured.
3. Benchmark forward_chain, RMAP compile, check.rs run — compare
   cached vs interpreter.
4. Flip the feature flag to default-on once the bench shows ≥ 3×
   on the hot paths.
5. `Func` freeze-roundtrip test confirms the cache key survives
   kernel-image baking (#185).

## Not-in-scope (follow-up)

- Native codegen (cranelift / LLVM). WASM via wasmtime (#152) is the
  intermediate step; native is only worth it if wasmtime's overhead
  shows up on the bench.
- Partial evaluation (constant folding, dead-code elim over Func
  trees). The closure-tree form above is structural compile only.
- AOT across apply boundaries — e.g., fusing `ApplyToAll(Compose)`
  passes into one loop. Saves allocation but deep compiler work;
  defer until bench says it matters.
