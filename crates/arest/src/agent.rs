// crates/arest/src/agent.rs
//
// Agent verb resolver — pure walker over the agents metamodel
// (`readings/templates/agents.md`). Given a state and a verb name,
// returns the (Model.code, Prompt) pair the registered Platform
// handler needs to fire a completion.
//
// Why a separate helper
// ---------------------
// The Platform-fn pattern (`arest::externals`) is the engine's
// install-and-call surface for any external function. For agent
// verbs specifically, the handler needs to read three FORML facts
// before calling out: which Agent Definition the Verb invokes, what
// Model that AgentDef uses, and what Prompt it carries. Putting the
// walk in the engine means every target-installed handler stays
// thin (just: parse input, call resolver, call API, record
// Completion) instead of re-implementing the cell walk.
//
// The walker is target-agnostic: same code works in the worker, in
// the kernel (once an LLM provider lands there), in the CLI. It's
// no_std-clean — only `alloc::String` + cell-graph walks via the
// engine's existing `fetch_or_phi` / `binding`.
//
// Cell-naming convention
// ----------------------
// The Stage-1 + Stage-2 parser cascade emits cells named after the
// canonical reading text with spaces replaced by underscores. So
// `Agent Definition uses Model.` produces an `Agent_Definition_uses_Model`
// cell whose facts carry `Agent Definition` and `Model` bindings.
// (Mirror of #553's `Wine_App_requires_dll_override_of_DLL_Name_with_DLL_Behavior`.)
// The walker reads those cells directly — no IR required.

#[allow(unused_imports)]
use alloc::string::{String, ToString};

use crate::ast::{binding, fetch_or_phi, Object};

/// Resolved agent-call binding: the Model.code the registered
/// Platform handler should target, plus the Prompt text from the
/// Agent Definition. Both are owned strings so the caller can ship
/// them through whatever transport (HTTP body, async closure capture)
/// without lifetime ties to the input state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentBinding {
    pub model_code: String,
    pub prompt: String,
    pub agent_definition_id: String,
}

/// Append a Completion record to `state`, mirror of the agents
/// metamodel's four facts: `Completion belongs to Agent`,
/// `Completion has input Text`, `Completion has output Text`,
/// `Completion occurred at Timestamp`. Returns the new state with
/// all four cell rows pushed; caller commits via `system::apply` (or
/// the equivalent on its target).
///
/// The Platform-fn handler (worker-installed for `extract` / `chat`)
/// calls this after the LLM call returns, so the audit trail lands
/// in the same cell graph as every other fact. From there:
///
///   * `GET /arest/completions` lists the audit log via the existing
///     HATEOAS read fallback (no extra route needed).
///   * `Citation_is_backed_by_External_System` (the existing E3
///     #305 path) ties the Completion to the model provider for
///     provenance.
///
/// The `completion_id` is caller-supplied (typically content-addressed
/// over `(agent_id, input, timestamp)` so re-emitting the same call
/// is idempotent at the cell level — `cell_push` itself doesn't
/// dedupe, but the caller can use `cell_push_unique` if needed).
pub fn record_completion(
    state: &Object,
    completion_id: &str,
    agent_id: &str,
    input_text: &str,
    output_text: &str,
    timestamp: &str,
) -> Object {
    use crate::ast::{cell_push, fact_from_pairs};

    let s = cell_push(
        "Completion_belongs_to_Agent",
        fact_from_pairs(&[("Completion", completion_id), ("Agent", agent_id)]),
        state,
    );
    let s = cell_push(
        "Completion_has_input_Text",
        fact_from_pairs(&[("Completion", completion_id), ("input Text", input_text)]),
        &s,
    );
    let s = cell_push(
        "Completion_has_output_Text",
        fact_from_pairs(&[("Completion", completion_id), ("output Text", output_text)]),
        &s,
    );
    cell_push(
        "Completion_occurred_at_Timestamp",
        fact_from_pairs(&[("Completion", completion_id), ("Timestamp", timestamp)]),
        &s,
    )
}

