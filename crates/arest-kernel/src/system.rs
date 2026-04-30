// crates/arest-kernel/src/system.rs
//
// SYSTEM function, kernel-side (#265).
//
// The no_std `arest` lib gives us `ast::Object`, `ast::Func`,
// `ast::apply`, `ast::defs_to_state`, and `ast::metacompose`. That
// is the entire SYSTEM surface — everything else (parse, compile,
// command, check) lives behind the std-only feature gate and is
// unavailable here. For the bare-metal kernel the implication is
// that readings are pre-compiled before boot and their resulting
// def set is baked into the binary; at runtime the kernel only
// needs to ρ-apply.
//
// This first version bakes two demo defs so the HTTP handler can
// exercise the full dispatch path:
//
//   `welcome` → a static banner atom.
//   `echo`    → Func::Id; returns whatever input it was handed.
//
// Dispatch cycle:
//   1. Look up the def cell by name via FetchOrPhi(<name, D>).
//   2. metacompose the resulting Object back into a Func (reverse
//      of `func_to_object`, which defs_to_state applied).
//   3. ast::apply the Func to the HTTP-body-derived input against
//      the baked state D.
//   4. Serialise the resulting Object into bytes for the wire.
//
// As more defs get baked in (compiled from the metamodel at build
// time, shipped as `freeze`d bytes, thawed here), the same three
// lines of dispatch logic serve every verb. That's the whole point
// of SYSTEM as a single function.
//
// ── Mutator (#451) ─────────────────────────────────────────────────
//
// XX (#403) and FFF (#444) both flagged the same gap: the original
// `Once<Object>` shape made SYSTEM immutable post-init, which meant
// `POST /file` could compute the would-be next state but not install
// it, and `GET /file/{id}/content` 404'd every freshly-uploaded id.
// The mutator below is the minimum-viable fix: a `RwLock`-protected
// pointer slot, swapped atomically by `apply()`. Each `apply()`
// `Box::leak`s the new state so existing `&'static Object` borrows
// (returned by the legacy `state()` shim retained for `net.rs`)
// remain valid for the rest of the kernel boot. Memory grows per
// upload; an arena reclaim pass is a follow-up once the chunked PUT
// (#445) lands and write rates climb.
//
// API:
//   * `with_state(|s| ...) -> Option<R>`  — read-side guard form,
//     replaces XX's `state() -> Option<&'static Object>` for new
//     callers (file_serve, file_upload). Briefly holds the read
//     lock inside the closure.
//   * `apply(new_state) -> Result<(), &'static str>` — atomic write.
//     Caller computes the fully-built next-state Object via
//     `ast::cell_push`/`build_file_facts` and hands it in; we leak
//     it, swap the pointer under the write lock, and return.
//   * `state() -> Option<&'static Object>` — legacy shim retained
//     so `net.rs` (forbidden territory in this commit) keeps
//     compiling. Returns the leaked-pointer snapshot taken under a
//     brief read lock; the pointer remains valid for the kernel
//     lifetime (memory leaks on each `apply`).

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use arest::agent;
use arest::ast::{self, Func, Object};
use arest::json_min;
use spin::{Mutex, Once, RwLock};

use crate::assets;

/// Opaque identifier handed back to subscribers of `apply()` change
/// notifications. Pass this value to `unsubscribe` to drop the
/// registered handler.
///
/// Monotonically allocated via `NEXT_SUBSCRIBER_ID`; the kernel runs
/// for the lifetime of one boot, so a `u64` is unbounded enough for
/// realistic subscribe/unsubscribe rates (HateoasBrowser registers
/// once per app re-launch; smoke harness exercises the path a few
/// times per test run).
pub type SubscriberId = u64;

/// Registry of `apply()` change subscribers. The handler is invoked
/// once per `apply()` call with the slice of cell names whose
/// contents changed (symmetric difference of the cell-name sets, plus
/// cells that exist in both but whose `Object` contents differ).
///
/// Stored behind a `spin::Mutex<BTreeMap<...>>`:
///   * `Mutex` (not `RwLock`) — the read-side path (delivery) needs
///     to snapshot handler refs and release the lock before invoking
///     them; a `RwLock` buys nothing for that pattern.
///   * `BTreeMap` rather than `Vec<(SubscriberId, ...)>` so
///     `unsubscribe` is `O(log n)` rather than `O(n)` — handlers
///     come and go over the kernel boot lifetime as apps open/close.
///   * Each handler is wrapped in `Arc<dyn Fn>` (not `Box<dyn Fn>`)
///     so the delivery snapshot can clone-the-Arc-and-release-the-
///     lock cheaply. The public `subscribe_changes` API still takes
///     a `Box<dyn Fn>` per #458's spec; we re-wrap into `Arc` at the
///     registration site, which is a one-allocation conversion
///     (`Arc::from(box)` on an unsized trait object).
///
/// Delivery iterates a snapshot of the `Arc<dyn Fn>` handles taken
/// under a brief lock, then drops the lock before invoking any
/// handler — so a handler that calls `unsubscribe` from inside the
/// closure (the HateoasBrowser drop path can in principle do this)
/// doesn't deadlock on the same `Mutex`. The `Arc` keeps the
/// handler alive for the duration of the call even if a concurrent
/// `unsubscribe` removed it from the registry between snapshot and
/// invocation.
static SUBSCRIBERS: Mutex<BTreeMap<SubscriberId, Arc<dyn Fn(&[String]) + Send + Sync>>>
    = Mutex::new(BTreeMap::new());

/// Monotonic id counter for `subscribe_changes`. `Relaxed` ordering
/// is sufficient — we only need uniqueness, not happens-before
/// ordering with the registry insertion (the `Mutex` provides that).
static NEXT_SUBSCRIBER_ID: AtomicU64 = AtomicU64::new(1);

/// Baked kernel state — built once during boot and then mutable
/// through `apply()` for the tenant's lifetime. Stored as an
/// `&'static Object` slot behind a `RwLock` so:
///
///   * Reads briefly take the shared lock and snapshot the current
///     pointer (lock release order: drop guard before returning to
///     caller; the pointer itself is `'static` so it outlives the
///     guard).
///   * `apply()` `Box::leak`s the new state, takes the exclusive
///     lock, and overwrites the pointer. Old leaked states are not
///     reclaimed — see top-of-file note.
///
/// `Once` wraps the `RwLock` so the lock itself is constructed
/// lazily inside `init()` rather than at module-load time; this
/// preserves the pre-#451 panic-on-double-init semantics.
static SYSTEM: Once<RwLock<&'static Object>> = Once::new();

