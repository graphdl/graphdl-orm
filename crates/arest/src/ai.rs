// crates/arest/src/ai.rs
//
// LLM completion slot — target-installable provider for the engine's
// agent dispatch (#agents-1). Sibling of `arest::entropy` (#567): a
// pure-FORML interface in the engine, target-specific provider impls
// install via a process-wide slot.
//
// Why this lives in arest, not apis
// ---------------------------------
// The agents metamodel (`readings/templates/agents.md`) defines the
// surface meta-circularly: a SYSTEM verb invokes an Agent Definition
// which uses a Model and carries a Prompt. Every deployment target
// (Cloudflare worker, kernel, CLI, WASM) walks that cell graph the
// same way; the only target-specific bit is the bytes-in / bytes-out
// transport to the actual model (CF AI Gateway binding, OpenAI HTTP,
// local llama.cpp, etc.). Centralising the trait here means:
//
//   * The engine's verb→agent→model dispatch is shared across targets.
//   * Each Completion cell is recorded by the engine, not the target;
//     the audit trail (`Completion belongs to Agent`,
//     `Completion has input/output Text`) lives where the rest of
//     state lives.
//   * Targets become thin glue: they only `install()` a provider.
//
// What's deliberately out of scope here
// -------------------------------------
// This module is the *transport* slot only. The engine code that
// walks `Verb invokes Agent Definition` → `Agent Definition uses Model`
// → `Model.code` → `arest::ai::complete(...)` lives in
// `agent_dispatch.rs` (next slice). And the prompt-template story
// (where Agent Definition's `Prompt` text lives, how it's populated
// from readings vs runtime) is a layer above. This file is just the
// install-once / call-anywhere lookup that #agents-2 / #agents-3 lean on.

#[allow(unused_imports)]
use alloc::{boxed::Box, string::{String, ToString}};

// ── Errors ──────────────────────────────────────────────────────────

/// Failure modes the completion call can surface to the engine. The
/// engine's job is to decide whether the failure becomes a Bottom
/// audit row, an HTTP 5xx, or a retried reseed — this enum carries
/// just enough taxonomy for that decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiError {
    /// No provider installed in this build / target. Kernel boots
    /// today fall here; the engine writes a Completion with an empty
    /// `output Text` and the verb returns Bottom. Workers / CLI
    /// targets install a provider during early boot.
    NotInstalled,
    /// The installed provider doesn't recognise the requested
    /// `model` code (e.g. an `OpenAI` provider asked for
    /// `claude-sonnet-4.6`). Engine policy: treat as a hard miss and
    /// emit Bottom + a violation row. Future #584 (mixing) could
    /// register multiple providers per code; one-provider-per-process
    /// is the v1 shape.
    UnsupportedModel(String),
    /// Network / IO failure inside the provider. The string is a
    /// short human-readable summary suitable for an audit row.
    Network(String),
    /// Provider returned a structured failure (rate limit, content
    /// filter, oversized prompt, etc.). String captures the
    /// provider-specific context for the audit row; engine code
    /// shouldn't pattern-match on it.
    Provider(String),
}

// ── Provider trait ──────────────────────────────────────────────────

/// A target-installed transport for LLM completions. Implementations:
///   * Cloudflare worker — `CfAiGatewayProvider` (calls
///     `env.AI.run(model, ...)` against the Workers AI binding).
///   * Kernel — none today; #584 mixing source could pull a local
///     llama.cpp WASM bake later.
///   * CLI — direct OpenAI / Anthropic HTTP via `reqwest`.
///   * Tests — `DeterministicProvider` below.
///
/// `Send + Sync` because the global slot lives behind a spin lock and
/// the same provider instance is reused across every concurrent verb
/// invocation. Implementations must be re-entrant — multiple worker
/// threads / CPU cores will call `complete` concurrently.
pub trait Provider: Send + Sync {
    /// Run one completion. Inputs:
    ///   * `model` — the `Model.code` from the Agent Definition's
    ///     `uses Model` fact (e.g. `gpt-4o`, `claude-sonnet-4.6`).
    ///   * `prompt` — the Agent Definition's `Prompt` text, used as
    ///     system / instruction prompt to the model.
    ///   * `input` — the user-supplied content (request body of the
    ///     SYSTEM verb that invoked this agent).
    ///
    /// Returns the completion text on success. Engine wraps this in a
    /// `Completion` cell with `input/output Text` + `occurred at
    /// Timestamp`. Errors surface as `AiError` per above.
    ///
    /// `&self` (not `&mut`) so the spin lock can hand out a read
    /// guard; concurrent completions don't serialise on the slot.
    fn complete(&self, model: &str, prompt: &str, input: &str)
        -> Result<String, AiError>;
}

// ── Global slot ─────────────────────────────────────────────────────

/// One provider per process. Targets install during early boot via
/// `install`; engine code calls `complete` lazily. `RwLock<Option<...>>`
/// (rather than `OnceLock`) so tests can install / replace / clear
/// across cases and so a worker re-init can swap providers without
/// recompiling.
static GLOBAL_PROVIDER: spin::RwLock<Option<Box<dyn Provider>>> =
    spin::RwLock::new(None);

/// Install the process-wide LLM provider. Targets call this exactly
/// once during early boot (after configuration is loaded so the
/// provider can capture API keys / Workers AI bindings / local model
/// handles). Subsequent calls REPLACE the previously installed
/// provider — production paths must avoid this; tests use it to swap
/// a `DeterministicProvider` in for reproducible output.
pub fn install(provider: Box<dyn Provider>) {
    *GLOBAL_PROVIDER.write() = Some(provider);
}

