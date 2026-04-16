// crates/arest/src/query.rs
//
// Population query via partial application of fact types.
//
// A query is a partially applied fact type (Backus 1977):
//   Schema     = CONS(Sel(1), ..., Sel(n))       — a Construction
//   Bind roles = eq ∘ [Sel(i), valuē]             — predicate per bound role
//   Filter     = Filter(predicate)                — keep matching facts
//   Extract    = α(Sel(target))                   — map selector over matches
//   Query      = α(Sel(target)) ∘ Filter(pred)   — composed function
//   Execute    = apply(query, population)          — beta reduction
//
// No Func::Native. No manual iteration. Pure AST throughout.

use crate::ast::{self, Func, Object};
use crate::compile::CompiledSchema;
#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned};

// ── Partial application as query ────────────────────────────────────
// Query = partial application of a fact type.
// Given a schema and known bindings, return matching resources from population.

/// Convert facts from an Object state for a given fact type into a positional Object sequence.
/// Each fact becomes a sequence ordered by the schema's role_names.
pub(crate) fn state_to_object(state: &Object, schema: &CompiledSchema) -> Object {
    let facts = ast::fetch_or_phi(&schema.id, state);
    let items = match facts.as_seq() {
        Some(fact_objs) => fact_objs.iter().map(|fact| {
            let bindings: Vec<Object> = schema.role_names.iter().map(|role_name| {
                ast::binding(fact, role_name)
                    .map(Object::atom)
                    .unwrap_or(Object::Bottom)
            }).collect();
            Object::seq(bindings)
        }).collect(),
        None => return Object::phi(),
    };
    Object::Seq(items)
}

/// Build a predicate Func from filter bindings.
///
/// Single binding:  eq ∘ [Sel(i), valuē]
/// Multiple:        nested Condition — each check gates the next (pure AND).
/// Zero:            constant T (match all)
fn build_predicate(filter_bindings: &[(usize, &str)]) -> Func {
    let checks: Vec<Func> = filter_bindings.iter().map(|(role_idx, value)| {
        Func::compose(
            Func::Eq,
            Func::construction(vec![
                Func::Selector(*role_idx),
                Func::constant(Object::atom(value)),
            ]),
        )
    }).collect();

    match checks.len() {
        0 => Func::constant(Object::t()),
        1 => checks.into_iter().next().unwrap(),
        _ => {
            // AND via nested Condition: (p₁ → (p₂ → ... → T̄; F̄); F̄)
            // Each check gates the next — all must pass.
            checks.into_iter().rev().fold(
                Func::constant(Object::t()),
                |inner, check| Func::condition(check, inner, Func::constant(Object::f())),
            )
        }
    }
}

/// Build a query Func: α(Sel(target)) ∘ Filter(predicate).
///
/// This is partial application of a fact type:
///   Schema = CONS(Sel(1), ..., Sel(n))
///   Bind some roles to constants → predicate
///   Filter(predicate) selects matching facts
///   α(Sel(target)) extracts the free role from matches
pub fn build_query(target_role: usize, filter_bindings: &[(usize, &str)]) -> Func {
    let predicate = build_predicate(filter_bindings);
    Func::compose(
        Func::apply_to_all(Func::Selector(target_role)),
        Func::filter(predicate),
    )
}

/// Query an Object state using partial application of a fact type.
///
/// Given a compiled schema, a role index to extract (1-indexed), and
/// filter bindings (role_index, value), returns matching values.
///
/// This is: α(Sel(target)) ∘ Filter(predicate) applied to the state.
/// Pure AST — no Native closures, no manual iteration.
pub(crate) fn query_with_ast(
    state: &Object,
    schema: &CompiledSchema,
    target_role: usize,
    filter_bindings: &[(usize, &str)],
) -> Vec<String> {
    let pop = state_to_object(state, schema);
    let query = build_query(target_role, filter_bindings);
    let result = ast::apply(&query, &pop, &ast::Object::phi());

    match result.as_seq() {
        Some(items) => items.iter()
            .filter_map(|obj| obj.as_atom().map(|s| s.to_string()))
            .collect(),
        None => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── AST-based query tests (partial application) ─────────────

    fn make_schema(id: &str, role_names: Vec<&str>) -> CompiledSchema {
        let selectors: Vec<Func> = (0..role_names.len())
            .map(|i| Func::Selector(i + 1))
            .collect();
        CompiledSchema {
            id: id.to_string(),
            reading: String::new(),
            construction: Func::Construction(selectors),
            role_names: role_names.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn state_with_facts(ft_id: &str, pairs_list: &[&[(&str, &str)]]) -> ast::Object {
        pairs_list.iter().fold(ast::Object::phi(), |acc, pairs|
            ast::cell_push(ft_id, ast::fact_from_pairs(pairs), &acc))
    }

    #[test]
    fn ast_query_filters_by_single_binding() {
        let schema = make_schema("ft1", vec!["User", "Organization"]);
        let state = state_with_facts("ft1", &[
            &[("User", "alice"), ("Organization", "org-1")],
            &[("User", "bob"), ("Organization", "org-2")],
            &[("User", "alice"), ("Organization", "org-3")],
        ]);

        let results = query_with_ast(&state, &schema, 2, &[(1, "alice")]);
        assert_eq!(results, vec!["org-1", "org-3"]);
    }

    #[test]
    fn ast_query_filters_by_multiple_bindings() {
        let schema = make_schema("ft1", vec!["User", "Role", "Organization"]);
        let state = state_with_facts("ft1", &[
            &[("User", "alice"), ("Role", "owner"), ("Organization", "org-1")],
            &[("User", "alice"), ("Role", "member"), ("Organization", "org-2")],
        ]);

        let results = query_with_ast(&state, &schema, 3, &[(1, "alice"), (2, "owner")]);
        assert_eq!(results, vec!["org-1"]);
    }

    #[test]
    fn ast_query_no_matches_returns_empty() {
        let schema = make_schema("ft1", vec!["A", "B"]);
        let state = ast::Object::phi();
        let results = query_with_ast(&state, &schema, 2, &[(1, "x")]);
        assert!(results.is_empty());
    }

    #[test]
    fn ast_query_no_filter_returns_all() {
        let schema = make_schema("ft1", vec!["X", "Y"]);
        let state = state_with_facts("ft1", &[
            &[("X", "a"), ("Y", "1")],
            &[("X", "b"), ("Y", "2")],
        ]);

        let results = query_with_ast(&state, &schema, 1, &[]);
        assert_eq!(results, vec!["a", "b"]);
    }

    // ── build_query produces inspectable AST ────────────────────

    #[test]
    fn build_query_is_pure_ast() {
        // Verify the query function contains no Native nodes
        let query = build_query(2, &[(1, "alice"), (3, "active")]);
        let debug = format!("{:?}", query);
        assert!(!debug.contains("<native>"), "query must be pure AST, got: {}", debug);
        assert!(debug.contains("Filter"), "query must use Filter, got: {}", debug);
    }
}