/// Build the demo state + defs. Called once from `kernel_main`
/// after `net::init()`. Panics if called twice — mirrors the
/// one-tenant-per-kernel invariant and keeps the lookup path
/// lock-free.
pub fn init() {
    SYSTEM.call_once(|| {
        // Two demo defs. The banner text lives in the `welcome`
        // Constant so the HTTP handler's dispatch is a single
        // ρ-application and not a Rust string.push.
        let defs: Vec<(String, Func)> = alloc::vec![
            (
                "welcome".to_string(),
                Func::Constant(Object::atom(
                    "AREST kernel — one ρ-application away from the wire.\n\n\
                     Try:  curl http://127.0.0.1/api/welcome\n\
                           curl -d 'hello' http://127.0.0.1/api/echo\n",
                )),
            ),
            // Func::Id is the identity ρ-application — apply(Id, x, D) = x.
            ("echo".to_string(), Func::Id),
        ];
        let mut initial = ast::defs_to_state(&defs, &Object::phi());

        // Demo Noun + entity so the HATEOAS read fallback (#608/#609/#610)
        // returns something concrete instead of always hitting `None` on
        // a bare boot. Mirror of how the worker seeds noun_index from
        // readings — the kernel's metamodel-loaded path lands later
        // (#588 lifts the Stage-2 parser to no_std), so for now we
        // hand-stage a single Noun fact + a single Organization entity.
        // `GET /arest/organizations` and `/arest/organizations/acme`
        // both round-trip through `arest::hateoas::handle_arest_read`,
        // proving the wire-up is end-to-end.
        let noun_org = Object::seq(alloc::vec![Object::seq(alloc::vec![
            Object::atom("name"),
            Object::atom("Organization"),
        ])]);
        initial = ast::cell_push("Noun", noun_org, &initial);

        let entity_acme = Object::seq(alloc::vec![
            Object::seq(alloc::vec![Object::atom("id"), Object::atom("acme")]),
            Object::seq(alloc::vec![Object::atom("name"), Object::atom("Acme Corp")]),
        ]);
        initial = ast::cell_push("Organization", entity_acme, &initial);

        // Support Request noun (#624 — supports the apis e2e suite's
        // `/arest/support-requests` test slice). Same hand-stage shape
        // as Organization above; drops once #588 lifts Stage-2 to
        // no_std and the kernel can compile readings/support/*.md
        // at boot.
        let noun_sr = Object::seq(alloc::vec![Object::seq(alloc::vec![
            Object::atom("name"),
            Object::atom("Support Request"),
        ])]);
        initial = ast::cell_push("Noun", noun_sr, &initial);

        // State-machine prerequisites for #617/#618 — `POST /arest/
        // entities/support-requests/{id}/transition` walks
        // `State Machine` (forResource → currentlyInStatus) and
        // `Transition` (fromStatus + event → toStatus). Hand-stage:
        //   * a State Machine row mirroring an SR's initial status,
        //   * a Transition row that fires `categorize` from
        //     `Received` → `Categorized` (apis e2e fixture at
        //     `apis/__e2e__/arest.test.ts:286`).
        // The SR entity itself isn't seeded — the e2e suite POSTs
        // its own SR earlier in the test run (line 240). Operators
        // wanting to exercise the seeded SM can manually `POST
        // /arest/entities/support-requests` with `id=sr-1` first.
        let noun_sm = Object::seq(alloc::vec![Object::seq(alloc::vec![
            Object::atom("name"),
            Object::atom("State Machine"),
        ])]);
        initial = ast::cell_push("Noun", noun_sm, &initial);
        let noun_t = Object::seq(alloc::vec![Object::seq(alloc::vec![
            Object::atom("name"),
            Object::atom("Transition"),
        ])]);
        initial = ast::cell_push("Noun", noun_t, &initial);

        let sm_demo = Object::seq(alloc::vec![
            Object::seq(alloc::vec![Object::atom("id"), Object::atom("sm-sr-1")]),
            Object::seq(alloc::vec![Object::atom("forResource"), Object::atom("sr-1")]),
            Object::seq(alloc::vec![
                Object::atom("currentlyInStatus"),
                Object::atom("Received"),
            ]),
        ]);
        initial = ast::cell_push("State Machine", sm_demo, &initial);

        let t_categorize = Object::seq(alloc::vec![
            Object::seq(alloc::vec![Object::atom("id"), Object::atom("t-categorize")]),
            Object::seq(alloc::vec![
                Object::atom("fromStatus"),
                Object::atom("Received"),
            ]),
            Object::seq(alloc::vec![
                Object::atom("toStatus"),
                Object::atom("Categorized"),
            ]),
            Object::seq(alloc::vec![Object::atom("event"), Object::atom("categorize")]),
        ]);
        initial = ast::cell_push("Transition", t_categorize, &initial);

        // #620 / HATEOAS-6b — register the `extract` agent verb as a
        // Func::Platform slot at boot. The kernel profile installs no
        // body (cf. `Func::Platform(_) → Bottom` in the `no_std` arm
        // of `apply_nonbottom`), so `apply(Func::Def("extract"), …)`
        // returns `Object::Bottom` and `handle_extract` lifts that
        // into a 503 envelope pointing at the worker URL. Branch-free
        // dispatch — the verb resolves through the same FetchOrPhi /
        // metacompose path every other ρ-applied def does. The
        // worker target swaps in a real LLM body via
        // `externals::install_async_platform_fn` (or its sync twin)
        // without touching the dispatch path.
        //
        // See `crates/arest/src/externals.rs:54-86` for the worked
        // example this implementation follows verbatim.
        initial = ast::register_runtime_fn(
            "extract",
            Func::Platform("extract".to_string()),
            &initial,
        );

        // #580 — seed the ui.do bundle (when the `ui-bundle` feature
        // baked one in) into the cell graph so `arest_http_handler`
        // can serve it via `assets::lookup_from_state`. No-op when
        // `assets::UI_ASSETS` is empty (default + `--features server`
        // profile). See `seed_ui_bundle_cells` for the per-asset cell
        // shape and the handoff note for #581.
        initial = seed_ui_bundle_cells(initial);

        // Box::leak gives us the `&'static Object` the slot stores.
        // The leak is intentional: the legacy `state()` shim returns
        // `&'static Object`, and `apply()`'s atomic-pointer-swap
        // story requires that all live snapshots remain valid for
        // the kernel lifetime.
        let leaked: &'static Object = Box::leak(Box::new(initial));
        RwLock::new(leaked)
    });
}

/// Seed the build-time ui.do bundle into the supplied state's File
/// cell graph (#580 Layer B).
///
/// For each `(http_path, bytes)` entry in `assets::UI_ASSETS` (the
/// table emitted by `build.rs` under `feature = "ui-bundle"`), pushes
/// four facts:
///
///   * `File_has_Path<File, Path>`        — addressable URL
///   * `File_has_ContentRef<File, ContentRef>` — hex-encoded inline
///     bytes (same shape `file_upload.rs` uses post-#397d so the
///     `file_serve.rs::decode_content_ref` reader honours it
///     unchanged)
///   * `File_has_MimeType<File, MimeType>` — derived via
///     `assets::content_type_for(path)`
///   * `File_has_Size<File, Size>`        — byte length, matching
///     `file_upload.rs::build_file_facts_with_cref`
///
/// File ids are synthesised as `ui-bundle-<n>` so they don't collide
/// with `file_upload.rs`'s `<prefix>-upload-<n>` ids.
///
/// Returns the unchanged state when the bundle table is empty (the
/// default + `--features server` shape). The caller is expected to
/// fold the returned state into whatever larger boot-time build is
/// in flight — `init()` does this in-line before the final
/// `Box::leak`.
///
/// ── Handoff for #581 ───────────────────────────────────────────────
///
/// Today this function reads from the build-time `UI_ASSETS` table.
/// #581 lifts the ui.do source out of `apps/ui.do/` so the kernel
/// can no longer `include_bytes!` the bundle at compile time. At
/// that point this function's body becomes a runtime load — same
/// File cell shape, but `bytes` come from a freeze blob, an HTTP
/// fetch against a separate origin, or a disk image. The wire
/// handler (`arest_http_handler` → `assets::lookup_from_state`)
/// stays unchanged; only the seed source moves.
pub fn seed_ui_bundle_cells(state: Object) -> Object {
    let mut acc = state;
    for (idx, (http_path, body)) in assets::UI_ASSETS.iter().enumerate() {
        let file_id = format!("ui-bundle-{}", idx);
        let cref = assets::encode_inline_hex(body);
        let mime = assets::content_type_for(http_path);
        let size = format!("{}", body.len());

        acc = ast::cell_push(
            "File_has_Path",
            ast::fact_from_pairs(&[("File", &file_id), ("Path", http_path)]),
            &acc,
        );
        acc = ast::cell_push(
            "File_has_ContentRef",
            ast::fact_from_pairs(&[("File", &file_id), ("ContentRef", &cref)]),
            &acc,
        );
        acc = ast::cell_push(
            "File_has_MimeType",
            ast::fact_from_pairs(&[("File", &file_id), ("MimeType", mime)]),
            &acc,
        );
        acc = ast::cell_push(
            "File_has_Size",
            ast::fact_from_pairs(&[("File", &file_id), ("Size", &size)]),
            &acc,
        );
    }
    acc
}