/// Clear the installed provider. Used by tests that want the
/// `NotInstalled` branch to fire on the next `complete` call.
/// Production paths never uninstall.
pub fn uninstall() {
    *GLOBAL_PROVIDER.write() = None;
}

/// `true` when a provider is installed. Engine code uses this to
/// decide between calling `complete` and short-circuiting to a
/// `NotInstalled` Bottom row without touching the lock.
pub fn is_installed() -> bool {
    GLOBAL_PROVIDER.read().is_some()
}

/// Run a completion against the installed provider. Engine glue
/// (verb dispatch — landing in #agents-2) calls this once it has
/// resolved the Verb→AgentDef→Model chain into `(model, prompt,
/// input)`. Returns `Err(AiError::NotInstalled)` when no provider is
/// installed (kernel default), so the caller can record an empty
/// `Completion` cell instead of panicking.
pub fn complete(model: &str, prompt: &str, input: &str) -> Result<String, AiError> {
    let guard = GLOBAL_PROVIDER.read();
    let provider = guard.as_ref().ok_or(AiError::NotInstalled)?;
    provider.complete(model, prompt, input)
}

// ── Test fixture: deterministic provider ────────────────────────────

/// Reproducible provider for tests. Echoes a canned response so the
/// engine's verb→agent→model→complete dispatch can be exercised
/// without a real LLM. Mirror of `entropy::DeterministicSource`'s
/// shape so the install / call / assert pattern is identical.
#[cfg(test)]
pub struct DeterministicProvider {
    pub response: String,
    pub last_model: spin::Mutex<String>,
    pub last_prompt: spin::Mutex<String>,
    pub last_input: spin::Mutex<String>,
}

#[cfg(test)]
impl DeterministicProvider {
    pub fn new(response: &str) -> Self {
        Self {
            response: response.to_string(),
            last_model: spin::Mutex::new(String::new()),
            last_prompt: spin::Mutex::new(String::new()),
            last_input: spin::Mutex::new(String::new()),
        }
    }
}

#[cfg(test)]
impl Provider for DeterministicProvider {
    fn complete(&self, model: &str, prompt: &str, input: &str) -> Result<String, AiError> {
        *self.last_model.lock() = model.to_string();
        *self.last_prompt.lock() = prompt.to_string();
        *self.last_input.lock() = input.to_string();
        Ok(self.response.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each test installs / uninstalls under this serial guard so
    /// concurrent test runners don't see a partial install state.
    /// `entropy.rs` uses the same pattern.
    static SERIAL: spin::Mutex<()> = spin::Mutex::new(());

    #[test]
    fn complete_returns_not_installed_when_no_provider() {
        let _g = SERIAL.lock();
        uninstall();
        assert!(!is_installed());
        assert_eq!(
            complete("gpt-4o", "you are helpful", "hi"),
            Err(AiError::NotInstalled),
        );
    }

    #[test]
    fn complete_succeeds_with_deterministic_provider() {
        let _g = SERIAL.lock();
        install(Box::new(DeterministicProvider::new("the answer is 42")));
        assert!(is_installed());
        let out = complete("gpt-4o", "be terse", "what is the answer?")
            .expect("provider installed");
        assert_eq!(out, "the answer is 42");
        uninstall();
    }

    #[test]
    fn provider_records_inputs_for_later_assert() {
        let _g = SERIAL.lock();
        let provider = Box::new(DeterministicProvider::new("ok"));
        // Stash a raw pointer to the provider so we can read the
        // recorded inputs after install (which moves ownership into
        // the slot). Tests-only — production code should never reach
        // for the slot's contents this way.
        let provider_ptr = &*provider as *const DeterministicProvider;
        install(provider);
        let _ = complete("claude-sonnet-4.6", "system prompt", "user input");
        // SAFETY: the slot still holds the Box; Box::leak-style
        // address stability lets the raw pointer remain valid for
        // this test's lifetime.
        let recorded = unsafe { &*provider_ptr };
        assert_eq!(*recorded.last_model.lock(), "claude-sonnet-4.6");
        assert_eq!(*recorded.last_prompt.lock(), "system prompt");
        assert_eq!(*recorded.last_input.lock(), "user input");
        uninstall();
    }

    #[test]
    fn install_replaces_existing_provider() {
        let _g = SERIAL.lock();
        install(Box::new(DeterministicProvider::new("first")));
        assert_eq!(complete("any", "any", "any"), Ok("first".to_string()));
        install(Box::new(DeterministicProvider::new("second")));
        assert_eq!(complete("any", "any", "any"), Ok("second".to_string()));
        uninstall();
    }

    #[test]
    fn uninstall_clears_slot() {
        let _g = SERIAL.lock();
        install(Box::new(DeterministicProvider::new("ok")));
        assert!(is_installed());
        uninstall();
        assert!(!is_installed());
        assert_eq!(complete("any", "any", "any"), Err(AiError::NotInstalled));
    }

    #[test]
    fn ai_error_supports_equality_for_audit_assertions() {
        // Engine code matches on AiError variants when deciding how
        // to record the Completion row. PartialEq + Clone keep that
        // ergonomic — assert_eq! works in tests, and the engine can
        // clone an error into a Bottom audit row without consuming it.
        let e = AiError::UnsupportedModel("local-only".to_string());
        let cloned = e.clone();
        assert_eq!(e, cloned);
        assert_ne!(e, AiError::Network("timeout".to_string()));
    }
}
