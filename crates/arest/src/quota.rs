// crates/arest/src/quota.rs
//
// Per-App resource quotas — the kernel's cgroup-equivalent.
//
// Per-App namespacing: Apps are containers. A noisy App's runaway
// derivation chain shouldn't be able to starve other Apps; the
// kernel enforces ceilings.
//
// Three accountable resources:
//   - audit-log entries  (count of audit_log cell entries)
//   - CPU time           (cumulative nanoseconds of μ evaluation)
//   - memory             (population size — approximated as cell count)
//
// This initial cut implements the enforcement primitive for all
// three dimensions. CPU + memory accounting instrumentation (when
// to record, where to read) follow in a separate commit; this
// module lands the shape and the audit-log counter that's always
// available.
//
// Quota declarations live as instance facts on the App:
//   App 'X' has Audit Log Limit '1000'.
//   App 'X' has Cpu Ms Quota '5000'.
//   App 'X' has Memory Cells Limit '10000'.
//
// `check_quota(state, resource, limit)` returns the current usage
// and whether the limit is exceeded. Callers decide whether to
// reject the mutation, emit a deontic warning, or just record.

use crate::ast::{fetch, Object};
use crate::types::Domain;
#[allow(unused_imports)]
use alloc::{string::{String, ToString}, vec::Vec, boxed::Box, borrow::ToOwned};

/// A resource dimension under quota.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resource {
    /// Number of entries in the audit_log cell. Each mutation
    /// appends one entry, so this counts total committed
    /// operations across all tenants sharing this App's namespace.
    AuditLogEntries,
    /// Cumulative nanoseconds of μ-evaluation time. Instrumentation
    /// to record this is wired in a follow-up — today the counter
    /// cell is read as-is and returns zero if absent.
    CpuNanos,
    /// Approximate population size, measured as cell count. Each
    /// fact-type cell in the state contributes one unit. Keep away
    /// from trying to measure bytes — Object sizes are recursive
    /// and the accounting would dominate the budget we're capping.
    MemoryCells,
}

/// Result of a quota check: current usage plus an exceeded flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuotaStatus {
    pub resource: Resource,
    pub usage: u64,
    pub limit: u64,
    pub exceeded: bool,
}

/// Count the audit-log entries in state. Used as the usage value
/// for Resource::AuditLogEntries. The audit_log cell is a Seq of
/// audit records appended by record_audit.
pub fn audit_log_count(state: &Object) -> u64 {
    match fetch("audit_log", state) {
        Object::Seq(items) => items.len() as u64,
        _ => 0,
    }
}

/// Count non-def cells in state. Used as the usage value for
/// Resource::MemoryCells. Def cells (names containing ':') are
/// kernel-internal and don't count against the App's budget.
pub fn cell_count(state: &Object) -> u64 {
    match state {
        Object::Map(m) => m.keys().filter(|k| !k.contains(':')).count() as u64,
        Object::Seq(cells) => cells.iter()
            .filter_map(|cell| cell.as_seq())
            .filter(|items| items.len() == 3
                && items[0].as_atom() == Some("CELL")
                && items[1].as_atom().is_some_and(|n| !n.contains(':')))
            .count() as u64,
        _ => 0,
    }
}

/// Read the cumulative CPU-nanosecond counter from state. When the
/// instrumentation hasn't recorded anything yet, returns 0.
///
/// The counter lives at cell name `quota:cpu_nanos` and holds a
/// single atom whose value is the cumulative ns as a decimal
/// string. Writes are expected to come from a future
/// `record_cpu_usage(state, delta_ns)` helper invoked after each
/// μ application.
pub fn cpu_nanos_used(state: &Object) -> u64 {
    match fetch("quota:cpu_nanos", state) {
        Object::Atom(s) => s.parse::<u64>().unwrap_or(0),
        _ => 0,
    }
}

/// Look up an App's declared quota for `resource` from the domain's
/// instance facts. Returns None when no limit is declared —
/// callers decide the default policy (enforce a system-wide
/// ceiling, fail open, etc.).
///
/// Fact-shape:
///   App '<slug>' has Audit Log Limit '<n>'.
///   App '<slug>' has Cpu Ms Quota '<ms>'.
///   App '<slug>' has Memory Cells Limit '<n>'.
pub fn app_quota_limit(domain: &Domain, app: &str, resource: Resource) -> Option<u64> {
    let field = match resource {
        Resource::AuditLogEntries => "Audit Log Limit",
        Resource::CpuNanos        => "Cpu Ms Quota",
        Resource::MemoryCells     => "Memory Cells Limit",
    };
    domain.general_instance_facts.iter()
        .find(|f| f.subject_noun == "App"
              && f.subject_value == app
              && f.field_name == field)
        .and_then(|f| f.object_value.parse::<u64>().ok())
        .map(|raw| match resource {
            // Cpu Ms Quota is declared in milliseconds; convert to ns
            // so the usage & limit are in the same unit.
            Resource::CpuNanos => raw.saturating_mul(1_000_000),
            _ => raw,
        })
}