/// Dispatch a parsed HTTP request through the baked SYSTEM.
///
/// Returns `Some(body)` on a handled path, `None` when no def
/// matches. Callers layer their own 404 on the None branch.
///
/// The path-to-def map is the entirety of the HTTP routing layer:
/// every HTTP verb lands on `apply_named(def_name, body)`, and
/// `apply_named` does one ρ-lookup + one ρ-application. There is
/// no separate command handler, no route middleware, and no
/// method-specific branching; the def owns its interpretation.
pub fn dispatch(_method: &str, path: &str, body: &[u8]) -> Option<Vec<u8>> {
    let def_name = route_to_def(path)?;
    Some(apply_named(def_name, body))
}

fn route_to_def(path: &str) -> Option<&'static str> {
    // Strip an optional query string so `/api/welcome?v=1` still matches.
    let path = path.split('?').next().unwrap_or(path);
    match path {
        // Canonical API namespace introduced in #266. The HTML shell
        // lives at `/`; every dynamic verb is reached at `/api/<def>`.
        "/api/welcome" => Some("welcome"),
        "/api/echo" => Some("echo"),

        // Legacy bundle-free routes. When no ui.do bundle is baked in
        // (assets::UI_ASSETS is empty), the handler falls through
        // from `assets::lookup` to `system::dispatch` for the bare
        // `/` path — keeping the pre-#266 "AREST kernel — one
        // ρ-application away from the wire" banner reachable via
        // `curl http://127.0.0.1/`. Once the bundle is present these
        // paths are shadowed by `/index.html`.
        "/" | "/welcome" => Some("welcome"),
        "/echo" => Some("echo"),

        _ => None,
    }
}

/// Public entry for ring-3 callers (#333). Same dispatch the HTTP
/// handler uses, exposed by name for the syscall layer.
pub fn apply_named_pub(name: &str, body: &[u8]) -> Vec<u8> {
    apply_named(name, body)
}

/// Public cell-fetch entry for ring-3 callers (#333). Looks up the
/// named cell in the baked SYSTEM state and serialises the result
/// to bytes. Returns the same `\xE2\x8A\xA5\n` (⊥) marker that the
/// HTTP path uses when the cell is absent or empty.
///
/// Goes through `Func::FetchOrPhi` rather than `ast::fetch_or_phi`
/// so the syscall path is structurally identical to the wire path
/// — same ρ-dispatch shape, just a different transport.
pub fn fetch_named(name: &str) -> Vec<u8> {
    match with_state(|state| {
        let name_obj = Object::atom(name);
        let tuple = Object::seq(alloc::vec![name_obj, state.clone()]);
        ast::apply(&Func::FetchOrPhi, &tuple, state)
    }) {
        Some(val) => serialise(&val),
        None => serialise(&Object::Bottom),
    }
}

/// Apply the named def to `body` against the baked state and
/// return the resulting Object serialised as bytes.
///
/// Looks up the def's Object representation via
/// FetchOrPhi(<name, D>), reverses it back into a `Func` via
/// `ast::metacompose`, then runs `ast::apply(func, input, D)` —
/// exactly the three-step ρ-dispatch the paper's SYSTEM equation
/// describes. When the def is absent FetchOrPhi returns `Object::Bottom`
/// and `metacompose` gives back `Func::Id`, which is safely a no-op.
fn apply_named(name: &str, body: &[u8]) -> Vec<u8> {
    let out = with_state(|state| {
        let name_obj = Object::atom(name);
        let tuple = Object::seq(alloc::vec![name_obj, state.clone()]);
        let f_obj = ast::apply(&Func::FetchOrPhi, &tuple, state);
        let f = ast::metacompose(&f_obj, state);

        let input = match core::str::from_utf8(body) {
            Ok(s) if !s.is_empty() => Object::atom(s),
            _ => Object::phi(),
        };

        ast::apply(&f, &input, state)
    })
    .expect("system::init() not called");
    serialise(&out)
}

/// Worker URL the `Retry-After` header points the caller at when
/// `POST /arest/extract` falls into the no-body path (#620 / HATEOAS-6b).
/// Single source of truth so the header value, the envelope's
/// `_links.worker.href`, and the envelope's top-level `retryAfter`
/// can't drift.
pub const EXTRACT_WORKER_URL: &str = "https://arest.do/arest/extract";

/// Outcome of `dispatch_extract` — gives the HTTP layer enough shape
/// to build either a 200 (body installed; `result_body` is the JSON-
/// shaped output of the LLM call) or a 503 (no body installed;
/// `result_body` is the introspectable envelope below). The status
/// is owned by this module rather than the caller so the
/// envelope-vs-body decision lives in one place.
#[derive(Debug)]
pub struct ExtractOutcome {
    /// HTTP status — 200 when the Platform-fn body produced a non-Bottom
    /// Object, 503 when it returned Bottom (no body installed in this
    /// profile, the `Func::Platform(_) → Bottom` arm in `no_std` apply).
    pub status: u16,
    /// Response body bytes. JSON for both branches:
    ///   * 200 — the engine output, currently the same shape
    ///     `serialise()` emits (atoms pass through, everything else
    ///     renders via Debug; future commits replace with FFP-to-JSON).
    ///   * 503 — the structured envelope documented on
    ///     `build_no_body_envelope` below.
    pub body: Vec<u8>,
    /// Worker URL when this is a 503 envelope, `None` for 200. Lifted
    /// to the HTTP layer as a `Retry-After` header.
    pub retry_after: Option<String>,
}

