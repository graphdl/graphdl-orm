// crates/arest/src/externals.rs
//
// Doc + worked example for the canonical "external function in DEFS"
// pattern. All machinery lives in `ast.rs`; this module is a single
// place to point at when adding a new external integration (LLM,
// HTTP API, hardware sensor, …).
//
// The pattern in one paragraph
// ----------------------------
// External calls are just `Func::Platform(name)` entries in DEFS. The
// engine treats them like any other binding — `apply(Func::Def(name),
// input, D)` runs the registered handler. Targets register handlers
// at boot via `install_platform_fn` (sync) or `install_async_platform_fn`
// (async). When no handler is installed, `apply` returns `Object::Bottom`
// — graceful skip, not a panic. Citations for provenance come for free
// via `emit_citation_fact` with `Authority Type = 'Runtime-Function'`
// (or `'Federated-Fetch'` when the External System backing the call
// is also recorded).
//
// Why no separate "AI provider" or "HTTP client" trait
// ----------------------------------------------------
// LLMs, HTTP APIs, sensor reads, message queues, hardware RNG,
// payment processors — all are *external functions*: a name in DEFS
// with a target-installed body. Carving each into its own trait +
// global slot duplicated machinery and obscured the symmetry. The
// earlier `arest::ai` module was such a duplicate; it was removed in
// the same commit that added this doc, in favour of the unified
// pattern below.
//
// What about `External System`?
// -----------------------------
// `External System` (declared in `readings/core.md`) keeps its narrow
// role: provenance + auth metadata for callers. It records WHO is
// being called (`Stripe`, `OpenAI`, `Cloudflare AI Gateway`) and
// holds URL / Header / Prefix / Kind for the per-system auth shape.
// The mechanics of MAKING the call are independent — a verb invokes
// `Func::Platform("ai_complete")`, and the handler the worker
// registered for `ai_complete` may consult an `External System` cell
// to pick the right base URL + auth header. Two separate concerns,
// one mechanism each.
//
// Worked example: an LLM-shaped external function
// -----------------------------------------------
// 1. Declare the verb in a reading (e.g. `readings/templates/agents.md`):
//
//        Verb invokes Agent Definition.
//          Each Verb invokes at most one Agent Definition.
//
//    The Agent Definition carries `Name`, `uses Model`, and `has
//    Prompt` — that's the metadata the handler needs. The verb name
//    becomes the Platform-fn key (e.g. `extract`, `chat`,
//    `summarize`).
//
// 2. At engine boot, register the verb in DEFS as a Platform fn:
//
//        let d = register_runtime_fn(
//            "extract",
//            Func::Platform("extract".to_string()),
//            &state,
//        );
//
//    No body yet — `apply(Func::Def("extract"), input, d)` returns
//    `Object::Bottom`. That's the kernel's default state today.
//
// 3. On the worker target, install the body:
//
//        install_async_platform_fn("extract", Arc::new(|input, d| {
//            Box::pin(async move {
//                // 1. Walk D for the Agent Definition that the
//                //    `extract` verb invokes (Verb→AgentDef).
//                // 2. Walk D for its Model (AgentDef→Model.code) +
//                //    Prompt (AgentDef→Prompt).
//                // 3. Call the actual LLM (env.AI on CF Workers,
//                //    or HTTP POST to the model's endpoint).
//                // 4. cell_push the resulting Completion fact onto
//                //    D — input/output/timestamp + belongs-to-Agent.
//                // 5. Return the parsed output as an Object so the
//                //    caller can use it as the verb's result.
//                Object::atom(&completion_text)
//            })
//        }));
//
// 4. The kernel target installs nothing. `apply(Func::Def("extract"),
//    ...)` returns Bottom; the HTTP handler emits a 404 (or a
//    Bottom-shaped envelope) without panicking. Same dispatch path,
//    target-specific reach.
//
// What this replaces
// ------------------
// The earlier `arest::ai::{Provider, install, complete}` slot was
// exactly the above pattern with a more specialised trait. Removing
// it folds LLMs into the existing surface and frees the next external
// integration (Stripe, GitHub API, schema.org fetch) to follow the
// same recipe — one fewer module per integration.
//
// All identifiers re-exported below come from `ast.rs`; this module
// adds zero new public surface.

// Re-exports of the *public* registration surface. The dispatch
// entry points (`apply_platform`, `apply_platform_async`) are
// engine-internal — callers never invoke them directly; they go
// through `apply(Func::Def(name), input, d)` and the engine routes.
#[allow(unused_imports)]
pub use crate::ast::{
    install_platform_fn,
    register_runtime_fn,
    uninstall_platform_fn,
};

#[cfg(not(feature = "no_std"))]
#[allow(unused_imports)]
pub use crate::ast::{
    emit_citation_fact,
    install_async_platform_fn,
    uninstall_async_platform_fn,
};

#[cfg(test)]
mod tests {
    use crate::ast::{
        apply, install_platform_fn, register_runtime_fn,
        uninstall_platform_fn, Func, Object,
    };
    use alloc::string::ToString;

    /// Worked example mirroring the LLM-shaped use that the deleted
    /// `arest::ai` module previously hand-rolled. The engine surface
    /// — register a Platform-fn name, install a body, call via
    /// `apply(Func::Def(name), ...)` — is identical to every other
    /// external integration. No new trait, no new global slot, no
    /// new test fixture.
    #[test]
    fn external_llm_call_round_trips_through_func_platform() {
        // 1. Boot phase: register the verb name in DEFS.
        let d = register_runtime_fn(
            "extract_test",
            Func::Platform("extract_test".to_string()),
            &Object::phi(),
        );

        // 2. Worker boot phase: install the body. In production this
        //    would call out to CF AI Gateway / OpenAI / a local model;
        //    here it echoes a canned response so the test is
        //    deterministic.
        install_platform_fn(
            "extract_test",
            crate::sync::Arc::new(|input: &Object, _d: &Object| {
                let prompt_text = input.as_atom().unwrap_or("");
                Object::atom(&alloc::format!("LLM(echo): {}", prompt_text))
            }),
        );

        // 3. Engine call site (the agent-dispatch code path that
        //    walks Verb→AgentDef→Model→Platform) reaches us here.
        let result = apply(
            &Func::Def("extract_test".to_string()),
            &Object::atom("summarise this"),
            &d,
        );
        uninstall_platform_fn("extract_test");

        assert_eq!(result, Object::atom("LLM(echo): summarise this"));
    }

    /// Mirror of the kernel default: name registered, no body
    /// installed → caller sees `Bottom`. Documents the graceful-skip
    /// semantics that make per-target opt-in safe (kernel can leave
    /// the slot empty without crashing every `/arest/extract`).
    #[test]
    fn unconfigured_external_returns_bottom_not_panic() {
        let d = register_runtime_fn(
            "extract_no_body_test",
            Func::Platform("extract_no_body_test".to_string()),
            &Object::phi(),
        );
        let result = apply(
            &Func::Def("extract_no_body_test".to_string()),
            &Object::atom("any input"),
            &d,
        );
        assert_eq!(result, Object::Bottom);
    }
}
