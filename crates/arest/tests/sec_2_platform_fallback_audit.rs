//! Sec-2 guard (2026-04-21): no production path in the arest crate
//! writes to PLATFORM_FALLBACK or ASYNC_PLATFORM_FALLBACK. Lives in
//! its own integration-test binary so installs inside other test
//! binaries (e.g. e3_integration.rs) cannot leak into its view of the
//! process-global registries.
//!
//! When a future production path installs a body, add its name to the
//! matching APPROVED_*_PLATFORM_FN_NAMES constant in ast.rs AND update
//! _reports/sec-2-platform-audit-2026-04-21.md with the classification.

use std::collections::HashSet;

use arest::ast;

fn extras<'a>(installed: &'a [String], approved: &'static [&'static str]) -> Vec<&'a String> {
    let allow: HashSet<&str> = approved.iter().copied().collect();
    installed.iter().filter(|n| !allow.contains(n.as_str())).collect()
}

#[test]
fn sync_platform_fallback_contains_only_approved_names() {
    let installed = ast::installed_platform_fn_names();
    let extra = extras(&installed, ast::APPROVED_PLATFORM_FN_NAMES);
    assert!(
        extra.is_empty(),
        "PLATFORM_FALLBACK contains unapproved names {extra:?}. \
         Add them to ast::APPROVED_PLATFORM_FN_NAMES and update \
         _reports/sec-2-platform-audit-2026-04-21.md, or drop the writer."
    );
}

#[test]
fn async_platform_fallback_contains_only_approved_names() {
    let installed = ast::installed_async_platform_fn_names();
    let extra = extras(&installed, ast::APPROVED_ASYNC_PLATFORM_FN_NAMES);
    assert!(
        extra.is_empty(),
        "ASYNC_PLATFORM_FALLBACK contains unapproved names {extra:?}. \
         Add them to ast::APPROVED_ASYNC_PLATFORM_FN_NAMES and update \
         _reports/sec-2-platform-audit-2026-04-21.md, or drop the writer."
    );
}