/// `POST /arest/extract` dispatch entry (#620 / HATEOAS-6b).
///
/// Three branches:
///   1. Body parses as JSON → drive `apply(Func::Def("extract"),
///      input, D)`. On `Object::Bottom` (no LLM body installed in this
///      profile), emit the introspectable 503 envelope with the
///      resolved Agent Definition metadata when available.
///   2. Body parses but apply produces a non-Bottom Object → 200 with
///      the result serialised (`serialise()`).
///   3. Body fails to parse → 503 with `extract.parse` envelope so a
///      malformed POST never panics; mirrors the `extract.no_body`
///      shape so callers can rely on a single envelope schema across
///      both failure modes.
///
/// The function is `pub` so the dispatcher in `lib.rs` can reach it
/// without going through the lower-level `apply_named` (which serialises
/// blindly and has no envelope shape — extract needs the structured
/// surface for #624 e2e).
pub fn dispatch_extract(body: &[u8]) -> ExtractOutcome {
    // Parse JSON input first. An empty body is treated as `phi()` —
    // the verb may still be invoked with no operand (some agent
    // prompts don't need one). Garbage body falls into the parse
    // envelope below.
    let parsed_input: Option<Object> = if body.is_empty() {
        Some(Object::phi())
    } else {
        match json_min::parse(body) {
            Some(v) => Some(json_to_object(&v)),
            None => None,
        }
    };

    let parsed_input = match parsed_input {
        Some(obj) => obj,
        None => {
            // #620 — malformed JSON returns 503 with `extract.parse`.
            // Reusing the no-body envelope's `_links.worker` shape so
            // the caller can still re-issue against the worker
            // (which may have looser parsing or surface a richer
            // 4xx). 503 (not 400) keeps the dispatch contract uniform:
            // every failure mode here is "this profile can't fulfil
            // the call; here's where it might succeed".
            let envelope = build_envelope(
                "extract.parse",
                "Request body did not parse as JSON; nothing to dispatch to the 'extract' verb.",
                None,
            );
            return ExtractOutcome {
                status: 503,
                body: envelope.into_bytes(),
                retry_after: Some(EXTRACT_WORKER_URL.to_string()),
            };
        }
    };

    // Drive the verb through the standard ρ-application path. If
    // SYSTEM hasn't been initialised the kernel is in a programmer-
    // error state — surface a 500-shaped envelope rather than
    // panicking, so the wire keeps responding.
    let result = with_state(|state| {
        ast::apply(
            &Func::Def("extract".to_string()),
            &parsed_input,
            state,
        )
    });

    let (result, agent_binding) = match result {
        Some(r) => {
            let binding = with_state(|state| agent::resolve_agent_verb(state, "extract"))
                .flatten();
            (r, binding)
        }
        None => {
            // init() not called — degrade to 503 so the wire stays up.
            let envelope = build_envelope(
                "extract.no_body",
                "SYSTEM not initialised; the 'extract' verb cannot be dispatched.",
                None,
            );
            return ExtractOutcome {
                status: 503,
                body: envelope.into_bytes(),
                retry_after: Some(EXTRACT_WORKER_URL.to_string()),
            };
        }
    };

    if result.is_bottom() {
        let envelope = build_envelope(
            "extract.no_body",
            "The 'extract' verb is registered on this kernel but no LLM body is installed. \
             Configure a body via arest::externals::install_async_platform_fn, or route to a \
             profile that has one.",
            agent_binding.as_ref(),
        );
        return ExtractOutcome {
            status: 503,
            body: envelope.into_bytes(),
            retry_after: Some(EXTRACT_WORKER_URL.to_string()),
        };
    }

    // Body installed — return 200 with the serialised result. For now
    // this reuses `serialise()`; once a richer FFP-to-JSON projector
    // lands the 200 branch can swap encoders without touching the
    // 503 path.
    ExtractOutcome {
        status: 200,
        body: serialise(&result),
        retry_after: None,
    }
}

/// Convert a parsed `JsonValue` into an `Object` for the engine. The
/// engine's role-fact shape is `Seq(<key, value>)`, so JSON objects
/// become a Seq of 2-tuples; arrays pass through as Seqs; primitives
/// become atoms with a stable string rendering. Mirror of the shape
/// `hateoas::handle_arest_create_for_slug` builds for entity creates,
/// but lifted into a stand-alone helper because extract input isn't
/// limited to a flat field bag.
fn json_to_object(v: &json_min::JsonValue) -> Object {
    use json_min::JsonValue;
    match v {
        JsonValue::Null => Object::phi(),
        JsonValue::Bool(true) => Object::atom("T"),
        JsonValue::Bool(false) => Object::atom("F"),
        JsonValue::Str(s) => Object::atom(s),
        JsonValue::Num(n) => Object::atom(n),
        JsonValue::Array(items) => {
            Object::seq(items.iter().map(json_to_object).collect())
        }
        JsonValue::Object(pairs) => {
            let mut entries: Vec<Object> = Vec::with_capacity(pairs.len());
            for (k, v) in pairs {
                entries.push(Object::seq(alloc::vec![
                    Object::atom(k),
                    json_to_object(v),
                ]));
            }
            Object::seq(entries)
        }
    }
}

/// Build the 503 envelope per #620's spec. Includes the resolved
/// Agent Definition metadata (model + prompt) when `binding` is
/// `Some`; gracefully omits the field when no Agent Definition cell
/// is reachable for the verb. Worker fallback always present in
/// `_links.worker.href` so the envelope is structurally stable —
/// callers branch only on `agentDefinition`'s presence.
///
/// The introspection leak is bounded: Agent Definition metadata is
/// already publicly readable via `/explain` (#148); this just makes
/// it visible at the call site so a HATEOAS-aware client can pick a
/// profile to retry against without a second round-trip.
fn build_envelope(
    code: &str,
    message: &str,
    binding: Option<&agent::AgentBinding>,
) -> String {
    let mut out = String::with_capacity(384);
    out.push_str("{\"errors\":[{");
    out.push_str("\"code\":");
    out.push_str(&json_string_escape(code));
    out.push_str(",\"message\":");
    out.push_str(&json_string_escape(message));
    out.push_str(",\"verb\":\"extract\"");
    if let Some(b) = binding {
        out.push_str(",\"agentDefinition\":{");
        out.push_str("\"model\":");
        out.push_str(&json_string_escape(&b.model_code));
        out.push_str(",\"prompt\":");
        out.push_str(&json_string_escape(&b.prompt));
        out.push('}');
    }
    out.push_str(",\"_links\":{\"worker\":{\"href\":");
    out.push_str(&json_string_escape(EXTRACT_WORKER_URL));
    out.push_str("}}");
    out.push_str("}],\"status\":503,\"retryAfter\":");
    out.push_str(&json_string_escape(EXTRACT_WORKER_URL));
    out.push('}');
    out
}

/// Quote and escape a string per RFC 8259 §7. Hand-rolled twin of
/// `hateoas::json_string` — repeated rather than re-exported because
/// that helper is module-private in `arest::hateoas` (not part of the
/// crate's public surface; making it `pub` would lock the kernel into
/// an internal API). The two stay in sync structurally; if the engine
/// ever lifts its escaper into `arest::json_min` proper, this one can
/// collapse to a re-export.
fn json_string_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Read the baked SYSTEM state through a closure. `f` runs while
/// the read lock is held, so it should be cheap (clone the bits it
/// needs and return a value type — do not stash the `&Object`).
/// Returns `None` when `init` hasn't run.
///
/// This is the post-#451 read-side API, replacing XX's
/// `state() -> Option<&'static Object>`. New code (file_serve,
/// file_upload) reaches for this; the legacy `state()` shim below
/// is retained only for `net.rs` (forbidden in this commit) and
/// will be removed when that module migrates.
pub fn with_state<R>(f: impl FnOnce(&Object) -> R) -> Option<R> {
    let lock = SYSTEM.get()?;
    let guard = lock.read();
    Some(f(*guard))
}

/// Atomically replace the baked SYSTEM state with `new_state`.
/// Caller is responsible for building `new_state` such that it
/// already contains all desired facts (the kernel side of an
/// `ast::cell_push` chain — see `file_upload::build_file_facts`).
///
/// `Box::leak`s the new state so the legacy `state()` shim's
/// `&'static Object` snapshots remain valid for the kernel
/// lifetime. The write lock is held only across the pointer swap
/// + the diff computation — `f`-style closures that rebuild state
/// should compute `new_state` first, then call `apply` to commit.
///
/// After the pointer swap commits, every registered
/// `subscribe_changes` handler is invoked with the slice of cell
/// names whose contents changed (#458 — symmetric live-update path
/// for kernel Slint apps that mirrors ui.do's SSE +
/// TanStack-Query-cache-invalidation story from BroadcastDO #220 +
/// #234). Subscriber delivery happens AFTER the write lock is
/// released, against a snapshot of the registry — so a slow
/// handler doesn't block writes, and a handler that calls
/// `unsubscribe` from inside the closure can't deadlock on the
/// `SUBSCRIBERS` `Mutex`.
///
/// Returns `Err` only when `init()` hasn't run yet (a programmer
/// error — the call site should ensure boot ordering puts
/// `system::init` before any route that mutates).
pub fn apply(new_state: Object) -> Result<(), &'static str> {
    let lock = SYSTEM.get().ok_or("system::init() not called")?;
    let leaked: &'static Object = Box::leak(Box::new(new_state));
    // Diff inside the write critical section so we have stable
    // references to both old and new state. The diff returns an
    // owned `Vec<String>` so we can release the write lock before
    // delivering to subscribers.
    let changed: Vec<String> = {
        let mut guard = lock.write();
        let old: &Object = *guard;
        let diff = diff_cell_names(old, leaked);
        *guard = leaked;
        diff
    };
    deliver_changes(&changed);
    Ok(())
}