/// Walk `state` to find the Agent Definition that `verb_name`
/// invokes, then read its `uses Model` + `has Prompt` facts.
/// Returns `None` when:
///   * The verb isn't registered (no `Verb` cell entry by that name).
///   * The verb invokes no Agent Definition.
///   * The Agent Definition has no Model or no Prompt fact.
///
/// Callers (Platform-fn handlers, agent-aware test fixtures) treat
/// `None` as "this verb isn't an agent verb in this state" and fall
/// through to whatever non-agent handler exists.
///
/// All cell reads go through `fetch_or_phi` so missing cells
/// degrade to empty seqs cleanly — same shape as every other
/// engine walker.
pub fn resolve_agent_verb(state: &Object, verb_name: &str) -> Option<AgentBinding> {
    // 1. Verb cell — find the entry whose `name` matches.
    let verb_id = fetch_or_phi("Verb", state)
        .as_seq()?
        .iter()
        .find(|v| binding(v, "name") == Some(verb_name))
        .and_then(|v| binding(v, "id"))?
        .to_string();

    // 2. Verb_invokes_Agent_Definition — find the binding for this verb.
    let agent_def_id = fetch_or_phi("Verb_invokes_Agent_Definition", state)
        .as_seq()?
        .iter()
        .find(|f| binding(f, "Verb") == Some(&verb_id))
        .and_then(|f| binding(f, "Agent Definition"))?
        .to_string();

    // 3. Agent_Definition_uses_Model — find the binding for this agent def.
    let model_code = fetch_or_phi("Agent_Definition_uses_Model", state)
        .as_seq()?
        .iter()
        .find(|f| binding(f, "Agent Definition") == Some(&agent_def_id))
        .and_then(|f| binding(f, "Model"))?
        .to_string();

    // 4. Agent_Definition_has_Prompt — find the binding for this agent def.
    let prompt = fetch_or_phi("Agent_Definition_has_Prompt", state)
        .as_seq()?
        .iter()
        .find(|f| binding(f, "Agent Definition") == Some(&agent_def_id))
        .and_then(|f| binding(f, "Prompt"))?
        .to_string();

    Some(AgentBinding { model_code, prompt, agent_definition_id: agent_def_id })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{cell_push, fact_from_pairs, Object};

    /// Hand-stage the four cells the resolver walks: Verb,
    /// Verb_invokes_Agent_Definition, Agent_Definition_uses_Model,
    /// Agent_Definition_has_Prompt. Mirror of what the parser
    /// cascade produces from `readings/templates/agents.md` — the
    /// canonical underscored cell names + canonical role bindings.
    fn state_with_extract_agent() -> Object {
        let s = Object::phi();
        let s = cell_push(
            "Verb",
            fact_from_pairs(&[("id", "verb-extract"), ("name", "extract")]),
            &s,
        );
        let s = cell_push(
            "Verb_invokes_Agent_Definition",
            fact_from_pairs(&[
                ("Verb", "verb-extract"),
                ("Agent Definition", "agent-extractor"),
            ]),
            &s,
        );
        let s = cell_push(
            "Agent_Definition_uses_Model",
            fact_from_pairs(&[
                ("Agent Definition", "agent-extractor"),
                ("Model", "claude-sonnet-4.6"),
            ]),
            &s,
        );
        cell_push(
            "Agent_Definition_has_Prompt",
            fact_from_pairs(&[
                ("Agent Definition", "agent-extractor"),
                ("Prompt", "Extract one fact per claim. Be terse."),
            ]),
            &s,
        )
    }

    #[test]
    fn resolve_finds_full_binding_when_all_cells_present() {
        let s = state_with_extract_agent();
        let binding = resolve_agent_verb(&s, "extract").expect("verb is registered");
        assert_eq!(binding.model_code, "claude-sonnet-4.6");
        assert_eq!(binding.prompt, "Extract one fact per claim. Be terse.");
        assert_eq!(binding.agent_definition_id, "agent-extractor");
    }

    #[test]
    fn resolve_returns_none_for_unknown_verb() {
        let s = state_with_extract_agent();
        assert!(resolve_agent_verb(&s, "summarise").is_none());
    }

    #[test]
    fn resolve_returns_none_when_verb_invokes_no_agent() {
        // Verb registered but no Verb_invokes_Agent_Definition row —
        // the verb isn't an agent verb in this state.
        let s = cell_push(
            "Verb",
            fact_from_pairs(&[("id", "verb-orphan"), ("name", "orphan")]),
            &Object::phi(),
        );
        assert!(resolve_agent_verb(&s, "orphan").is_none());
    }

    #[test]
    fn resolve_returns_none_when_agent_def_has_no_model() {
        // Verb→AgentDef binding exists, but Agent_Definition_uses_Model
        // doesn't carry a row for this agent. Mirror of a partially
        // configured state — resolver must fail closed rather than
        // silently dispatch with an empty model code.
        let s = Object::phi();
        let s = cell_push(
            "Verb",
            fact_from_pairs(&[("id", "verb-incomplete"), ("name", "incomplete")]),
            &s,
        );
        let s = cell_push(
            "Verb_invokes_Agent_Definition",
            fact_from_pairs(&[
                ("Verb", "verb-incomplete"),
                ("Agent Definition", "agent-incomplete"),
            ]),
            &s,
        );
        // No Agent_Definition_uses_Model row.
        let s = cell_push(
            "Agent_Definition_has_Prompt",
            fact_from_pairs(&[
                ("Agent Definition", "agent-incomplete"),
                ("Prompt", "doesn't matter"),
            ]),
            &s,
        );
        assert!(resolve_agent_verb(&s, "incomplete").is_none());
    }

    #[test]
    fn resolve_returns_none_when_agent_def_has_no_prompt() {
        // Inverse of the previous case: Model is set but Prompt is
        // missing. Resolver fails closed for the same reason.
        let s = Object::phi();
        let s = cell_push(
            "Verb",
            fact_from_pairs(&[("id", "verb-noprompt"), ("name", "noprompt")]),
            &s,
        );
        let s = cell_push(
            "Verb_invokes_Agent_Definition",
            fact_from_pairs(&[
                ("Verb", "verb-noprompt"),
                ("Agent Definition", "agent-noprompt"),
            ]),
            &s,
        );
        let s = cell_push(
            "Agent_Definition_uses_Model",
            fact_from_pairs(&[
                ("Agent Definition", "agent-noprompt"),
                ("Model", "gpt-4o"),
            ]),
            &s,
        );
        assert!(resolve_agent_verb(&s, "noprompt").is_none());
    }

    #[test]
    fn record_completion_writes_all_four_facts() {
        // Caller supplies (agent_id, input, output, timestamp); helper
        // pushes the four canonical cell rows. After this lands the
        // existing HATEOAS read fallback can serve `/arest/completions`
        // without any extra route.
        let s = Object::phi();
        let s = record_completion(
            &s,
            "comp-001",
            "agent-extractor",
            "summarise this paragraph",
            "the paragraph asserts X.",
            "2026-04-29T12:00:00Z",
        );

        let agent = crate::ast::fetch_or_phi("Completion_belongs_to_Agent", &s);
        let agent_seq = agent.as_seq().expect("agent cell populated");
        assert_eq!(agent_seq.len(), 1);
        assert_eq!(crate::ast::binding(&agent_seq[0], "Completion"), Some("comp-001"));
        assert_eq!(crate::ast::binding(&agent_seq[0], "Agent"), Some("agent-extractor"));

        let input = crate::ast::fetch_or_phi("Completion_has_input_Text", &s);
        let input_seq = input.as_seq().expect("input cell populated");
        assert_eq!(crate::ast::binding(&input_seq[0], "input Text"),
            Some("summarise this paragraph"));

        let output = crate::ast::fetch_or_phi("Completion_has_output_Text", &s);
        let output_seq = output.as_seq().expect("output cell populated");
        assert_eq!(crate::ast::binding(&output_seq[0], "output Text"),
            Some("the paragraph asserts X."));

        let ts = crate::ast::fetch_or_phi("Completion_occurred_at_Timestamp", &s);
        let ts_seq = ts.as_seq().expect("timestamp cell populated");
        assert_eq!(crate::ast::binding(&ts_seq[0], "Timestamp"),
            Some("2026-04-29T12:00:00Z"));
    }

    #[test]
    fn record_completion_is_additive_across_calls() {
        // Two completions for the same agent — both rows land, no
        // overwrite. Mirror of how every other entity-create cell-push
        // accumulates across writes.
        let s = Object::phi();
        let s = record_completion(&s, "comp-001", "agent-a", "in1", "out1", "t1");
        let s = record_completion(&s, "comp-002", "agent-a", "in2", "out2", "t2");

        let agent = crate::ast::fetch_or_phi("Completion_belongs_to_Agent", &s);
        assert_eq!(agent.as_seq().map(|s| s.len()), Some(2));
    }

    #[test]
    fn resolve_handles_multiple_agents_in_state() {
        // Two verbs, each with its own agent. Resolver picks the
        // right one without crosstalk.
        let s = state_with_extract_agent();
        let s = cell_push(
            "Verb",
            fact_from_pairs(&[("id", "verb-chat"), ("name", "chat")]),
            &s,
        );
        let s = cell_push(
            "Verb_invokes_Agent_Definition",
            fact_from_pairs(&[
                ("Verb", "verb-chat"),
                ("Agent Definition", "agent-chatter"),
            ]),
            &s,
        );
        let s = cell_push(
            "Agent_Definition_uses_Model",
            fact_from_pairs(&[
                ("Agent Definition", "agent-chatter"),
                ("Model", "gpt-4o"),
            ]),
            &s,
        );
        let s = cell_push(
            "Agent_Definition_has_Prompt",
            fact_from_pairs(&[
                ("Agent Definition", "agent-chatter"),
                ("Prompt", "You are a friendly chatbot."),
            ]),
            &s,
        );

        let extract = resolve_agent_verb(&s, "extract").expect("extract resolves");
        assert_eq!(extract.model_code, "claude-sonnet-4.6");
        let chat = resolve_agent_verb(&s, "chat").expect("chat resolves");
        assert_eq!(chat.model_code, "gpt-4o");
        assert_eq!(chat.prompt, "You are a friendly chatbot.");
    }
}
