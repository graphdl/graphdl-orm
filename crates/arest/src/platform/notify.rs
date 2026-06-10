// crates/arest/src/platform/notify.rs
//
// `notify` — the canonical §5.2 notify effect (pb-effect-fns-canonical),
// as a Platform fn body. The minimal host surface: one line to stderr
// (the CLI/MCP host's diagnostic stream; the serial console on a std
// kernel build), echoing the message back as the result so a Verb
// dispatch can observe success. Richer sinks (kernel toast surface,
// worker webhook/push) install their own body under the same name —
// the name is the contract, the body is the target's.
//
// Operand shapes (either):
//   <message-atom>
//   < <'message', m>, <'level', l>? >
//
// Result: the message atom (echo = delivered). Object::Bottom on a
// malformed operand (apply() totality). Deliberately NO state reach:
// recording notifications as facts is the upsert/effect-delta design
// question tracked on the board, not smuggled in here.

use crate::ast::{self, Object};
use crate::sync::Arc;
use alloc::string::{String, ToString};

/// Register the `notify` body. Pre-approved in
/// `ast::APPROVED_PLATFORM_FN_NAMES` (sec-2: stderr write only).
pub fn install() {
    let f: ast::PlatformFn = Arc::new(|x: &Object, d: &Object| notify_apply(x, d));
    ast::install_platform_fn("notify", f);
}

fn notify_apply(x: &Object, _d: &Object) -> Object {
    let Some((message, level)) = decode_operand(x) else {
        return Object::Bottom;
    };
    #[cfg(not(feature = "no_std"))]
    {
        std::eprintln!("[notify{}] {}",
            level.as_deref().map(|l| alloc::format!(":{}", l)).unwrap_or_default(),
            message);
    }
    Object::atom(&message)
}

fn decode_operand(x: &Object) -> Option<(String, Option<String>)> {
    if let Some(m) = x.as_atom() {
        return Some((m.to_string(), None));
    }
    let sections = x.as_seq()?;
    let field = |tag: &str| -> Option<String> {
        sections.iter().find_map(|s| {
            let pair = s.as_seq()?;
            (pair.first()?.as_atom()? == tag)
                .then(|| pair.get(1)?.as_atom().map(str::to_string))?
        })
    };
    let message = field("message")?;
    Some((message, field("level")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Echo semantics: the message comes back as the result, both shapes.
    #[test]
    fn echoes_message_for_both_operand_shapes() {
        assert_eq!(notify_apply(&Object::atom("build done"), &Object::Bottom),
            Object::atom("build done"));
        let tagged = Object::seq(alloc::vec![
            Object::seq(alloc::vec![
                Object::atom("message"), Object::atom("rebuilt"),
            ]),
            Object::seq(alloc::vec![
                Object::atom("level"), Object::atom("info"),
            ]),
        ]);
        assert_eq!(notify_apply(&tagged, &Object::Bottom), Object::atom("rebuilt"));
    }

    /// Malformed operand bottoms; install() round-trips via dispatch.
    #[test]
    fn malformed_bottoms_and_dispatch_works() {
        assert_eq!(notify_apply(&Object::seq(alloc::vec![]), &Object::Bottom),
            Object::Bottom);
        install();
        let out = ast::apply(
            &ast::Func::Platform("notify".to_string()),
            &Object::atom("via dispatch"),
            &Object::Bottom,
        );
        assert_eq!(out, Object::atom("via dispatch"));
    }
}