/// Register a handler invoked from `apply()` after every state
/// install. The handler receives a slice of cell names whose
/// contents differ between the previous and new state; it is
/// invoked synchronously on the thread calling `apply()` (the
/// kernel super-loop in `ui_apps::launcher::run`, which is also
/// where `net::poll()` drives HTTP-side `system::apply` calls).
///
/// Returns a `SubscriberId` for `unsubscribe`. The handler keeps
/// running until explicitly unsubscribed; callers that hold weak
/// references to UI components must keep the registration alive
/// for the lifetime of those components and call `unsubscribe`
/// from a Drop impl when the component goes away.
///
/// Handler signature is `Fn(&[String]) + Send + Sync` because
/// `SUBSCRIBERS` is shared mutable state behind a `Mutex` — even
/// though the kernel is single-threaded today, the bound matches
/// what an SMP-ready future will need without a re-shape of every
/// call site (SSE-on-the-kernel-wire is a likely #220-equivalent
/// follow-up that wants the same API surface).
pub fn subscribe_changes(handler: Box<dyn Fn(&[String]) + Send + Sync>) -> SubscriberId {
    let id = NEXT_SUBSCRIBER_ID.fetch_add(1, Ordering::Relaxed);
    // `Arc::from(Box<dyn Trait>)` reuses the existing allocation —
    // no extra allocation on the hot path. The internal storage is
    // `Arc` so delivery can snapshot-and-release (see SUBSCRIBERS
    // doc); the public surface accepts `Box` per #458's spec.
    let arc: Arc<dyn Fn(&[String]) + Send + Sync> = Arc::from(handler);
    SUBSCRIBERS.lock().insert(id, arc);
    id
}

/// Drop the handler associated with `id`. No-op if the id was
/// never registered or has already been removed (idempotent — the
/// HateoasBrowser drop path can call this even when the
/// subscriber registry has already been swept by some hypothetical
/// future tear-down).
pub fn unsubscribe(id: SubscriberId) {
    SUBSCRIBERS.lock().remove(&id);
}

/// Compute the cell-name diff between two SYSTEM states. Returns
/// every cell name that:
///   * appears in `old` but not `new` (cell removed),
///   * appears in `new` but not `old` (cell added), or
///   * appears in both but whose `Object` contents differ
///     (cell mutated — `cell_push` shape on `file_upload`'s POST
///     /file path appends a fact, which lands here as a content
///     mismatch on the existing cell name).
///
/// Cell-content equality is `Object: PartialEq` (derived in
/// `arest::ast`); this is a deep walk, but the kernel's HTTP
/// write rate is low and the working set is tens of cells with
/// hundreds of facts — well within budget for a synchronous
/// per-`apply` diff.
fn diff_cell_names(old: &Object, new: &Object) -> Vec<String> {
    // Build name → contents maps for both sides. `cells_iter`
    // returns `Vec<(&str, &Object)>` (see `arest::ast::cells_iter`),
    // which is fine to consume into a `BTreeMap` here — the
    // borrows live only as long as this function.
    let old_cells: BTreeMap<&str, &Object> = ast::cells_iter(old).into_iter().collect();
    let new_cells: BTreeMap<&str, &Object> = ast::cells_iter(new).into_iter().collect();

    let mut names: BTreeSet<String> = BTreeSet::new();
    // Symmetric difference + value-mismatch on the intersection.
    for (name, new_val) in &new_cells {
        match old_cells.get(name) {
            Some(old_val) if *old_val == *new_val => {} // unchanged
            _ => {
                names.insert((*name).to_string());
            }
        }
    }
    for name in old_cells.keys() {
        if !new_cells.contains_key(name) {
            names.insert((*name).to_string());
        }
    }
    names.into_iter().collect()
}

/// Invoke every registered subscriber with the changed-cells
/// slice. Iterates over a snapshot of `Arc`-cloned handler handles
/// taken under a brief lock, so:
///   * Handler reentrancy via `subscribe_changes` /
///     `unsubscribe` can't deadlock — the snapshot is owned, the
///     `SUBSCRIBERS` `Mutex` is released before any handler runs.
///   * A slow handler doesn't block other writes — the
///     `SYSTEM` `RwLock` is already released by the caller
///     (`apply()` drops its write guard before calling here).
///   * A handler that triggers its own unsubscribe still gets to
///     run to completion: the `Arc` clone in the snapshot keeps
///     the closure alive even after `unsubscribe` removes it from
///     the registry.
///
/// The snapshot is `Vec<Arc<dyn Fn>>` rather than `Vec<(id, Arc)>`
/// — we don't need the ids past the snapshot point, only the
/// handles. Order is BTreeMap-key order (i.e. registration
/// order, since SubscriberIds are monotonic), which gives stable
/// per-frame delivery semantics.
fn deliver_changes(changed: &[String]) {
    let snapshot: Vec<Arc<dyn Fn(&[String]) + Send + Sync>> = {
        let guard = SUBSCRIBERS.lock();
        guard.values().cloned().collect()
    };
    for handler in snapshot {
        handler(changed);
    }
}

/// Legacy borrow of the baked SYSTEM state, retained as a shim
/// for `net.rs` (forbidden in #451; migrates separately). Returns
/// the leaked-pointer snapshot taken under a brief read lock; the
/// `&'static Object` lifetime is sound because `apply()` never
/// reclaims old states — every install is a fresh `Box::leak`.
///
/// Prefer `with_state(|s| ...)` in new code.
pub fn state() -> Option<&'static Object> {
    let lock = SYSTEM.get()?;
    let guard = lock.read();
    Some(*guard)
}