/// Compute the quota status for a given resource. Returns usage,
/// limit (or u64::MAX for "no declared ceiling"), and whether the
/// usage has met or exceeded the limit.
pub fn check_quota(
    state: &Object,
    domain: &Domain,
    app: &str,
    resource: Resource,
) -> QuotaStatus {
    let usage = match resource {
        Resource::AuditLogEntries => audit_log_count(state),
        Resource::CpuNanos        => cpu_nanos_used(state),
        Resource::MemoryCells     => cell_count(state),
    };
    let limit = app_quota_limit(domain, app, resource).unwrap_or(u64::MAX);
    let exceeded = usage >= limit && limit != u64::MAX;
    QuotaStatus { resource, usage, limit, exceeded }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{cell_push, fact_from_pairs, Object};
    use crate::types::{Domain, GeneralInstanceFact};

    fn domain_with_app_quota(app: &str, field: &str, value: &str) -> Domain {
        let mut d = Domain::default();
        d.general_instance_facts.push(GeneralInstanceFact {
            subject_noun: "App".into(),
            subject_value: app.into(),
            field_name: field.into(),
            object_noun: "Quota".into(),
            object_value: value.into(),
        });
        d
    }

    #[test]
    fn audit_log_count_on_empty_state_is_zero() {
        assert_eq!(audit_log_count(&Object::phi()), 0);
    }

    #[test]
    fn audit_log_count_scales_with_appended_entries() {
        let s0 = Object::phi();
        let s1 = cell_push("audit_log",
            fact_from_pairs(&[("op", "create"), ("outcome", "ok")]), &s0);
        let s2 = cell_push("audit_log",
            fact_from_pairs(&[("op", "update"), ("outcome", "ok")]), &s1);
        assert_eq!(audit_log_count(&s0), 0);
        assert_eq!(audit_log_count(&s1), 1);
        assert_eq!(audit_log_count(&s2), 2);
    }

    #[test]
    fn app_quota_limit_reads_audit_log_limit_fact() {
        let d = domain_with_app_quota("sherlock", "Audit Log Limit", "1000");
        assert_eq!(app_quota_limit(&d, "sherlock", Resource::AuditLogEntries), Some(1000));
        assert_eq!(app_quota_limit(&d, "other", Resource::AuditLogEntries), None);
    }

    #[test]
    fn app_quota_limit_converts_cpu_ms_to_ns() {
        let d = domain_with_app_quota("sherlock", "Cpu Ms Quota", "5");
        // 5ms = 5_000_000 ns
        assert_eq!(app_quota_limit(&d, "sherlock", Resource::CpuNanos), Some(5_000_000));
    }

    #[test]
    fn check_quota_reports_under_at_over() {
        let d = domain_with_app_quota("sherlock", "Audit Log Limit", "2");

        // Under: 0 entries < 2
        let status = check_quota(&Object::phi(), &d, "sherlock", Resource::AuditLogEntries);
        assert_eq!(status.usage, 0);
        assert_eq!(status.limit, 2);
        assert!(!status.exceeded);

        // At limit: 2 entries >= 2 — exceeded
        let state = cell_push("audit_log",
            fact_from_pairs(&[("op", "a")]),
            &cell_push("audit_log", fact_from_pairs(&[("op", "b")]), &Object::phi()));
        let status = check_quota(&state, &d, "sherlock", Resource::AuditLogEntries);
        assert_eq!(status.usage, 2);
        assert!(status.exceeded);
    }

    #[test]
    fn check_quota_no_declared_limit_never_exceeds() {
        let d = Domain::default();
        let state = cell_push("audit_log", fact_from_pairs(&[("op", "a")]), &Object::phi());
        let status = check_quota(&state, &d, "sherlock", Resource::AuditLogEntries);
        assert_eq!(status.usage, 1);
        assert_eq!(status.limit, u64::MAX);
        assert!(!status.exceeded);
    }

    #[test]
    fn cell_count_ignores_def_cells() {
        // `schema:` / `validate:` / etc are kernel-internal and
        // shouldn't count against the App's memory budget.
        let mut m = hashbrown::HashMap::new();
        m.insert("Noun".to_string(), Object::phi());
        m.insert("FactType".to_string(), Object::phi());
        m.insert("schema:Order".to_string(), Object::phi()); // def — excluded
        m.insert("validate:c1".to_string(), Object::phi());  // def — excluded
        let state = Object::Map(m);
        assert_eq!(cell_count(&state), 2);
    }
}