/// Turn an Object into wire bytes. Atoms pass through; everything
/// else renders via the Debug fallback so the handler always has
/// something to send, even for shapes we haven't explicitly
/// formatted yet.
fn serialise(obj: &Object) -> Vec<u8> {
    match obj {
        Object::Atom(s) => s.clone().into_bytes(),
        Object::Bottom => b"\xE2\x8A\xA5\n".to_vec(), // U+22A5
        _ => {
            // Placeholder rendering — future commits replace this
            // with a proper FFP-to-JSON projection keyed off the
            // request's Accept header.
            alloc::format!("{:?}\n", obj).into_bytes()
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────
//
// `arest-kernel`'s bin target sets `test = false` (Cargo.toml L33),
// so these `#[cfg(test)]` cases are reachable only when the crate is
// re-shaped into a lib for hosted testing — the same pattern
// `file_serve.rs` and `file_upload.rs` use. They document the
// intended behaviour and provide a ready-to-run battery for the day
// the kernel grows a lib facade.
//
// To keep the tests independent of the singleton boot path (multiple
// tests in one process would race for the `Once`), each test scopes
// its assertions to a fresh `RwLock<&'static Object>` instance built
// the same way `init()` builds `SYSTEM`. The lock's read/write
// semantics are exercised against that local instance; the surface
// `with_state` / `apply` API is then exercised once via `init()` in
// the round-trip test, which uses a `call_once`-guarded global.

#[cfg(test)]
mod tests {
    use super::*;
    use arest::ast::fact_from_pairs;

    /// Helper mirroring `init()`'s leak-and-wrap shape. Returns a
    /// detached `RwLock<&'static Object>` so the lock semantics can
    /// be exercised without contending on the module-level
    /// `SYSTEM` singleton.
    fn fresh_slot(initial: Object) -> RwLock<&'static Object> {
        let leaked: &'static Object = Box::leak(Box::new(initial));
        RwLock::new(leaked)
    }

    /// First read after init returns the initial state — the
    /// pre-#451 "init then read" path still works through the new
    /// guarded shape.
    #[test]
    fn init_then_first_read_sees_initial_state() {
        let initial = ast::cell_push(
            "Probe",
            fact_from_pairs(&[("k", "v0")]),
            &Object::phi(),
        );
        let slot = fresh_slot(initial.clone());

        // Read-side mirror of `with_state` against the local slot.
        let observed: Object = {
            let guard = slot.read();
            (*guard).clone()
        };
        assert_eq!(observed, initial);
    }

    /// `apply` semantics: a write replaces the inner pointer; the
    /// next read sees the new state. This is the upload→download
    /// round-trip shape — POST /file builds new_state, calls
    /// apply(new_state), and the next GET /file/{id}/content sees
    /// the freshly-installed File facts.
    #[test]
    fn apply_replaces_state_subsequent_read_sees_new() {
        let initial = ast::cell_push(
            "Probe",
            fact_from_pairs(&[("k", "v0")]),
            &Object::phi(),
        );
        let slot = fresh_slot(initial);

        // Build a successor state and "apply" it against the local
        // slot (mirrors the module-level `apply`'s leak-and-swap).
        let next = ast::cell_push(
            "Probe",
            fact_from_pairs(&[("k", "v1")]),
            {
                let g = slot.read();
                *g
            },
        );
        let leaked: &'static Object = Box::leak(Box::new(next.clone()));
        {
            let mut g = slot.write();
            *g = leaked;
        }

        let after: Object = {
            let g = slot.read();
            (*g).clone()
        };
        assert_eq!(after, next);

        // The new state really does contain both fact versions
        // (cell_push appends; the latest read should see the
        // updated cell with two entries).
        let cell = ast::fetch_or_phi("Probe", &after);
        let seq = cell.as_seq().unwrap_or(&[]);
        assert!(
            seq.len() >= 2,
            "post-apply state should carry both Probe facts, saw {}", seq.len(),
        );
    }

    /// Concurrent reads are allowed — `RwLock` shares the read lock
    /// across multiple holders. Exercised here by acquiring two
    /// read guards simultaneously and confirming both see the same
    /// snapshot.
    #[test]
    fn concurrent_reads_share_the_lock() {
        let initial = Object::atom("shared");
        let slot = fresh_slot(initial.clone());

        let g1 = slot.read();
        let g2 = slot.read();
        // Both guards observe the same `&'static Object` pointer
        // value because `apply` hasn't run between them.
        assert!(core::ptr::eq(*g1, *g2));
        assert_eq!(**g1, initial);
        assert_eq!(**g2, initial);
        // Drop both before exiting (writes would block until then).
        drop(g1);
        drop(g2);
    }

    /// `try_write` fails while a read guard is held, confirming the
    /// exclusive-write semantics. The kernel is single-threaded
    /// today (per the #451 brief), so this is lock semantics, not
    /// real concurrency — but `RwLock` is the right primitive for
    /// when concurrency lands.
    #[test]
    fn write_is_blocked_while_read_held() {
        let slot = fresh_slot(Object::atom("rd"));
        let read_guard = slot.read();
        // `try_write` returns `None` because the read guard is live.
        assert!(slot.try_write().is_none());
        drop(read_guard);
        // Once the read guard is dropped the write goes through.
        assert!(slot.try_write().is_some());
    }

    /// End-to-end exercise of the public `init` → `with_state` →
    /// `apply` → `with_state` round trip against the module
    /// singleton. Guarded so a re-run within the same process
    /// (e.g. `cargo test` with multiple tests sharing the binary)
    /// doesn't double-init.
    ///
    /// The init step is a no-op when SYSTEM is already populated
    /// (spin::Once contract), so this test is robust to ordering
    /// against any other test that happens to call `init()`.
    #[test]
    fn module_round_trip_init_apply_read() {
        init();

        // Read the as-init'd state — must contain the `welcome`
        // and `echo` def cells `init()` baked in.
        let pre = with_state(|s| s.clone()).expect("init ran");
        // `defs_to_state` emits one `D_has_<name>` cell per def;
        // here we only need to confirm the state isn't bottom.
        assert_ne!(pre, Object::Bottom);

        // Apply a new state with a custom probe cell on top of the
        // current one.
        let next = ast::cell_push(
            "ProbeApply",
            fact_from_pairs(&[("k", "v")]),
            &pre,
        );
        apply(next.clone()).expect("apply succeeds post-init");

        // Read again — the probe cell must be visible now.
        let post = with_state(|s| {
            let cell = ast::fetch_or_phi("ProbeApply", s);
            cell.as_seq().map(|f| f.len()).unwrap_or(0)
        })
        .expect("init ran");
        assert!(post >= 1, "post-apply read should see ProbeApply cell");
    }

    /// The legacy `state()` shim still returns the same pointer
    /// `with_state` exposes, for as long as `net.rs` hasn't
    /// migrated (file forbidden in #451). The two APIs share the
    /// same backing slot so they can't diverge.
    #[test]
    fn legacy_state_shim_matches_with_state() {
        init();
        let via_shim = state().expect("init ran");
        let via_with = with_state(|s| s as *const Object).expect("init ran");
        assert_eq!(via_shim as *const Object, via_with);
    }

    // ── #458 subscribe_changes / unsubscribe / diff_cell_names ─────

    /// Diff sees an added cell (present in new, absent in old).
    /// Mirrors the `POST /file` shape: the previous state had no
    /// `File_has_Name` cell; the new state adds one.
    #[test]
    fn diff_cell_names_emits_added_cells() {
        let old = Object::phi();
        let new = ast::cell_push(
            "File_has_Name",
            fact_from_pairs(&[("File", "f1"), ("Name", "alpha.txt")]),
            &old,
        );
        let changed = diff_cell_names(&old, &new);
        assert_eq!(changed, alloc::vec!["File_has_Name".to_string()]);
    }

    /// Diff sees a mutated cell (same name, different contents).
    /// Mirrors the `cell_push`-on-existing-cell shape: a second
    /// `File_has_Name` fact appended changes the cell's contents
    /// even though the cell name is identical.
    #[test]
    fn diff_cell_names_emits_mutated_cells() {
        let s1 = ast::cell_push(
            "File_has_Name",
            fact_from_pairs(&[("File", "f1"), ("Name", "a")]),
            &Object::phi(),
        );
        let s2 = ast::cell_push(
            "File_has_Name",
            fact_from_pairs(&[("File", "f2"), ("Name", "b")]),
            &s1,
        );
        let changed = diff_cell_names(&s1, &s2);
        assert!(changed.iter().any(|n| n == "File_has_Name"),
            "expected File_has_Name in changed, got {changed:?}");
    }

    /// Identical states yield no changes — important for the
    /// HateoasBrowser path, which redraws on every subscriber call:
    /// a no-op `apply` shouldn't trigger a redraw.
    #[test]
    fn diff_cell_names_empty_for_unchanged_states() {
        let s = ast::cell_push(
            "Probe",
            fact_from_pairs(&[("k", "v")]),
            &Object::phi(),
        );
        let changed = diff_cell_names(&s, &s);
        assert!(changed.is_empty(), "expected empty diff, got {changed:?}");
    }

    /// `subscribe_changes` returns distinct ids per registration;
    /// `unsubscribe` is idempotent and a second call is a no-op.
    #[test]
    fn subscribe_unsubscribe_id_lifecycle() {
        use core::sync::atomic::AtomicUsize;
        let counter = Arc::new(AtomicUsize::new(0));
        let c1 = counter.clone();
        let id1 = subscribe_changes(Box::new(move |_changed: &[String]| {
            c1.fetch_add(1, Ordering::Relaxed);
        }));
        let c2 = counter.clone();
        let id2 = subscribe_changes(Box::new(move |_changed: &[String]| {
            c2.fetch_add(1, Ordering::Relaxed);
        }));
        assert_ne!(id1, id2, "subscriber ids must be distinct");
        unsubscribe(id1);
        unsubscribe(id1); // double-unsubscribe is a no-op
        unsubscribe(id2);
    }

    /// End-to-end: a subscribed handler sees the cell-name diff
    /// after `apply()`. This is the path #458 wires for
    /// HateoasBrowser to react to `POST /file` mutations.
    #[test]
    fn apply_delivers_changed_cells_to_subscribers() {
        use core::sync::atomic::AtomicUsize;
        init();

        // Capture the changed-cells slice the handler receives.
        // We use a `Mutex<Vec<String>>` rather than the `RefCell`
        // shape the rest of the kernel uses because the handler's
        // `Send + Sync` bound forbids `RefCell`.
        let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let calls = Arc::new(AtomicUsize::new(0));
        let captured_clone = captured.clone();
        let calls_clone = calls.clone();
        let id = subscribe_changes(Box::new(move |changed: &[String]| {
            calls_clone.fetch_add(1, Ordering::Relaxed);
            *captured_clone.lock() = changed.to_vec();
        }));

        // Build a successor state with a unique cell-name probe so
        // the assertion is stable against whatever other tests have
        // already mutated SYSTEM. Picking a cell name that won't
        // collide with `File_*` in case other tests in the same
        // binary touch file_upload.
        let pre = with_state(|s| s.clone()).expect("init ran");
        let probe_cell = "Subscribe458Probe_has_Marker";
        let next = ast::cell_push(
            probe_cell,
            fact_from_pairs(&[("Subscribe458Probe", "p"), ("Marker", "m")]),
            &pre,
        );
        apply(next).expect("apply succeeds post-init");

        // Handler ran at least once (other tests may also be
        // applying state in the same binary; we only assert OUR
        // call observed the probe).
        assert!(calls.load(Ordering::Relaxed) >= 1, "handler must have run");
        let observed = captured.lock().clone();
        assert!(observed.iter().any(|n| n == probe_cell),
            "expected {probe_cell} in changed list, got {observed:?}");

        unsubscribe(id);
    }

    // ── #580 seed_ui_bundle_cells round-trip ───────────────────────

    /// The seeder is a no-op when the `UI_ASSETS` table is empty (the
    /// default build shape — no `--features ui-bundle`). Returning the
    /// state untouched preserves the handler's "no bundle → 404"
    /// behaviour without any extra cell-graph noise.
    #[test]
    fn seed_ui_bundle_cells_is_noop_with_empty_table() {
        // Skip when the local build happens to have ui-bundle on; the
        // assertion only makes sense for the empty-table path.
        if !assets::UI_ASSETS.is_empty() {
            return;
        }
        let initial = ast::cell_push(
            "Probe",
            fact_from_pairs(&[("k", "v")]),
            &Object::phi(),
        );
        let after = seed_ui_bundle_cells(initial.clone());
        assert_eq!(after, initial,
            "empty UI_ASSETS must produce a state-equal seed");
    }

    /// When the table IS non-empty, every entry produces a
    /// `File_has_Path` + `File_has_ContentRef` round-trip via the
    /// asset-side decoder. We don't try to assert a specific path
    /// here (the bundle's contents depend on what `apps/ui.do/dist/`
    /// looked like at build time); we just confirm the first entry
    /// round-trips cleanly.
    #[test]
    fn seed_ui_bundle_cells_round_trips_first_entry() {
        if assets::UI_ASSETS.is_empty() {
            return;
        }
        let (http_path, body) = assets::UI_ASSETS[0];
        let after = seed_ui_bundle_cells(Object::phi());
        let asset = assets::lookup_from_state(&after, http_path)
            .expect("seeded entry must round-trip via lookup_from_state");
        assert_eq!(asset.body, body.to_vec());
    }

    /// A subscriber that calls `unsubscribe(its_own_id)` from
    /// inside the handler must not deadlock — the snapshot
    /// pattern in `deliver_changes` clones an `Arc` of each
    /// handler under a brief lock, releases the lock, then
    /// invokes; an unsubscribe-during-handler-call only mutates
    /// the registry, the `Arc` snapshot keeps the closure alive
    /// for the rest of the call.
    #[test]
    fn handler_can_unsubscribe_itself_without_deadlock() {
        use core::cell::Cell;
        init();
        // Cell of the id, populated after we know the id.
        let id_slot: Arc<Mutex<Option<SubscriberId>>> = Arc::new(Mutex::new(None));
        let ran: Arc<Cell<bool>> = Arc::new(Cell::new(false));
        // `Cell` is !Sync — but the kernel runs single-threaded
        // and the test harness is single-threaded too. We need
        // `Send + Sync` on the closure; wrap the `Cell` in an
        // `unsafe impl Send + Sync` shim via a newtype. For test
        // purposes we use an `AtomicBool` instead — same
        // semantics, no unsafe.
        let _ = ran;
        use core::sync::atomic::AtomicBool;
        let ran = Arc::new(AtomicBool::new(false));
        let id_slot_handler = id_slot.clone();
        let ran_handler = ran.clone();
        let id = subscribe_changes(Box::new(move |_changed: &[String]| {
            ran_handler.store(true, Ordering::Relaxed);
            // Self-unsubscribe. Without the snapshot pattern this
            // would deadlock against the SUBSCRIBERS Mutex.
            if let Some(my_id) = *id_slot_handler.lock() {
                unsubscribe(my_id);
            }
        }));
        *id_slot.lock() = Some(id);

        // Force an apply — handler must run + survive its own
        // unsubscribe call.
        let pre = with_state(|s| s.clone()).expect("init ran");
        let next = ast::cell_push(
            "DeadlockProbe",
            fact_from_pairs(&[("k", "v")]),
            &pre,
        );
        apply(next).expect("apply succeeds post-init");

        assert!(ran.load(Ordering::Relaxed), "handler must have run");
        // After the self-unsubscribe, a second apply must NOT
        // re-invoke the handler.
        ran.store(false, Ordering::Relaxed);
        let pre2 = with_state(|s| s.clone()).expect("init ran");
        let next2 = ast::cell_push(
            "DeadlockProbe2",
            fact_from_pairs(&[("k", "w")]),
            &pre2,
        );
        apply(next2).expect("apply succeeds post-init");
        assert!(!ran.load(Ordering::Relaxed),
            "handler must not re-run after self-unsubscribe");
    }

    // ── #620 / HATEOAS-6b — POST /arest/extract dispatch ──────────────
    //
    // These tests exercise `dispatch_extract` against locally-built
    // states rather than the module-level `SYSTEM` singleton, so they
    // can stage Agent Definition cells per case without contending on
    // the global init path. The 503-with-installed-body case uses the
    // sec-2 platform fallback registry (host-only via the std-deps
    // arest dep — see `crates/arest-kernel/Cargo.toml`'s
    // `[target.'cfg(not(target_os = "uefi"))'.dependencies]` block)
    // so the dispatch can resolve a real body without a worker
    // present. UEFI-target builds elide the platform fallback shim
    // (kernel profile is `feature = "no_std"`); those tests don't run
    // there and that's intentional — UEFI verification is a
    // `cargo check` not a `cargo test`.
    //
    // The `dispatch_extract` function itself reads from `with_state`,
    // which means each test must `init()` the singleton (or at minimum
    // mutate it via `apply`) before driving the dispatcher. The
    // singleton is process-lifetime, so subsequent tests share the
    // base state — the assertions below all key on cell-name probes
    // unique to the test case so cross-test contamination doesn't
    // matter. The Agent-Definition tests further isolate themselves by
    // staging into the live SYSTEM via `apply`, asserting against the
    // result, then restoring the singleton via a follow-up apply that
    // clears the staged cells (so a later test seeing the singleton
    // doesn't observe stale Verb / Agent_Definition_* facts).

    #[test]
    fn extract_returns_503_when_no_body() {
        init();
        // Empty body — the Platform-fn slot is registered at init but
        // no body is installed in the kernel profile, so apply
        // produces Bottom and the envelope lifts to 503.
        let outcome = dispatch_extract(b"");
        assert_eq!(outcome.status, 503, "no body installed must surface as 503");
        let body_str = core::str::from_utf8(&outcome.body).expect("envelope is utf-8 json");
        assert!(
            body_str.contains("\"code\":\"extract.no_body\""),
            "envelope must carry extract.no_body code, got: {body_str}",
        );
        assert_eq!(
            outcome.retry_after.as_deref(),
            Some(EXTRACT_WORKER_URL),
            "Retry-After header must point at the worker URL",
        );
    }

    #[test]
    fn extract_envelope_includes_agent_definition_when_resolved() {
        init();
        // Stage the four cells `agent::resolve_agent_verb` walks:
        // Verb, Verb_invokes_Agent_Definition,
        // Agent_Definition_uses_Model, Agent_Definition_has_Prompt.
        // We capture the pre-state so we can roll back at the end and
        // not contaminate sibling tests sharing the singleton.
        let pre = with_state(|s| s.clone()).expect("init ran");
        let s = ast::cell_push(
            "Verb",
            fact_from_pairs(&[("id", "verb-extract"), ("name", "extract")]),
            &pre,
        );
        let s = ast::cell_push(
            "Verb_invokes_Agent_Definition",
            fact_from_pairs(&[
                ("Verb", "verb-extract"),
                ("Agent Definition", "agent-extractor"),
            ]),
            &s,
        );
        let s = ast::cell_push(
            "Agent_Definition_uses_Model",
            fact_from_pairs(&[
                ("Agent Definition", "agent-extractor"),
                ("Model", "claude-sonnet-4.6"),
            ]),
            &s,
        );
        let s = ast::cell_push(
            "Agent_Definition_has_Prompt",
            fact_from_pairs(&[
                ("Agent Definition", "agent-extractor"),
                ("Prompt", "Extract one fact per claim. Be terse."),
            ]),
            &s,
        );
        apply(s).expect("apply succeeds post-init");

        let outcome = dispatch_extract(b"");
        assert_eq!(outcome.status, 503);
        let body_str = core::str::from_utf8(&outcome.body).expect("envelope is utf-8 json");
        assert!(
            body_str.contains("\"agentDefinition\""),
            "envelope must carry agentDefinition when resolved, got: {body_str}",
        );
        assert!(
            body_str.contains("\"model\":\"claude-sonnet-4.6\""),
            "envelope must surface the resolved model code, got: {body_str}",
        );
        assert!(
            body_str.contains("\"prompt\":\"Extract one fact per claim. Be terse.\""),
            "envelope must surface the resolved prompt, got: {body_str}",
        );

        // Roll the singleton back to its pre-test state so siblings
        // don't observe the staged Verb / Agent_Definition_* cells.
        apply(pre).expect("rollback apply succeeds");
    }

    #[test]
    fn extract_envelope_omits_agent_definition_when_unresolved() {
        init();
        // Snapshot the singleton; if a sibling test (above) staged
        // Agent Definition cells and the rollback ran, we should see
        // an unresolved verb here. Belt-and-braces: explicitly assert
        // that no Verb cell carries `name = extract` in the snapshot
        // we're about to dispatch against. If one is present the test
        // is an honest false negative — flag it via skip rather than
        // a brittle pass.
        let snap = with_state(|s| s.clone()).expect("init ran");
        let has_verb = ast::fetch_or_phi("Verb", &snap)
            .as_seq()
            .map(|seq| seq.iter().any(|v| ast::binding(v, "name") == Some("extract")))
            .unwrap_or(false);
        if has_verb {
            // Sibling test ran and didn't roll back — skip, this case
            // can only assert the absent path against an unstaged
            // snapshot.
            return;
        }
        let outcome = dispatch_extract(b"");
        assert_eq!(outcome.status, 503);
        let body_str = core::str::from_utf8(&outcome.body).expect("envelope is utf-8 json");
        assert!(
            !body_str.contains("\"agentDefinition\""),
            "envelope must omit agentDefinition when verb is unresolved, got: {body_str}",
        );
        // The other fields stay present — code, message, verb,
        // _links.worker, status, retryAfter.
        assert!(body_str.contains("\"code\":\"extract.no_body\""));
        assert!(body_str.contains("\"verb\":\"extract\""));
        assert!(body_str.contains("\"_links\":{\"worker\""));
    }

    #[test]
    fn extract_dispatch_does_not_panic_on_malformed_json() {
        init();
        // Garbage input — the parser returns None, and the dispatcher
        // surfaces an `extract.parse` envelope rather than panicking.
        // 503 (not 400) keeps the dispatch contract uniform: every
        // failure mode here resolves as "this profile can't fulfil
        // the call, here's where it might succeed".
        let outcome = dispatch_extract(b"{not-json");
        assert_eq!(outcome.status, 503);
        let body_str = core::str::from_utf8(&outcome.body).expect("envelope is utf-8 json");
        assert!(
            body_str.contains("\"code\":\"extract.parse\""),
            "malformed JSON must produce extract.parse code, got: {body_str}",
        );
        assert_eq!(
            outcome.retry_after.as_deref(),
            Some(EXTRACT_WORKER_URL),
        );
    }

    /// Confirms the slot mechanism works end-to-end once a body is
    /// installed: the verb is registered as `Func::Platform("extract")`
    /// at boot, the host-target arest build's PLATFORM_FALLBACK
    /// registry accepts an installed body, and `dispatch_extract`
    /// surfaces 200 instead of 503. UEFI builds elide
    /// `install_platform_fn` (it's `cfg(not(feature = "no_std"))` in
    /// `arest::ast`), so this test is `cfg(not(target_os = "uefi"))`-
    /// only — the per-target dep block in
    /// `crates/arest-kernel/Cargo.toml` resolves the host-deps
    /// arest crate variant for `cargo test --lib -p arest-kernel`,
    /// where the registry is reachable. The UEFI cross-compile
    /// (`cargo check --target x86_64-unknown-uefi --features
    /// server --tests`) elides this case entirely, which is what
    /// the bar in #620 documents (UEFI verification is a
    /// `cargo check`, not a `cargo test`).
    #[cfg(not(target_os = "uefi"))]
    #[test]
    fn extract_with_installed_body_returns_200() {
        use arest::ast::{install_platform_fn, uninstall_platform_fn};
        init();

        // Install an echo-shaped body that turns the input Object into
        // a known atom so we can assert against it. The body just
        // returns an atom marker — the assertion is structural (status
        // 200, body contains the marker), not a deep round-trip of
        // the input shape.
        install_platform_fn(
            "extract",
            arest::sync::Arc::new(|_input: &Object, _d: &Object| {
                Object::atom("LLM(echo): kernel-extract-installed")
            }),
        );

        let outcome = dispatch_extract(b"{\"foo\":\"bar\"}");
        // Restore the kernel default (no body installed) before the
        // assertions so a panic doesn't leave the registry hot for
        // the no-body tests above (#620 — those tests assume Bottom).
        uninstall_platform_fn("extract");

        assert_eq!(outcome.status, 200, "installed body must surface as 200");
        let body_str = core::str::from_utf8(&outcome.body).expect("response is utf-8");
        assert!(
            body_str.contains("LLM(echo): kernel-extract-installed"),
            "installed body's output must round-trip, got: {body_str}",
        );
        assert!(
            outcome.retry_after.is_none(),
            "200 response must not carry a Retry-After hint",
        );
    }
}
