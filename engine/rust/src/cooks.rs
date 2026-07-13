//! The Stage-1 COOK boundary, native (#20, the usability push; #18's doctrine).
//!
//! Python's `_COOK` table (compiler.py) performs every text→X resolution a
//! translator body needs BEFORE the translator sees its groups: reading→ftid,
//! value-spec parsing, rule-body compilation, the subtype/fact/class cooks.
//! This module is that boundary ported whole, plus the _stmt_translator_impl
//! g-loop body (production match → _plan → ⟨asserts, objs⟩), so
//! op_compile_model translates natively instead of Err-ing "cook not ported".
//!
//! DOCTRINE SPLIT (the mission's note): _reading/_ftid ride main.rs's
//! certified-equal native twins (reading_split/ftid_from — the same functions
//! the prepass already trusts); system:cs_rows and system:sm_rows are REDUCED
//! FROM THE CANON exactly as Python's _cook_cs/_sm_rows reduce them (parity by
//! construction); only what is genuinely host regex in Python — the
//! productions, _num, _slug, the quantifier strips, the rule-body scans — is
//! hand-rolled here, zero-dep, in the skeleton's own style.
//!
//! Every function names its Python original; the acceptance is the per-kind
//! differential (emitted rows verbatim-equal, obj specs from_lam-equal)
//! against python compile_model over the shared/base corpus.

use super::*;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashSet;

// ============================ the data tree ==================================
// Rows and operands as plain data (python tuples of str/int/float), the
// to_lam-able boundary between the cooks and the reducer.
#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    S(String),
    I(i64),
    F(f64),
    T(Vec<Val>),
}

fn vs(s: &str) -> Val {
    Val::S(s.to_string())
}

fn vt(xs: Vec<Val>) -> Val {
    Val::T(xs)
}

pub fn val_to_v(v: &Val) -> V {
    match v {
        Val::S(s) => atom(Leaf::S(s.clone())),
        Val::I(i) => atom(Leaf::I(*i)),
        Val::F(f) => atom(Leaf::F(*f)),
        Val::T(xs) => seq(from_vec(xs.iter().map(val_to_v).collect())),
    }
}

fn v_to_val(v: &V) -> Option<Val> {
    match shape(v) {
        Shape::Atom(l) => Some(match &*l {
            Leaf::S(s) => Val::S(s.clone()),
            Leaf::I(i) => Val::I(*i),
            Leaf::F(f) => Val::F(*f),
            Leaf::AppTag => return None,
        }),
        Shape::Seq(l) => {
            let mut out = Vec::new();
            for x in items(&l) {
                out.push(v_to_val(&x)?);
            }
            Some(Val::T(out))
        }
        Shape::Bot => None,
    }
}

// json of a Val — floats through float_text so 1.0 prints "1.0" like python
pub fn val_json(v: &Val, out: &mut String) {
    match v {
        Val::S(s) => json_escape_into(s, out),
        Val::I(i) => out.push_str(&i.to_string()),
        Val::F(f) => out.push_str(&float_text(*f)),
        Val::T(xs) => {
            out.push('[');
            for (i, x) in xs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                val_json(x, out);
            }
            out.push(']');
        }
    }
}

// json of a reduced V (objs) — same float discipline; bot/app render null
pub fn obj_json(v: &V, out: &mut String) {
    match v_to_val(v) {
        Some(val) => val_json(&val, out),
        None => out.push_str("null"),
    }
}

// python repr() of a str, for the f"{x!r}" diagnostics rows (ruleDiag)
fn py_repr(s: &str) -> String {
    let (q, esc_q) = if s.contains('\'') && !s.contains('"') {
        ('"', '"')
    } else {
        ('\'', '\'')
    };
    let mut out = String::new();
    out.push(q);
    for c in s.chars() {
        if c == '\\' {
            out.push_str("\\\\");
        } else if c == esc_q {
            out.push('\\');
            out.push(c);
        } else {
            out.push(c);
        }
    }
    out.push(q);
    out
}

// ============================ the known context ==============================
// _Known (compiler.py): the type names + the prepass context — subtype
// closure, declared fact-type slugs, plain reading declarations. Built once
// per compile from the driver's prepass; the sorted views are precomputed.
pub struct Known {
    pub names: HashSet<String>,
    words_sorted: Vec<Vec<String>>,           // word count desc (reading scan)
    pairs_sorted: Vec<(String, Vec<String>)>, // same order, with the joined name
    chars_sorted: Vec<String>,                // char count desc (_subject)
    subs: BTreeMap<String, Vec<String>>,      // noun -> sorted ancestors
    fts: BTreeSet<String>,
    plain: BTreeSet<String>,
    vals: HashSet<String>,                    // #31: the value-type names
}

impl Known {
    pub fn new(
        names: &HashSet<String>,
        subs: &BTreeMap<String, BTreeSet<String>>,
        fts: &BTreeSet<String>,
        plain: &BTreeSet<String>,
        vals: &HashSet<String>,
    ) -> Known {
        let mut pairs: Vec<(String, Vec<String>)> = names
            .iter()
            .map(|k| {
                (
                    k.clone(),
                    k.split_whitespace().map(|w| w.to_string()).collect(),
                )
            })
            .collect();
        pairs.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.1.cmp(&b.1)));
        let words: Vec<Vec<String>> = pairs.iter().map(|(_, w)| w.clone()).collect();
        let mut chars: Vec<String> = names.iter().cloned().collect();
        chars.sort_by(|a, b| {
            b.chars()
                .count()
                .cmp(&a.chars().count())
                .then_with(|| a.cmp(b))
        });
        Known {
            names: names.clone(),
            words_sorted: words,
            pairs_sorted: pairs,
            chars_sorted: chars,
            subs: subs
                .iter()
                .map(|(k, v)| (k.clone(), v.iter().cloned().collect()))
                .collect(),
            fts: fts.clone(),
            plain: plain.clone(),
            vals: vals.clone(),
        }
    }
}

// ============================ small string helpers ===========================

// _num (compiler.py): int(s) else float(s) else the stripped string
fn num(s: &str) -> Val {
    let t = s.trim();
    if let Ok(i) = t.parse::<i64>() {
        return Val::I(i);
    }
    // python float() and rust f64 parse agree on the corpus forms; python
    // rejects internal whitespace and empty, as does rust
    if !t.is_empty() && !t.contains(char::is_whitespace) {
        if let Ok(f) = t.parse::<f64>() {
            return Val::F(f);
        }
    }
    Val::S(t.to_string())
}

// re.sub(r"\s+", " ", s).strip()
fn ws_norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn take_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

// zlib.crc32 (the standard IEEE CRC-32), bitwise, zero-dep
fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

// _QUOTED.findall: the contents of sequentially paired 'spans'
fn quoted_findall(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = s;
    loop {
        match rest.find('\'') {
            None => break,
            Some(a) => match rest[a + 1..].find('\'') {
                None => break,
                Some(b) => {
                    out.push(rest[a + 1..a + 1 + b].to_string());
                    rest = &rest[a + 1 + b + 1..];
                }
            },
        }
    }
    out
}

// _QUOTED.sub("", s): remove each paired span, quotes included
fn quoted_sub(s: &str) -> String {
    blank_spans(s, "")
}

// _strip_derivation (compiler.py): (storage kind, name without the marker)
fn strip_derivation(text: &str) -> (Option<&'static str>, String) {
    for (mark, kind) in [
        (" **", "derived-and-stored"),
        (" ++", "partially-derived-and-stored"),
        (" *", "fully-derived"),
        (" +", "semi-derived"),
    ] {
        if let Some(pre) = text.strip_suffix(mark) {
            return (Some(kind), pre.trim().to_string());
        }
    }
    (None, text.to_string())
}

// re.sub(r"\b(w1|w2|…) ", "", t): the quantifier strips (_QUANT/_QUANT_MIN).
// A match needs a word boundary before the word and a literal space after;
// scanning resumes after the consumed span, exactly like re.sub.
fn strip_quant(t: &str, words: &[&str]) -> String {
    let mut out = String::new();
    let mut i = 0usize;
    let mut prev: Option<char> = None;
    'outer: while i < t.len() {
        let boundary = match prev {
            None => true,
            Some(p) => !(p.is_alphanumeric() || p == '_'),
        };
        if boundary {
            for w in words {
                if t[i..].starts_with(w) && t[i + w.len()..].starts_with(' ') {
                    i += w.len() + 1;
                    prev = Some(' ');
                    continue 'outer;
                }
            }
        }
        let c = t[i..].chars().next().unwrap();
        out.push(c);
        prev = Some(c);
        i += c.len_utf8();
    }
    out
}

const QUANT_MIN: [&str; 4] = ["some", "that", "each", "no"];
const QUANT_FULL: [&str; 6] = ["some", "that", "each", "no", "an", "a"];

// _subject (compiler.py): the leading object type + the remainder
fn subject(text: &str, k: &Known) -> (String, String) {
    for kn in &k.chars_sorted {
        if text == kn
            || (text.len() > kn.len()
                && text.starts_with(kn.as_str())
                && text.as_bytes()[kn.len()] == b' ')
        {
            return (kn.clone(), text[kn.len()..].trim().to_string());
        }
    }
    match text.split_once(' ') {
        Some((a, b)) => (a.to_string(), b.to_string()),
        None => (text.to_string(), String::new()),
    }
}

// _conj (compiler.py): naive third-person conjugation of the first word
fn conj(rest: &str) -> String {
    let (head, tail) = match rest.split_once(' ') {
        Some((h, t)) => (h, Some(t)),
        None => (rest, None),
    };
    let es = head.ends_with('s')
        || head.ends_with('x')
        || head.ends_with('z')
        || head.ends_with("ch")
        || head.ends_with("sh");
    let mut out = String::from(head);
    out.push_str(if es { "es" } else { "s" });
    if let Some(t) = tail {
        out.push(' ');
        out.push_str(t);
    }
    out
}

// ============================ the reading machinery ==========================

// _reading via main.rs's certified-equal twin (test_reading_canon)
fn reading(text: &str, k: &Known) -> (String, Vec<String>) {
    reading_split(text, &k.words_sorted, &k.names)
}

// _fact_type (compiler.py): (ftid, declaration rows) with the parallel-ft
// unification (a subtype in a role position lands in the DECLARED supertype
// fact type when exactly one substitution matches)
fn fact_type(rd: &str, k: &Known) -> (String, Vec<(String, Val)>) {
    let (template, roles) = reading(rd, k);
    let ft = ftid_from(&template, &roles);
    if !k.subs.is_empty() && !k.fts.is_empty() && !k.fts.contains(&ft) {
        let mut hits: Vec<String> = Vec::new();
        for (i, p) in roles.iter().enumerate() {
            if let Some(ancs) = k.subs.get(p) {
                for anc in ancs {
                    let mut cand_roles = roles.clone();
                    cand_roles[i] = anc.clone();
                    let cand = ftid_from(&template, &cand_roles);
                    if k.fts.contains(&cand) && !hits.contains(&cand) {
                        hits.push(cand);
                    }
                }
            }
        }
        if hits.len() == 1 {
            return (hits.remove(0), Vec::new());
        }
    }
    let mut decl = vec![(
        "factType".to_string(),
        vt(vec![vs(&ft), vs(&template)]),
    )];
    for (i, r) in roles.iter().enumerate() {
        decl.push((
            "role".to_string(),
            vt(vec![
                vs(&format!("{}.{}", ft, i + 1)),
                vs(&ft),
                Val::I((i + 1) as i64),
                vs(r),
            ]),
        ));
    }
    (ft, decl)
}

// _clause_ft (compiler.py): minimal quantifier strip preferred when declared
fn clause_ft(text: &str, k: &Known) -> String {
    let t = ws_norm(text);
    let (ft_min, _) = fact_type(strip_quant(&t, &QUANT_MIN).trim(), k);
    if k.fts.contains(&ft_min) {
        return ft_min;
    }
    fact_type(strip_quant(&t, &QUANT_FULL).trim(), k).0
}

// _clause_ft_roles (compiler.py): same strip discipline, answers roles too —
// through the raw reading scan (no unification), exactly like the python
fn clause_ft_roles(text: &str, k: &Known) -> (String, Vec<String>) {
    let t = ws_norm(text);
    let mut best: Option<(String, Vec<String>)> = None;
    for words in [&QUANT_MIN[..], &QUANT_FULL[..]] {
        let stripped = strip_quant(&t, words);
        let (template, roles) = reading(stripped.trim(), k);
        let ft = ftid_from(&template, &roles);
        if best.is_none() {
            best = Some((ft.clone(), roles.clone()));
        }
        if k.fts.contains(&ft) {
            return (ft, roles);
        }
    }
    best.unwrap()
}

// ============================ the rule-clause scans ==========================

const QUALIFIERS: [&str; 6] = ["that", "some", "the", "other", "a", "an"];

fn strip_pnc_local(t: &str) -> &str {
    t.trim_matches(|c| c == '.' || c == ';' || c == ':')
}

// _type_span (compiler.py): the longest known type reading from toks[i], its
// last word optionally subscripted → (base, subscript, token span)
fn type_span<'a>(
    toks: &[&str],
    i: usize,
    k: &'a Known,
) -> Option<(&'a str, String, usize)> {
    for (name, kw) in &k.pairs_sorted {
        let n = kw.len();
        if n == 0 {
            continue;
        }
        let last = i + n - 1;
        if last < toks.len()
            && toks[i..last]
                .iter()
                .zip(kw.iter())
                .all(|(a, b)| *a == b.as_str())
        {
            let lt = toks[last];
            let lw = kw[n - 1].as_str();
            if lt.len() >= lw.len()
                && lt.starts_with(lw)
                && lt[lw.len()..].bytes().all(|b| b.is_ascii_digit())
                && atomic_run_guard(toks, i, kw, &k.names)
            {
                return Some((name.as_str(), lt[lw.len()..].to_string(), n));
            }
        }
    }
    None
}

// _quoted_at (compiler.py): the quoted literal starting at toks[i]
fn quoted_at(toks: &[&str], i: usize) -> (String, usize) {
    let mut buf: Vec<&str> = Vec::new();
    for j in i..toks.len() {
        buf.push(toks[j]);
        if toks[j].ends_with('\'') && (j > i || toks[j].chars().count() > 1) {
            let joined = buf.join(" ");
            let cs: Vec<char> = joined.chars().collect();
            let inner: String = cs[1..cs.len() - 1].iter().collect();
            return (inner, j + 1);
        }
    }
    (buf.join(" ").trim_matches('\'').to_string(), toks.len())
}

// _dequalify (compiler.py): the clause with anaphoric qualifiers dropped
fn dequalify(text: &str, k: &Known) -> String {
    let toks: Vec<&str> = text.split_whitespace().collect();
    let mut out: Vec<&str> = Vec::new();
    let mut i = 0usize;
    while i < toks.len() {
        if QUALIFIERS.contains(&toks[i]) && type_span(&toks, i + 1, k).is_some() {
            i += 1;
            continue;
        }
        out.push(toks[i]);
        i += 1;
    }
    out.join(" ")
}

// _rule_atom (compiler.py): a rule clause → (ftid, ordered variables, literal
// restrictions), with the qualifier-strip fallback and the subtype lift
fn rule_atom(text: &str, k: &Known) -> (String, Vec<String>, Vec<(usize, String)>) {
    let toks: Vec<&str> = text.split_whitespace().collect();
    let mut vars_: Vec<String> = Vec::new();
    let mut lits: Vec<(usize, String)> = Vec::new();
    let mut verbatim: Vec<String> = Vec::new();
    let mut stripped: Vec<String> = Vec::new();
    let mut i = 0usize;
    while i < toks.len() {
        let tok = toks[i];
        if QUALIFIERS.contains(&tok) && type_span(&toks, i + 1, k).is_some() {
            verbatim.push(tok.to_string());
            i += 1;
            continue;
        }
        if let Some((base, sub, ln)) = type_span(&toks, i, k) {
            vars_.push(format!("{}{}", base, sub));
            verbatim.push(base.to_string());
            stripped.push(base.to_string());
            i += ln;
            if i < toks.len() && toks[i].starts_with('\'') {
                let (lit, ni) = quoted_at(&toks, i);
                i = ni;
                lits.push((vars_.len() - 1, lit));
            }
            continue;
        }
        verbatim.push(tok.to_string());
        stripped.push(tok.to_string());
        i += 1;
    }
    let (mut ft, _decl) = fact_type(&verbatim.join(" "), k);
    if !k.fts.is_empty() && !k.fts.contains(&ft) {
        let (alt, _) = fact_type(&stripped.join(" "), k);
        if k.fts.contains(&alt) {
            ft = alt;
        }
    }
    if !vars_.is_empty() && !k.fts.is_empty() && !k.fts.contains(&ft) {
        let base: &str = vars_[0].trim_end_matches(|c: char| c.is_ascii_digit());
        let rd = stripped.join(" ");
        if let Some(ancs) = k.subs.get(base) {
            for anc in ancs {
                let lifted = fact_type(&rd.replacen(base, anc, 1), k).0;
                if k.fts.contains(&lifted) {
                    return (lifted, vars_, lits);
                }
            }
        }
    }
    (ft, vars_, lits)
}

// _coercion (compiler.py): a bare 'A is B' over two known types with NO
// declared fact type is an identity binding between the two variables
fn coercion(clause: &str, k: &Known) -> Option<(String, String)> {
    let toks: Vec<&str> = clause.split_whitespace().collect();
    let sa = type_span(&toks, 0, k)?;
    if sa.2 >= toks.len() || toks[sa.2] != "is" {
        return None;
    }
    let sb = type_span(&toks, sa.2 + 1, k)?;
    if sa.2 + 1 + sb.2 != toks.len() {
        return None;
    }
    if !k.fts.is_empty() {
        let (ft, _) = fact_type(&format!("{} is {}", sa.0, sb.0), k);
        if k.fts.contains(&ft) {
            return None;
        }
    }
    Some((format!("{}{}", sa.0, sa.1), format!("{}{}", sb.0, sb.1)))
}

// _role_path (compiler.py): split on ' that '/' who ', each hop 'V some T'
fn role_path(body: &str) -> Vec<(String, Option<String>)> {
    let mut parts: Vec<&str> = Vec::new();
    let mut rest = body;
    loop {
        let pt = rest.find(" that ");
        let pw = rest.find(" who ");
        let (p, l) = match (pt, pw) {
            (Some(a), Some(b)) if a <= b => (a, 6),
            (Some(a), None) => (a, 6),
            (_, Some(b)) => (b, 5),
            (None, None) => break,
        };
        parts.push(&rest[..p]);
        rest = &rest[p + l..];
    }
    parts.push(rest);
    let mut hops = Vec::new();
    for part in parts {
        let part = part.trim();
        // ^(.+?) some (.+)$ — the FIRST ' some ' with nonempty sides
        let mut hop: Option<(String, Option<String>)> = None;
        for (p, _) in part.match_indices(" some ") {
            if p >= 1 && p + 6 < part.len() {
                hop = Some((
                    part[..p].to_string(),
                    Some(part[p + 6..].to_string()),
                ));
                break;
            }
        }
        hops.push(hop.unwrap_or((part.to_string(), None)));
    }
    hops
}

// ============================ the value specs ================================
// _VALUE_SPECS / _value_spec / _vc_range / _enum_member (compiler.py): a
// pattern table over the NORMA value-spec surface; first match wins, else the
// enumeration. Hand-rolled matchers, exact backtracking order.

fn vc_range(lo: Option<Val>, hi: Option<Val>, lo_open: bool, hi_open: bool) -> (String, Val) {
    let side = |b: Option<Val>, open: bool| match b {
        Some(v) => vt(vec![v, vs(if open { "T" } else { "F" })]),
        None => vt(vec![]),
    };
    (
        "constraints:value_range".to_string(),
        vt(vec![Val::I(1), side(lo, lo_open), side(hi, hi_open)]),
    )
}

fn enum_member(v: &str) -> Val {
    let v = v.trim();
    let cs: Vec<char> = v.chars().collect();
    if cs.len() >= 2 && cs[0] == '\'' && cs[cs.len() - 1] == '\'' {
        let inner: String = cs[1..cs.len() - 1].iter().collect();
        return num(&inner);
    }
    num(v)
}

// find the FIRST occurrence of any of `seps` (leftmost; ties impossible for
// these tables) with a nonempty left side; the right side is the rest
fn first_sep<'a>(s: &'a str, seps: &[&str]) -> Option<(&'a str, &'a str)> {
    let mut best: Option<(usize, usize)> = None;
    for sep in seps {
        for (p, _) in s.match_indices(sep) {
            if p >= 1 {
                if best.map_or(true, |(bp, _)| p < bp) {
                    best = Some((p, sep.len()));
                }
                break;
            }
        }
    }
    best.map(|(p, l)| (&s[..p], &s[p + l..]))
}

fn value_spec(spec: &str) -> (String, Val) {
    let spec = spec.trim();
    // [lo..hi] — lazy: split at the FIRST '..'
    if let Some(body) = spec.strip_prefix('[').and_then(|b| b.strip_suffix(']')) {
        if let Some(p) = body.find("..") {
            if p >= 1 && p + 2 < body.len() {
                return vc_range(
                    Some(num(&body[..p])),
                    Some(num(&body[p + 2..])),
                    false,
                    false,
                );
            }
        }
    }
    if let Some(b) = spec.strip_prefix("at least ") {
        // at least (.+?) to at most (.+)
        if let Some((lo, hi)) = first_sep(b, &[" to at most "]) {
            if !hi.is_empty() {
                return vc_range(Some(num(lo)), Some(num(hi)), false, false);
            }
        }
        // at least (.+?) (?:to|and) below (.+)
        if let Some((lo, hi)) = first_sep(b, &[" to below ", " and below "]) {
            if !hi.is_empty() {
                return vc_range(Some(num(lo)), Some(num(hi)), false, true);
            }
        }
    }
    if let Some(b) = spec.strip_prefix("above ") {
        // above (.+?) to at most (.+)
        if let Some((lo, hi)) = first_sep(b, &[" to at most "]) {
            if !hi.is_empty() {
                return vc_range(Some(num(lo)), Some(num(hi)), true, false);
            }
        }
        // above (.+?) (?:to|and) below (.+)
        if let Some((lo, hi)) = first_sep(b, &[" to below ", " and below "]) {
            if !hi.is_empty() {
                return vc_range(Some(num(lo)), Some(num(hi)), true, true);
            }
        }
    }
    if let Some(b) = spec.strip_prefix("at least ") {
        if !b.is_empty() {
            return vc_range(Some(num(b)), None, false, false);
        }
    }
    if let Some(b) = spec.strip_prefix("above ") {
        if !b.is_empty() {
            return vc_range(Some(num(b)), None, true, false);
        }
    }
    if let Some(b) = spec.strip_prefix("at most ") {
        if !b.is_empty() {
            return vc_range(None, Some(num(b)), false, false);
        }
    }
    if let Some(b) = spec.strip_prefix("below ") {
        if !b.is_empty() {
            return vc_range(None, Some(num(b)), false, true);
        }
    }
    // the enumeration: re.split(r",| and ", spec), empties dropped
    let mut members: Vec<Val> = Vec::new();
    let mut rest = spec;
    loop {
        let pc = rest.find(',');
        let pa = rest.find(" and ");
        let (p, l) = match (pc, pa) {
            (Some(a), Some(b)) if a <= b => (a, 1),
            (Some(a), None) => (a, 1),
            (_, Some(b)) => (b, 5),
            (None, None) => break,
        };
        if !rest[..p].trim().is_empty() {
            members.push(enum_member(&rest[..p]));
        }
        rest = &rest[p + l..];
    }
    if !rest.trim().is_empty() {
        members.push(enum_member(rest));
    }
    (
        "constraints:value_enumeration".to_string(),
        vt(vec![Val::I(1), vt(members)]),
    )
}

// ============================ the canon bridge ===============================

// apply a canonical builder through the reducer over the resident store —
// python's _apply(_A(name), to_lam(operand)); ⊥ or a stuck app is a refusal
fn canon_apply(srv: &Srv, name: &str, operand: V) -> Result<V, String> {
    let res = reduce_over(srv, atom(Leaf::S(name.to_string())), operand, None);
    if isbot(&res) || isapp(&res) {
        return Err(format!("canon {} did not reduce", name));
    }
    Ok(res)
}

// C._canon_c: the NULLARY builder's canon VALUE (unapplied — the name IS the
// object; DEFS-resolved at use in python, read off the compiled-in table here)
fn canon_value(name: &str) -> Result<V, String> {
    CANON
        .with(|c| {
            c.borrow()
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.clone())
        })
        .ok_or_else(|| format!("no canon def {}", name))
}

// ============================ the crows shape ================================

#[derive(Clone, Debug)]
pub enum Mid {
    C(Vec<Val>),      // ("c", tail): a constraint-row tail, modality appended
    W(String, Val),   // ("w", (cell, row)): a whole row
}

#[derive(Clone, Debug, Default)]
pub struct Crows {
    decl: Vec<(String, Val)>,
    mid: Vec<Mid>,
    ospecs: Vec<(String, String, Val)>, // ⟨cid, builder, operand⟩; T([]) = nullary
}

type Asserts = Vec<(String, Val)>;
type Objs = Vec<(String, V)>;

// _h_crows (compiler.py): the generic constraint translator over cooked groups
fn h_crows(g: &Crows, m: &str, srv: &Srv) -> Result<(Asserts, Objs), String> {
    let mut rows: Asserts = g.decl.clone();
    for mid in &g.mid {
        match mid {
            Mid::C(tail) => {
                let mut t = tail.clone();
                t.push(vs(m));
                rows.push(("constraint".to_string(), Val::T(t)));
            }
            Mid::W(cell, row) => rows.push((cell.clone(), row.clone())),
        }
    }
    let mut objs: Objs = Vec::new();
    for (cid, b, op) in &g.ospecs {
        let obj = if matches!(op, Val::T(v) if v.is_empty()) {
            canon_value(b)?
        } else {
            canon_apply(srv, b, val_to_v(op))?
        };
        objs.push((cid.clone(), obj));
    }
    Ok((rows, objs))
}

// _mand_specs (compiler.py): the mandatory pair's obj specs
fn mand_specs(mand: &str, ft: &str, subj: &str) -> Vec<(String, String, Val)> {
    vec![
        (
            mand.to_string(),
            "constraints:scoped_mandatory_entities".to_string(),
            vs(subj),
        ),
        (
            format!("{}_e", mand),
            "constraints:scoped_mandatory_facts".to_string(),
            vs(ft),
        ),
    ]
}

// _mandatory_parts (compiler.py): rows + objs of one mandatory constraint
fn mandatory_parts(
    ft: &str,
    subj: &str,
    m: &str,
    srv: &Srv,
) -> Result<(Asserts, Objs), String> {
    let cid = format!("{}_mand", ft);
    let rows = vec![
        (
            "constraint".to_string(),
            vt(vec![vs(&cid), vs("mandatory"), vs(ft), vs(subj), vs(m)]),
        ),
        ("spans".to_string(), vt(vec![vs(&cid), Val::I(1)])),
    ];
    let objs = vec![
        (
            cid.clone(),
            canon_apply(srv, "constraints:scoped_mandatory_entities", atom(Leaf::S(subj.to_string())))?,
        ),
        (
            format!("{}_e", cid),
            canon_apply(srv, "constraints:scoped_mandatory_facts", atom(Leaf::S(ft.to_string())))?,
        ),
    ];
    Ok((rows, objs))
}

// ============================ the cooks ======================================

// _cook_ring
fn cook_ring(g: &[Option<String>], k: &Known) -> (Vec<(String, Val)>, String, String, String, String) {
    let g0 = g[0].as_deref().unwrap_or("");
    let g1 = g[1].as_deref().unwrap_or("");
    let (ft, decl) = fact_type(g0, k);
    (
        decl,
        format!("{}_ring_{}", ft, g1),
        format!("ring_{}", g1),
        ft,
        format!("constraints:ring_{}", g1),
    )
}

// _cook_frequency
fn cook_frequency(g: &[Option<String>], k: &Known) -> (String, String, Vec<i64>, Val) {
    let g0 = g[0].as_deref().unwrap_or("");
    let g1 = g[1].as_deref().unwrap_or("");
    let g2 = g[2].as_deref().unwrap_or("");
    let g3 = g[3].as_deref().unwrap_or("");
    let (template, rtypes) = reading(g0, k);
    let ftn = ftid_from(&template, &rtypes);
    let names: Vec<String> = g1.split(',').map(|s| s.trim().to_string()).collect();
    let mut roles: Vec<i64> = Vec::new();
    for nm in &names {
        if let Some(p) = rtypes.iter().position(|t| t == nm) {
            roles.push((p + 1) as i64);
        }
    }
    if roles.is_empty() {
        roles.push(1);
    }
    let n: i64 = g3.parse().unwrap_or(0);
    let (lo, hi): (Vec<Val>, Vec<Val>) = match g2 {
        "at most" => (vec![], vec![Val::I(n)]),
        "at least" => (vec![Val::I(n)], vec![]),
        _ => (vec![Val::I(n)], vec![Val::I(n)]), // "exactly"
    };
    let rv: Vec<Val> = roles.iter().map(|r| Val::I(*r)).collect();
    (
        format!("{}_freq", ftn),
        ftn,
        roles,
        vt(vec![vt(rv), vt(lo), vt(hi)]),
    )
}

// _cook_value_constraint
fn cook_value_constraint(g: &[Option<String>]) -> (String, String, String, String, Val) {
    let g0 = g[0].as_deref().unwrap_or("");
    let g1 = g[1].as_deref().unwrap_or("");
    let (builder, bop) = value_spec(g1);
    (
        g0.to_string(),
        g1.to_string(),
        format!("{}_vc", g0),
        builder,
        bop,
    )
}

// _cook_uniqueness (+ the 'exactly one' mandatory rider)
fn cook_uniqueness(g: &[Option<String>], k: &Known) -> Crows {
    let g0 = g[0].as_deref().unwrap_or("");
    let g1 = g[1].as_deref().unwrap_or("");
    let g2 = g[2].as_deref().unwrap_or("");
    let rd = format!("{} {}", g0, g2);
    let (ft, decl) = fact_type(&rd, k);
    let (_t, rtypes) = reading(&rd, k);
    let subj = subject(g0, k).0;
    let pos = rtypes
        .iter()
        .position(|t| *t == subj)
        .map(|p| (p + 1) as i64)
        .unwrap_or(1);
    let uc = format!("{}_uc", ft);
    let mut mid = vec![
        Mid::C(vec![vs(&uc), vs("uniqueness"), vs(&ft)]),
        Mid::W("spans".to_string(), vt(vec![vs(&uc), Val::I(pos)])),
    ];
    let mut ospecs = vec![(
        uc.clone(),
        "constraints:uniqueness".to_string(),
        vt(vec![Val::I(pos)]),
    )];
    if g1 == "exactly one" {
        let mand = format!("{}_mand", ft);
        mid.push(Mid::C(vec![vs(&mand), vs("mandatory"), vs(&ft), vs(&subj)]));
        mid.push(Mid::W(
            "spans".to_string(),
            vt(vec![vs(&mand), Val::I(pos)]),
        ));
        ospecs.extend(mand_specs(&mand, &ft, &subj));
    }
    Crows { decl, mid, ospecs }
}

// _cook_mandatory
fn cook_mandatory(g: &[Option<String>], k: &Known) -> Crows {
    let g0 = g[0].as_deref().unwrap_or("");
    let g1 = g[1].as_deref().unwrap_or("");
    let (ft, decl) = fact_type(&format!("{} {}", g0, g1), k);
    let subj = subject(g0, k).0;
    let mand = format!("{}_mand", ft);
    Crows {
        decl,
        mid: vec![
            Mid::C(vec![vs(&mand), vs("mandatory"), vs(&ft), vs(&subj)]),
            Mid::W("spans".to_string(), vt(vec![vs(&mand), Val::I(1)])),
        ],
        ospecs: mand_specs(&mand, &ft, &subj),
    }
}

// _cook_neg_uniqueness: the same uc constraint, NO spans row, NO conditional
fn cook_neg_uniqueness(g: &[Option<String>], k: &Known) -> Crows {
    let joined = g
        .iter()
        .map(|x| x.as_deref().unwrap_or(""))
        .collect::<Vec<_>>()
        .join(" ");
    let (ft, decl) = fact_type(&joined, k);
    let uc = format!("{}_uc", ft);
    Crows {
        decl,
        mid: vec![Mid::C(vec![vs(&uc), vs("uniqueness"), vs(&ft)])],
        ospecs: vec![(
            uc,
            "constraints:uniqueness".to_string(),
            vt(vec![Val::I(1)]),
        )],
    }
}

// _cook_neg_mandatory
fn cook_neg_mandatory(g: &[Option<String>], k: &Known) -> Crows {
    let joined = g
        .iter()
        .map(|x| x.as_deref().unwrap_or(""))
        .collect::<Vec<_>>()
        .join(" ");
    let (ft, decl) = fact_type(&joined, k);
    let subj = subject(g[0].as_deref().unwrap_or(""), k).0;
    let mand = format!("{}_mand", ft);
    Crows {
        decl,
        mid: vec![
            Mid::C(vec![vs(&mand), vs("mandatory"), vs(&ft), vs(&subj)]),
            Mid::W("spans".to_string(), vt(vec![vs(&mand), Val::I(1)])),
        ],
        ospecs: mand_specs(&mand, &ft, &subj),
    }
}

// _cook_for_each_mandatory
fn cook_for_each_mandatory(g: &[Option<String>], k: &Known) -> Crows {
    let subj = g[0].as_deref().unwrap_or("").trim().to_string();
    let clause = dequalify(g[1].as_deref().unwrap_or(""), k);
    let (ft, decl) = fact_type(&clause, k);
    let (_t, rtypes) = reading(&clause, k);
    let pos = rtypes
        .iter()
        .position(|t| *t == subj)
        .map(|p| (p + 1) as i64)
        .unwrap_or(1);
    let mand = format!("{}_mand", ft);
    Crows {
        decl,
        mid: vec![
            Mid::C(vec![vs(&mand), vs("mandatory"), vs(&ft), vs(&subj)]),
            Mid::W("spans".to_string(), vt(vec![vs(&mand), Val::I(pos)])),
        ],
        ospecs: mand_specs(&mand, &ft, &subj),
    }
}

// _cook_inverse_uc
fn cook_inverse_uc(g: &[Option<String>], k: &Known) -> Crows {
    let g0 = g[0].as_deref().unwrap_or("");
    let g1 = g[1].as_deref().unwrap_or("");
    let g2 = g[2].as_deref().unwrap_or("");
    let (a, _r) = subject(g0, k);
    let rd = format!("{} {}", g2, g0);
    let (ft, decl) = fact_type(&rd, k);
    let (_t, rtypes) = reading(&rd, k);
    let pos = rtypes
        .iter()
        .position(|t| *t == a)
        .map(|p| (p + 1) as i64)
        .unwrap_or(2);
    let cid = format!("{}_inv_uc", slug_str(&a));
    let mut mid = vec![
        Mid::C(vec![vs(&cid), vs("uniqueness"), vs(&ft)]),
        Mid::W("spans".to_string(), vt(vec![vs(&cid), Val::I(pos)])),
    ];
    let mut ospecs: Vec<(String, String, Val)> = Vec::new();
    if g1 == "exactly one" {
        let mand = format!("{}_mand", ft);
        mid.push(Mid::C(vec![vs(&mand), vs("mandatory"), vs(&ft), vs(&a)]));
        mid.push(Mid::W(
            "spans".to_string(),
            vt(vec![vs(&mand), Val::I(pos)]),
        ));
        ospecs = mand_specs(&mand, &ft, &a);
    }
    Crows { decl, mid, ospecs }
}

// _uc_columns (compiler.py): resolve named columns; every name must land
fn uc_columns(names: &[String], rtypes: &[String]) -> (Vec<i64>, Vec<String>) {
    let mut roles: Vec<i64> = Vec::new();
    let mut used: BTreeMap<String, usize> = BTreeMap::new();
    let mut missing: Vec<String> = Vec::new();
    for nm in names {
        let occ: Vec<usize> = rtypes
            .iter()
            .enumerate()
            .filter(|(_, t)| *t == nm)
            .map(|(i, _)| i)
            .collect();
        if !occ.is_empty() {
            let u = *used.get(nm).unwrap_or(&0);
            let idx = occ[u.min(occ.len() - 1)];
            roles.push((idx + 1) as i64);
            *used.entry(nm.clone()).or_insert(0) += 1;
        } else {
            missing.push(nm.clone());
        }
    }
    (roles, missing)
}

// _cook_spanning: 'In each population of <reading>, each A, B …'
fn cook_spanning(g: &[Option<String>], k: &Known) -> Result<Crows, String> {
    let g0 = g[0].as_deref().unwrap_or("");
    let g1 = g[1].as_deref().unwrap_or("");
    let ftn = g0.replace(' ', "_");
    let names: Vec<String> = g1.split(',').map(|s| s.trim().to_string()).collect();
    let (_t, rtypes) = reading(g0, k);
    let (roles, missing) = uc_columns(&names, &rtypes);
    if !missing.is_empty() || roles.is_empty() {
        return Err(format!("spanning UC names unresolved roles: {:?}", missing));
    }
    let cid = format!("{}_uc", ftn);
    let mut mid = vec![Mid::C(vec![vs(&cid), vs("spanning_uniqueness"), vs(&ftn)])];
    for p in &roles {
        mid.push(Mid::W(
            "spans".to_string(),
            vt(vec![vs(&cid), Val::I(*p)]),
        ));
    }
    Ok(Crows {
        decl: Vec::new(),
        mid,
        ospecs: vec![(
            cid,
            "constraints:uniqueness".to_string(),
            vt(roles.iter().map(|r| Val::I(*r)).collect()),
        )],
    })
}

// _cook_spanning_corpus: the roles-first spelling; the reading declares
fn cook_spanning_corpus(g: &[Option<String>], k: &Known) -> Result<Crows, String> {
    let g0 = g[0].as_deref().unwrap_or("");
    let g1 = g[1].as_deref().unwrap_or("");
    let names: Vec<String> = g0.split(',').map(|s| s.trim().to_string()).collect();
    let (ftn, decl) = fact_type(g1, k);
    let (_t, rtypes) = reading(g1, k);
    let (roles, missing) = uc_columns(&names, &rtypes);
    if !missing.is_empty() || roles.is_empty() {
        return Err(format!("spanning UC names unresolved roles: {:?}", missing));
    }
    let cid = format!("{}_uc", ftn);
    let mut mid = vec![Mid::C(vec![vs(&cid), vs("spanning_uniqueness"), vs(&ftn)])];
    for p in &roles {
        mid.push(Mid::W(
            "spans".to_string(),
            vt(vec![vs(&cid), Val::I(*p)]),
        ));
    }
    Ok(Crows {
        decl,
        mid,
        ospecs: vec![(
            cid,
            "constraints:uniqueness".to_string(),
            vt(roles.iter().map(|r| Val::I(*r)).collect()),
        )],
    })
}

// the negation cook (the _COOK lambda): one whole row, no objs
fn cook_negation(g: &[Option<String>], k: &Known) -> Crows {
    let g0 = g[0].as_deref().unwrap_or("");
    let g1 = g[1].as_deref().unwrap_or("");
    let (s0, s1) = subject(g0, k);
    Crows {
        decl: Vec::new(),
        mid: vec![Mid::W(
            "negation".to_string(),
            vt(vec![vs(&s0), vs(&format!("{} {}", s1, g1))]),
        )],
        ospecs: Vec::new(),
    }
}

// _cook_subtype: the inclusion rule + the subset check
fn cook_subtype(sub: &str, sup: &str) -> Crows {
    let sub = sub.trim();
    let sup = sup.trim();
    let cid = format!("{}_sub_{}", slug_str(sub), slug_str(sup));
    let rid = format!("{}_isa_{}", slug_str(sub), slug_str(sup));
    let decl = vec![
        (
            "instanceOf".to_string(),
            vt(vec![vs(sub), vs("ObjectType")]),
        ),
        (
            "instanceOf".to_string(),
            vt(vec![vs(sup), vs("ObjectType")]),
        ),
        ("subtype".to_string(), vt(vec![vs(sub), vs(sup)])),
        ("ruleDerives".to_string(), vt(vec![vs(&rid), vs(sup)])),
        ("ruleReads".to_string(), vt(vec![vs(&rid), vs(sub)])),
        (
            "ruleAtom".to_string(),
            vt(vec![vs(&rid), Val::I(1), vs(sub)]),
        ),
        (
            "ruleCopies".to_string(),
            vt(vec![vs(&rid), vs(sub), vs(sup)]),
        ),
    ];
    let atoms = vt(vec![vt(vec![vs(sub), Val::I(1), vt(vec![])])]);
    Crows {
        decl,
        mid: vec![Mid::C(vec![vs(&cid), vs("subtype"), vs(sub), vs(sup)])],
        ospecs: vec![
            (
                cid,
                "constraints:scoped_subset".to_string(),
                vs(sup),
            ),
            (
                rid.clone(),
                "system:compile_rule".to_string(),
                vt(vec![atoms.clone(), vt(vec![Val::I(1)]), vt(vec![])]),
            ),
            (
                format!("{}~d1", rid),
                "system:compile_rule_delta".to_string(),
                vt(vec![atoms, vt(vec![Val::I(1)]), vt(vec![]), Val::I(1)]),
            ),
        ],
    }
}

// _cook_fact: marker strip, quote detection, ids, ft resolution, subtype lift
fn cook_fact(g0: &str, k: &Known) -> Crows {
    let (kind, rd) = strip_derivation(g0);
    if rd.contains('\'') {
        let dequoted = ws_norm(&quoted_sub(&rd));
        let (ft, _decl) = fact_type(&dequoted, k);
        let (_t, rtypes) = reading(&dequoted, k);
        // #31: a quoted literal filling a VALUE-typed role coerces to its
        // native number (quotes are the reading's, not the value's); an
        // entity-typed (reference) role keeps its id verbatim
        let ids: Vec<Val> = quoted_findall(&rd)
            .into_iter()
            .enumerate()
            .map(|(i, v)| {
                if i < rtypes.len() && k.vals.contains(&rtypes[i]) {
                    num(&v)
                } else {
                    Val::S(v)
                }
            })
            .collect();
        if !k.fts.is_empty() && !k.fts.contains(&ft) {
            if let Some(first) = rtypes.first() {
                if let Some(ancs) = k.subs.get(first) {
                    for anc in ancs {
                        let lifted = fact_type(&dequoted.replacen(first.as_str(), anc, 1), k).0;
                        if k.fts.contains(&lifted) {
                            return Crows {
                                decl: Vec::new(),
                                mid: vec![Mid::W(lifted, Val::T(ids))],
                                ospecs: Vec::new(),
                            };
                        }
                    }
                }
            }
        }
        return Crows {
            decl: Vec::new(),
            mid: vec![Mid::W(ft, Val::T(ids))],
            ospecs: Vec::new(),
        };
    }
    let (ft, decl) = fact_type(&rd, k);
    let mid = match kind {
        Some(kd) => vec![Mid::W(
            "derivation".to_string(),
            vt(vec![vs(&ft), vs(kd)]),
        )],
        None => Vec::new(),
    };
    Crows {
        decl,
        mid,
        ospecs: Vec::new(),
    }
}

// _cook_derivation_rule: the linear role-path derivation
fn cook_derivation_rule(g: &[Option<String>], k: &Known) -> Crows {
    let derived = g[0].as_deref().unwrap_or("");
    let root = g[1].as_deref().unwrap_or("");
    let body = g[2].as_deref().unwrap_or("");
    let hops = role_path(body);
    let rule_cid = format!("{}_rule", slug_str(derived));
    let mut decl = vec![
        (
            "instanceOf".to_string(),
            vt(vec![vs(derived), vs("ObjectType")]),
        ),
        (
            "derivation".to_string(),
            vt(vec![vs(&slug_str(derived)), vs("fully-derived")]),
        ),
        (
            "derivationRule".to_string(),
            vt(vec![
                vs(&slug_str(derived)),
                vs(root),
                Val::I(hops.len() as i64),
            ]),
        ),
        (
            "ruleDerives".to_string(),
            vt(vec![vs(&rule_cid), vs(&slug_str(derived))]),
        ),
    ];
    let mut prev = root.to_string();
    for (verb, target) in &hops {
        let rd = match target {
            Some(t) => format!("{} {} {}", prev, verb, t),
            None => format!("{} {}", prev, verb),
        };
        decl.push((
            "ruleReads".to_string(),
            vt(vec![vs(&rule_cid), vs(&clause_ft(&rd, k))]),
        ));
        if let Some(t) = target {
            prev = t.clone();
        }
    }
    let ospecs = if hops.len() == 2 {
        vec![(
            rule_cid,
            "system:join_rule2".to_string(),
            vt(vec![Val::I(2), vt(vec![Val::I(1)])]),
        )]
    } else {
        Vec::new()
    };
    Crows {
        decl,
        mid: Vec::new(),
        ospecs,
    }
}

// _cook_neg_pair: NORMA's unary negation — the paired positive-shaped
// negation fact type with the pair exclusion auto-asserted
fn cook_neg_pair(g: &[Option<String>], k: &Known) -> Crows {
    let subj = g[0].as_deref().unwrap_or("");
    let mode = g[1].as_deref().unwrap_or("");
    let rest = g[2].as_deref().unwrap_or("");
    if !k.names.contains(subj) {
        return cook_fact(&format!("{} {} {}", subj, mode, rest), k);
    }
    let pos_read = if mode == "is not" {
        format!("{} is {}", subj, rest)
    } else {
        format!("{} {}", subj, conj(rest))
    };
    let (pos, decl_p) = fact_type(&pos_read, k);
    let (neg, decl_n) = fact_type(&format!("{} {} {}", subj, mode, rest), k);
    let cid = format!("negx_{}", take_chars(&neg, 40));
    let pair = vt(vec![vs(&pos), vs(&neg)]);
    let mut decl = decl_p;
    decl.extend(decl_n);
    decl.push(("negOf".to_string(), vt(vec![vs(&neg), vs(&pos)])));
    let mid = vec![Mid::W(
        "constraint".to_string(),
        vt(vec![
            vs(&cid),
            vs("exclusion"),
            vs(&neg),
            pair.clone(),
            vs("alethic"),
        ]),
    )];
    let mut ospecs = vec![(
        cid.clone(),
        "constraints:exclusion".to_string(),
        vt(vec![]),
    )];
    for ft in [&pos, &neg] {
        ospecs.push((
            format!("{}@{}", cid, ft),
            "constraints:scoped_exclusion".to_string(),
            vt(vec![pair.clone(), vs(ft)]),
        ));
    }
    Crows {
        decl,
        mid,
        ospecs,
    }
}

// _CLAUSE_RE: ^(\S.*?) has (\S.*?)(?: '(.+?)')?$ — the class-rule body clause
fn clause_re(c: &str) -> Option<(String, String, Option<String>)> {
    if c.is_empty() || c.starts_with(char::is_whitespace) {
        return None;
    }
    for (p, _) in c.match_indices(" has ") {
        if p < 1 {
            continue;
        }
        let g1 = &c[..p];
        let rest = &c[p + 5..];
        if rest.is_empty() || rest.starts_with(char::is_whitespace) {
            continue;
        }
        // (\S.*?)(?: '(.+?)')?$ over rest: lazy g2 — find the EARLIEST split
        // where the remainder is exactly " '<lit>'" reaching the end
        if c.ends_with('\'') {
            let mut best: Option<usize> = None;
            for (q, _) in rest.match_indices(" '") {
                if q >= 1 && q + 2 < rest.len() - 1 {
                    best = Some(q);
                    break;
                }
            }
            if let Some(q) = best {
                return Some((
                    g1.to_string(),
                    rest[..q].to_string(),
                    Some(rest[q + 2..rest.len() - 1].to_string()),
                ));
            }
        }
        return Some((g1.to_string(), rest.to_string(), None));
    }
    None
}

// re.split(r" and (?=(?:[^']*'[^']*')*[^']*$)", body): split at " and " with
// an even number of quotes from the split point to the end
fn split_and_quote_aware(body: &str) -> Vec<&str> {
    let total: usize = body.matches('\'').count();
    let b = body.as_bytes();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut before = 0usize; // quotes before the scan position
    let mut i = 0usize;
    while i + 5 <= b.len() {
        if &b[i..i + 5] == b" and " {
            // " and " carries no quotes: quotes after the match = total - before
            if (total - before) % 2 == 0 {
                out.push(&body[start..i]);
                start = i + 5;
                i += 5;
                continue;
            }
        }
        if b[i] == b'\'' {
            before += 1;
        }
        i += 1;
    }
    out.push(&body[start..]);
    out
}

// _cook_class_rule: the grammar-as-readings recognizer form
fn cook_class_rule(g: &[Option<String>]) -> Crows {
    let subjh = g[0].as_deref().unwrap_or("");
    let fieldh = g[1].as_deref().unwrap_or("");
    let headlit = g[2].as_deref().unwrap_or("");
    let body = g[3].as_deref().unwrap_or("");
    let head_ft = slug_str(&format!("{} has {}", subjh, fieldh));
    let mut clauses: Vec<(String, Option<String>)> = Vec::new();
    for c in split_and_quote_aware(body) {
        match clause_re(c.trim()) {
            None => {
                return Crows::default(); // the host's silent refusal, preserved
            }
            Some((s2, f2, lit)) => {
                clauses.push((slug_str(&format!("{} has {}", s2, f2)), lit));
            }
        }
    }
    let rid = format!(
        "{}_cls_{:x}",
        head_ft,
        crc32(format!("{}|{}", headlit, body).as_bytes())
    );
    let mut decl = vec![(
        "ruleDerives".to_string(),
        vt(vec![vs(&rid), vs(&head_ft)]),
    )];
    for (ftb, lit) in &clauses {
        decl.push(("ruleReads".to_string(), vt(vec![vs(&rid), vs(ftb)])));
        decl.push((
            "classSpec".to_string(),
            vt(vec![
                vs(&rid),
                vs(ftb),
                vs(lit.as_deref().unwrap_or("")),
                vs(headlit),
            ]),
        ));
        if let Some(l) = lit {
            decl.push(("classLit".to_string(), vt(vec![vs(ftb), vs(l)])));
        }
    }
    let pred_clauses = vt(clauses
        .iter()
        .map(|(ftb, lit)| {
            vt(vec![
                vs(ftb),
                match lit {
                    None => vt(vec![]),
                    Some(l) => vt(vec![
                        vs("COMP"),
                        vs("eq"),
                        vt(vec![vs("CONS"), Val::I(2), vt(vec![vs("CONST"), vs(l)])]),
                    ]),
                },
            ])
        })
        .collect());
    Crows {
        decl,
        mid: Vec::new(),
        ospecs: vec![(
            rid,
            "system:class_rule".to_string(),
            vt(vec![pred_clauses, vs(headlit)]),
        )],
    }
}

// ============================ the rule cook (the big one) ====================
// _cook_rule_if (compiler.py): the whole body resolution — clause split,
// column map, comparators-as-filters, coercion aliases, negation groups, the
// aggregate, and the head shape including skolem existentials — cooked to the
// generic crows groups ⟨rows, ⟨⟩, obj_specs⟩.

// python dict semantics for the variable → column map (insertion-ordered)
#[derive(Default, Clone)]
struct Cols(Vec<(String, i64)>);

impl Cols {
    fn get(&self, k: &str) -> Option<i64> {
        self.0.iter().find(|(n, _)| n == k).map(|(_, v)| *v)
    }
    fn contains(&self, k: &str) -> bool {
        self.get(k).is_some()
    }
    fn setdefault(&mut self, k: &str) {
        if !self.contains(k) {
            let v = self.0.len() as i64 + 1;
            self.0.push((k.to_string(), v));
        }
    }
    fn alias(&mut self, k: &str, v: i64) {
        if !self.contains(k) {
            self.0.push((k.to_string(), v));
        }
    }
    fn len(&self) -> usize {
        self.0.len()
    }
    fn keys(&self) -> Vec<String> {
        self.0.iter().map(|(n, _)| n.clone()).collect()
    }
    fn values_sorted(&self) -> Vec<i64> {
        let mut v: Vec<i64> = self.0.iter().map(|(_, c)| *c).collect();
        v.sort();
        v
    }
}

// _fspec: the comparator predicate as canonical DATA
fn fspec_lit(op: &str, col: i64, lit: Val) -> Val {
    vt(vec![
        vs("COMP"),
        vs(op),
        vt(vec![vs("CONS"), Val::I(col), vt(vec![vs("CONST"), lit])]),
    ])
}

fn fspec_col(op: &str, col: i64, col2: i64) -> Val {
    vt(vec![
        vs("COMP"),
        vs(op),
        vt(vec![vs("CONS"), Val::I(col), Val::I(col2)]),
    ])
}

// _atom_specs: ⟨⟨ft, width, join?⟩…⟩ as plain data
#[derive(Clone)]
enum Join {
    Linear,                            // None in python
    General(Vec<(i64, i64)>, Vec<i64>), // (key pairs, fresh projection)
}

fn atom_specs(atom_fts: &[String], widths: &[i64], joins: &[Join]) -> Val {
    let mut out: Vec<Val> = Vec::new();
    for (i, (ft, w)) in atom_fts.iter().zip(widths.iter()).enumerate() {
        let j: Option<&Join> = if i == 0 { None } else { joins.get(i - 1) };
        let jv = match j {
            None | Some(Join::Linear) => vt(vec![]),
            Some(Join::General(pairs, fresh)) => vt(vec![
                vt(pairs
                    .iter()
                    .map(|(a, b)| vt(vec![Val::I(*a), Val::I(*b)]))
                    .collect()),
                vt(fresh.iter().map(|f| Val::I(*f)).collect()),
            ]),
        };
        out.push(vt(vec![vs(ft), Val::I(*w), jv]));
    }
    vt(out)
}

// _AGG_CLAUSE: ^(.+?) is the (min|max|count|sum) of (.+)$
fn agg_clause_match(c: &str) -> Option<(String, String, String)> {
    for (p, _) in c.match_indices(" is the ") {
        if p < 1 {
            continue;
        }
        let rest = &c[p + 8..];
        for op in ["min", "max", "count", "sum"] {
            if let Some(tail) = rest.strip_prefix(op) {
                if let Some(over) = tail.strip_prefix(" of ") {
                    if !over.is_empty() {
                        return Some((c[..p].to_string(), op.to_string(), over.to_string()));
                    }
                }
            }
        }
    }
    None
}

// _CMP_CLAUSE: ^(\S*\d\S*) (exceeds|is greater than|is less than|is at least|
// is at most|equals) (\S+)$
fn cmp_clause_match(c: &str) -> Option<(String, &'static str, String)> {
    let sp = c.find(' ')?;
    let g1 = &c[..sp];
    if g1.is_empty() || !g1.bytes().any(|b| b.is_ascii_digit()) {
        return None;
    }
    let rest = &c[sp + 1..];
    for (opw, op) in [
        ("exceeds", "gt"),
        ("is greater than", "gt"),
        ("is less than", "lt"),
        ("is at least", "ge"),
        ("is at most", "le"),
        ("equals", "eq"),
    ] {
        if let Some(tail) = rest.strip_prefix(opw) {
            if let Some(objtxt) = tail.strip_prefix(' ') {
                if !objtxt.is_empty() && !objtxt.contains(char::is_whitespace) {
                    return Some((g1.to_string(), op, objtxt.to_string()));
                }
            }
        }
    }
    None
}

// re.search(r"\bat most 0 (.+)$", frag)
fn at_most_zero(frag: &str) -> Option<String> {
    for (p, _) in frag.match_indices("at most 0 ") {
        let boundary = p == 0 || {
            let prev = frag[..p].chars().last().unwrap();
            !(prev.is_alphanumeric() || prev == '_')
        };
        if boundary && p + 10 < frag.len() {
            return Some(frag[p + 10..].to_string());
        }
    }
    None
}

#[allow(clippy::type_complexity)]
fn cook_rule_if(head_txt: &str, body: &str, k: &Known, kind: &str) -> Crows {
    // clause split: ' and ' at top level; 'no ' groups and the 'at most 0'
    // idiom become negation groups; ' where ' folds per its scope
    let mut clauses: Vec<String> = Vec::new();
    let mut neg_groups: Vec<(Vec<String>, Option<String>)> = Vec::new();
    for frag in body.split(" and ").map(|c| c.trim()) {
        let mm0 = at_most_zero(frag);
        if let Some(rest3) = frag.strip_prefix("no ") {
            neg_groups.push((
                rest3.split(" where ").map(|p| p.trim().to_string()).collect(),
                None,
            ));
        } else if let Some(counted) = mm0 {
            neg_groups.push((
                vec![frag.replacen("at most 0 ", "", 1)],
                Some(counted.trim().to_string()),
            ));
        } else if frag.contains(" where ") {
            clauses.extend(frag.split(" where ").map(|p| p.trim().to_string()));
        } else {
            clauses.push(frag.to_string());
        }
    }
    let (hft, hvars, hlits) = rule_atom(head_txt, k);
    let rule_cid = format!("{}_rule_{:x}", hft, crc32(body.as_bytes()));
    let digitless: String = head_txt.chars().filter(|c| !c.is_ascii_digit()).collect();
    let (_hf, decl) = fact_type(digitless.trim(), k);
    let head_is_new = !k.plain.contains(&hft);
    let mut a_rows: Vec<(String, Val)> = decl;
    if head_is_new {
        a_rows.push((
            "derivation".to_string(),
            vt(vec![vs(&hft), vs(kind)]),
        ));
    }
    a_rows.push((
        "ruleDerives".to_string(),
        vt(vec![vs(&rule_cid), vs(&hft)]),
    ));
    // one pass, clauses in order; the aggregate extracted first, processed last
    let mut cols = Cols::default();
    let mut atoms: Vec<(String, Vec<String>)> = Vec::new();
    let mut filters: Vec<Val> = Vec::new();
    let mut joins: Vec<Join> = Vec::new();
    let mut ok = true;
    let mut diag: Option<String> = None;
    let mut agg: Option<(String, i64, String)> = None;
    let agg_clause: Option<String> = clauses.iter().find(|c| agg_clause_match(c).is_some()).cloned();
    if let Some(ac) = &agg_clause {
        clauses.retain(|c| c != ac);
    }
    for c in &clauses {
        if let Some((subj, opw, objtxt)) = cmp_clause_match(c) {
            if let Some(scol) = cols.get(&subj) {
                if let Some(ocol) = cols.get(&objtxt) {
                    filters.push(fspec_col(opw, scol, ocol));
                } else {
                    let lit = num(&objtxt);
                    if matches!(lit, Val::S(_)) {
                        ok = false;
                        diag = Some(format!(
                            "comparator operand {} is neither a bound variable nor a literal",
                            py_repr(&objtxt)
                        ));
                        break;
                    }
                    filters.push(fspec_lit(opw, scol, lit));
                }
                continue;
            }
        }
        if let Some((a, b)) = coercion(c, k) {
            let (ca, cb) = (cols.get(&a), cols.get(&b));
            match (ca, cb) {
                (Some(x), Some(y)) => filters.push(fspec_col("eq", x, y)),
                (Some(x), None) => cols.alias(&b, x),
                (None, Some(y)) => cols.alias(&a, y),
                (None, None) => {
                    ok = false;
                    diag = Some(format!("coercion clause {} has no bound side", py_repr(c)));
                    break;
                }
            }
            continue;
        }
        let (aft, avars, alits) = rule_atom(c, k);
        a_rows.push((
            "ruleReads".to_string(),
            vt(vec![vs(&rule_cid), vs(&aft)]),
        ));
        if atoms.is_empty() {
            for v in &avars {
                cols.setdefault(v);
            }
        } else if !avars.is_empty()
            && cols.get(&avars[0]) == Some(cols.len() as i64)
            && avars.iter().collect::<HashSet<_>>().len() == avars.len()
            && avars[1..].iter().all(|v| !cols.contains(v))
        {
            // the linear chain: NatJoin on the running tuple's last column
            joins.push(Join::Linear);
            for v in &avars[1..] {
                cols.setdefault(v);
            }
        } else {
            // the general conjunctive shape: join on EVERY bound variable
            let pairs: Vec<(i64, i64)> = avars
                .iter()
                .enumerate()
                .filter_map(|(i, v)| cols.get(v).map(|c0| (c0, (i + 1) as i64)))
                .collect();
            let mut fresh: Vec<i64> = Vec::new();
            let mut seen: HashSet<&String> = HashSet::new();
            for (i, v) in avars.iter().enumerate() {
                if !cols.contains(v) && !seen.contains(v) {
                    fresh.push((i + 1) as i64);
                    seen.insert(v);
                }
            }
            joins.push(Join::General(pairs, fresh));
            for v in &avars {
                cols.setdefault(v);
            }
        }
        for (vi, lit) in &alits {
            let col = cols.get(&avars[*vi]).unwrap_or(1);
            filters.push(fspec_lit("eq", col, num(lit)));
        }
        atoms.push((aft, avars));
    }
    // negation groups compile AFTER the positive body binds its columns
    #[allow(clippy::type_complexity)]
    let mut negs: Vec<(Vec<String>, Vec<i64>, Vec<i64>, Vec<Val>, Vec<Join>, Vec<i64>)> =
        Vec::new();
    if ok && !neg_groups.is_empty() && !atoms.is_empty() {
        'groups: for (parts, subject_override) in &neg_groups {
            let mut gatoms: Vec<(String, Vec<String>)> = Vec::new();
            let mut gcols = Cols::default();
            let mut gfilters: Vec<Val> = Vec::new();
            let mut gjoins: Vec<Join> = Vec::new();
            let mut subj: Option<String> = subject_override.clone();
            for (ci, c) in parts.iter().enumerate() {
                let (aft, avars, alits) = rule_atom(c, k);
                a_rows.push((
                    "ruleReads".to_string(),
                    vt(vec![vs(&rule_cid), vs(&aft)]),
                ));
                if ci == 0 && subj.is_none() {
                    subj = avars.first().cloned();
                }
                if gatoms.is_empty() {
                    for v in &avars {
                        gcols.setdefault(v);
                    }
                } else {
                    let pairs: Vec<(i64, i64)> = avars
                        .iter()
                        .enumerate()
                        .filter_map(|(i, v)| gcols.get(v).map(|c0| (c0, (i + 1) as i64)))
                        .collect();
                    let mut fresh: Vec<i64> = Vec::new();
                    let mut seen: HashSet<&String> = HashSet::new();
                    for (i, v) in avars.iter().enumerate() {
                        if !gcols.contains(v) && !seen.contains(v) {
                            fresh.push((i + 1) as i64);
                            seen.insert(v);
                        }
                    }
                    gjoins.push(Join::General(pairs, fresh));
                    for v in &avars {
                        gcols.setdefault(v);
                    }
                }
                for (vi, lit) in &alits {
                    let col = gcols.get(&avars[*vi]).unwrap_or(1);
                    gfilters.push(fspec_lit("eq", col, num(lit)));
                }
                gatoms.push((aft, avars));
            }
            let shared: Vec<String> = gcols
                .keys()
                .into_iter()
                .filter(|v| cols.contains(v) && Some(v.as_str()) != subj.as_deref())
                .collect();
            if shared.is_empty() {
                ok = false;
                diag = Some("negation group shares no bound variable with the body".to_string());
                break 'groups;
            }
            let gwidths: Vec<i64> = gatoms
                .iter()
                .map(|(_, av)| av.len().max(1) as i64)
                .collect();
            negs.push((
                gatoms.iter().map(|(f, _)| f.clone()).collect(),
                shared.iter().map(|v| gcols.get(v).unwrap()).collect(),
                gwidths,
                gfilters,
                gjoins,
                shared.iter().map(|v| cols.get(v).unwrap()).collect(),
            ));
        }
    }
    if ok {
        if let Some(ac) = &agg_clause {
            let (out_v, op, over_v) = agg_clause_match(ac).unwrap();
            if !neg_groups.is_empty() {
                ok = false;
                diag = Some("an aggregate with a negation group is not supported".to_string());
            } else if cols.contains(&over_v) && !cols.contains(&out_v) {
                agg = Some((op, cols.get(&over_v).unwrap(), out_v));
            } else {
                ok = false;
                diag = Some(format!(
                    "aggregate clause needs a bound source and an unbound output ({})",
                    py_repr(ac)
                ));
            }
        }
    }
    let widths: Vec<i64> = atoms.iter().map(|(_, av)| av.len().max(1) as i64).collect();
    let aspecs = atom_specs(
        &atoms.iter().map(|(f, _)| f.clone()).collect::<Vec<_>>(),
        &widths,
        &joins,
    );
    let mut obj: Option<(String, Vec<Val>)> = None;
    let fixed_idx: HashSet<usize> = hlits.iter().map(|(vi, _)| *vi).collect();
    if ok && !atoms.is_empty() && agg.is_some() {
        let (op, over_col, out_v) = agg.clone().unwrap();
        let rest: Vec<String> = hvars.iter().filter(|v| **v != out_v).cloned().collect();
        if rest.iter().all(|v| cols.contains(v)) {
            a_rows.push((
                "derivationRule".to_string(),
                vt(vec![vs(&hft), vs(&atoms[0].0), Val::I(atoms.len() as i64)]),
            ));
            a_rows.push(("ruleAgg".to_string(), vt(vec![vs(&rule_cid)])));
            return Crows {
                decl: a_rows,
                mid: Vec::new(),
                ospecs: vec![(
                    rule_cid,
                    "system:compile_agg_rule".to_string(),
                    vt(vec![
                        aspecs,
                        vt(rest.iter().map(|v| Val::I(cols.get(v).unwrap())).collect()),
                        Val::I(over_col),
                        vs(&op),
                        vt(filters.clone()),
                    ]),
                )],
            };
        }
        diag = Some(format!(
            "aggregate head variables unbound or output {} not in head",
            py_repr(&out_v)
        ));
    } else if ok
        && !atoms.is_empty()
        && hvars
            .iter()
            .enumerate()
            .all(|(i, v)| fixed_idx.contains(&i) || cols.contains(v))
    {
        a_rows.push((
            "derivationRule".to_string(),
            vt(vec![vs(&hft), vs(&atoms[0].0), Val::I(atoms.len() as i64)]),
        ));
        let litmap: BTreeMap<usize, &String> = hlits.iter().map(|(vi, l)| (*vi, l)).collect();
        let proj: Vec<Val> = hvars
            .iter()
            .enumerate()
            .map(|(i, v)| match litmap.get(&i) {
                Some(l) => vt(vec![vs("CONST"), num(l)]),
                None => Val::I(cols.get(v).unwrap()),
            })
            .collect();
        if !negs.is_empty() {
            a_rows.push(("ruleNeg".to_string(), vt(vec![vs(&rule_cid)])));
            let negspecs = vt(negs
                .iter()
                .map(|(nfts, nproj, nwidths, nfilters, njoins, anti_key)| {
                    vt(vec![
                        atom_specs(nfts, nwidths, njoins),
                        vt(nproj.iter().map(|p| Val::I(*p)).collect()),
                        vt(nfilters.clone()),
                        vt(vec![
                            vt(anti_key.iter().map(|a| Val::I(*a)).collect()),
                            vt((1..=nproj.len() as i64).map(Val::I).collect()),
                        ]),
                    ])
                })
                .collect());
            return Crows {
                decl: a_rows,
                mid: Vec::new(),
                ospecs: vec![(
                    rule_cid,
                    "system:compile_rule_neg".to_string(),
                    vt(vec![
                        aspecs,
                        vt(proj),
                        vt((1..=cols.len() as i64).map(Val::I).collect()),
                        vt(filters.clone()),
                        negspecs,
                    ]),
                )],
            };
        }
        let identity = proj.len() == widths[0] as usize
            && proj
                .iter()
                .enumerate()
                .all(|(i, p)| matches!(p, Val::I(c) if *c == (i as i64 + 1)));
        if atoms.len() == 1 && filters.is_empty() && identity {
            a_rows.push((
                "ruleCopies".to_string(),
                vt(vec![vs(&rule_cid), vs(&atoms[0].0), vs(&hft)]),
            ));
        }
        obj = Some((
            "system:compile_rule".to_string(),
            vec![aspecs.clone(), vt(proj), vt(filters.clone())],
        ));
    } else if ok && !atoms.is_empty() && negs.is_empty() && agg.is_none() {
        // EXISTENTIAL (TGD) heads: unbound head variables are SKOLEM roles
        let litmap: BTreeMap<usize, &String> = hlits.iter().map(|(vi, l)| (*vi, l)).collect();
        let frontier: Vec<i64> = cols.values_sorted();
        a_rows.push((
            "derivationRule".to_string(),
            vt(vec![vs(&hft), vs(&atoms[0].0), Val::I(atoms.len() as i64)]),
        ));
        a_rows.push((
            "ruleSkolem".to_string(),
            vt(vec![vs(&rule_cid), vs(&hft)]),
        ));
        let sk = |v: &str| {
            let mut inner = vec![vs("CONS"), vt(vec![vs("CONST"), vs(v)])];
            inner.extend(frontier.iter().map(|f| Val::I(*f)));
            vt(vec![vs("COMP"), vs("skolem"), vt(inner)])
        };
        let proj: Vec<Val> = hvars
            .iter()
            .enumerate()
            .map(|(i, v)| {
                if fixed_idx.contains(&i) {
                    vt(vec![vs("CONST"), num(litmap.get(&i).unwrap())])
                } else if let Some(c) = cols.get(v) {
                    Val::I(c)
                } else {
                    sk(v)
                }
            })
            .collect();
        obj = Some((
            "system:compile_rule".to_string(),
            vec![aspecs.clone(), vt(proj), vt(filters.clone())],
        ));
    } else if ok {
        let fixed: HashSet<&String> = hlits
            .iter()
            .filter(|(vi, _)| *vi < hvars.len())
            .map(|(vi, _)| &hvars[*vi])
            .collect();
        let unbound: Vec<&String> = if !atoms.is_empty() {
            let mut u: Vec<&String> = hvars
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .filter(|v| !cols.contains(v) && !fixed.contains(*v))
                .collect();
            u.sort();
            u
        } else {
            Vec::new()
        };
        diag = Some(if !unbound.is_empty() {
            format!(
                "head variable(s) [{}] unbound in the body",
                unbound
                    .iter()
                    .map(|v| py_repr(v))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else {
            "no fact-type clause in the body".to_string()
        });
    }
    match obj {
        None => {
            if let Some(d) = diag {
                a_rows.push((
                    "ruleDiag".to_string(),
                    vt(vec![vs(&rule_cid), vs(&d)]),
                ));
            }
            Crows {
                decl: a_rows,
                mid: Vec::new(),
                ospecs: Vec::new(),
            }
        }
        Some((oname, oparts)) => {
            let mut ospecs = vec![(
                rule_cid.clone(),
                oname,
                vt(oparts.clone()),
            )];
            for (i, (aft, _av)) in atoms.iter().enumerate() {
                a_rows.push((
                    "ruleAtom".to_string(),
                    vt(vec![vs(&rule_cid), Val::I((i + 1) as i64), vs(aft)]),
                ));
                let mut dparts = oparts.clone();
                dparts.push(Val::I((i + 1) as i64));
                ospecs.push((
                    format!("{}~d{}", rule_cid, i + 1),
                    "system:compile_rule_delta".to_string(),
                    vt(dparts),
                ));
            }
            Crows {
                decl: a_rows,
                mid: Vec::new(),
                ospecs,
            }
        }
    }
}

// _MARKER_KIND (compiler.py)
fn marker_kind(marker: &str) -> &'static str {
    match marker {
        "**" => "derived-and-stored",
        "+" => "semi-derived",
        "++" => "partially-derived-and-stored",
        _ => "fully-derived", // "*" and the unmarked default
    }
}

// ============================ the set-comparison family ======================
// _cook_cs (compiler.py): system:cs_rows REDUCED FROM THE CANON (python's own
// path — parity by construction), the cid mint and per-attach operands here.

fn cs_prefix(kind: &str) -> &'static str {
    match kind {
        "disjunctive_mandatory" => "ior_",
        "subset" => "subset_",
        "equality" => "eq_",
        _ => "",
    }
}

fn cook_cs(
    kind: &str,
    subj: &str,
    clause_fts: &[String],
    raws: &[String],
    srv: &Srv,
) -> Result<Crows, String> {
    let operand = vt(vec![
        vs(kind),
        vs(subj),
        vt(clause_fts.iter().map(|c| vs(c)).collect()),
        vt(raws.iter().map(|r| vs(r)).collect()),
        vs(""),
    ]);
    let rows_v = canon_apply(srv, "system:cs_rows", val_to_v(&operand))?;
    let rows = match v_to_val(&rows_v) {
        Some(Val::T(rs)) if !rs.is_empty() => rs,
        _ => return Err("system:cs_rows answered no rows".to_string()),
    };
    let arow = match &rows[0] {
        Val::T(a) if a.len() >= 5 => a.clone(),
        _ => return Err("cs_rows A-row malformed".to_string()),
    };
    let cid = match &arow[1] {
        Val::S(s) => s.clone(),
        _ => return Err("cs_rows cid not a string".to_string()),
    };
    let pre = cs_prefix(kind);
    let minted = if !pre.is_empty() {
        format!(
            "{}{}",
            pre,
            take_chars(cid.strip_prefix(pre).unwrap_or(&cid), 40)
        )
    } else {
        cid.clone()
    };
    let clauses = arow[4].clone();
    let mid = vec![Mid::C(vec![
        vs(&minted),
        arow[2].clone(),
        arow[3].clone(),
        clauses.clone(),
    ])];
    let mut ospecs: Vec<(String, String, Val)> = Vec::new();
    for att in &rows[1..] {
        let (cell, builder) = match att {
            Val::T(a) if a.len() >= 3 => match (&a[1], &a[2]) {
                (Val::S(c), Val::S(b)) => (c.clone(), b.clone()),
                _ => return Err("cs_rows attach malformed".to_string()),
            },
            _ => return Err("cs_rows attach malformed".to_string()),
        };
        let mut ft: Option<Val> = cell
            .split_once('@')
            .map(|(_, f)| vs(f));
        if builder == "scoped_equality_side" {
            ft = Some(if cell.ends_with("_a") {
                arow[4].clone()
            } else {
                arow[3].clone()
            });
        }
        let (b, op): (String, Val) = match builder.as_str() {
            "exclusion" => ("constraints:exclusion".to_string(), vt(vec![])),
            "exclusive_or" => ("constraints:exclusive_or".to_string(), vt(vec![])),
            "inclusive_or" => ("constraints:inclusive_or".to_string(), vt(vec![])),
            "scoped_exclusion" => (
                "constraints:scoped_exclusion".to_string(),
                vt(vec![clauses.clone(), ft.clone().unwrap_or(vs(""))]),
            ),
            "scoped_exclusive_or" => (
                "constraints:scoped_exclusive_or".to_string(),
                vt(vec![
                    arow[3].clone(),
                    clauses.clone(),
                    ft.clone().unwrap_or(vs("")),
                ]),
            ),
            "scoped_inclusive_or" => (
                "constraints:scoped_inclusive_or".to_string(),
                vt(vec![
                    arow[3].clone(),
                    clauses.clone(),
                    ft.clone().unwrap_or(vs("")),
                ]),
            ),
            "scoped_subset" => (
                "constraints:scoped_subset".to_string(),
                arow[4].clone(),
            ),
            "scoped_equality_side" => (
                "constraints:scoped_equality_side".to_string(),
                ft.clone().unwrap_or(vs("")),
            ),
            other => return Err(format!("cs_rows names unknown builder {}", other)),
        };
        ospecs.push((cell.replacen(&cid, &minted, 1), b, op));
    }
    Ok(Crows {
        decl: Vec::new(),
        mid,
        ospecs,
    })
}

// ============================ handlers beyond crows ==========================

// _sm_rows (compiler.py): system:sm_rows reduced from the canon
fn sm_rows(verb: &str, head: &str, l1: &str, l2: &str, srv: &Srv) -> Result<(Asserts, Objs), String> {
    let operand = vt(vec![vs(verb), vs(head), vs(l1), vs(l2)]);
    let rows_v = canon_apply(srv, "system:sm_rows", val_to_v(&operand))?;
    let rows = match v_to_val(&rows_v) {
        Some(Val::T(rs)) => rs,
        _ => return Err("system:sm_rows answered no rows".to_string()),
    };
    let mut asserts: Asserts = Vec::new();
    for r in rows {
        match r {
            Val::T(pair) if pair.len() == 2 => {
                if let Val::S(cell) = &pair[0] {
                    asserts.push((cell.clone(), pair[1].clone()));
                } else {
                    return Err("sm_rows cell not a string".to_string());
                }
            }
            _ => return Err("sm_rows row malformed".to_string()),
        }
    }
    Ok((asserts, Vec::new()))
}

// _h_brace_subtypes: each subtype link + the optional pairwise exclusion
fn h_brace_subtypes(
    g: &[Option<String>],
    m: &str,
    srv: &Srv,
) -> Result<(Asserts, Objs), String> {
    let g0 = g[0].as_deref().unwrap_or("");
    let marked = g[1].as_deref().map_or(false, |s| !s.is_empty());
    let g2 = g[2].as_deref().unwrap_or("");
    let subs: Vec<String> = g0.split(',').map(|s| s.trim().to_string()).collect();
    let mut asserts: Asserts = Vec::new();
    let mut objs: Objs = Vec::new();
    for s in &subs {
        let (a, o) = h_crows(&cook_subtype(s, g2), m, srv)?;
        asserts.extend(a);
        objs.extend(o);
    }
    if marked {
        let cid = format!("sxc_{}", take_chars(&slug_str(&subs.join("_")), 40));
        let subs_val = vt(subs.iter().map(|s| vs(s)).collect());
        asserts.push((
            "constraint".to_string(),
            vt(vec![
                vs(&cid),
                vs("exclusion"),
                vs(&subs[0]),
                subs_val.clone(),
                vs(m),
            ]),
        ));
        for s in &subs {
            asserts.push((
                "subtypePartition".to_string(),
                vt(vec![vs(s), vs(g2.trim())]),
            ));
        }
        objs.push((cid.clone(), canon_value("constraints:exclusion")?));
        for s in &subs {
            objs.push((
                format!("{}@{}", cid, s),
                canon_apply(
                    srv,
                    "constraints:scoped_exclusion",
                    val_to_v(&vt(vec![subs_val.clone(), vs(s)])),
                )?,
            ));
        }
    }
    Ok((asserts, objs))
}

// _cs_call: the set-comparison family through the crows path
fn cs_call(
    kind: &str,
    subj: &str,
    clause_fts: &[String],
    raws: &[String],
    m: &str,
    srv: &Srv,
) -> Result<(Asserts, Objs), String> {
    let crows = cook_cs(kind, subj, clause_fts, raws, srv)?;
    h_crows(&crows, m, srv)
}

// _h_set_comparison
fn h_set_comparison(
    g: &[Option<String>],
    m: &str,
    k: &Known,
    srv: &Srv,
) -> Result<(Asserts, Objs), String> {
    let subj = g[0].as_deref().unwrap_or("");
    let mode = g[1].as_deref().unwrap_or("");
    let body = g[2].as_deref().unwrap_or("");
    let mut raws: Vec<String> = Vec::new();
    let mut fts: Vec<String> = Vec::new();
    for c in body.split(';') {
        if !c.trim().is_empty() {
            raws.push(c.trim().to_string());
            fts.push(clause_ft(c, k));
        }
    }
    cs_call(mode, subj, &fts, &raws, m, srv)
}

// _h_disjunctive
fn h_disjunctive(
    g: &[Option<String>],
    m: &str,
    k: &Known,
    srv: &Srv,
) -> Result<(Asserts, Objs), String> {
    let body = g.last().and_then(|x| x.as_deref()).unwrap_or("");
    let (subj, rest) = if g.len() == 1 {
        subject(body, k)
    } else {
        (subject(g[0].as_deref().unwrap_or(""), k).0, body.to_string())
    };
    let mut raws: Vec<String> = Vec::new();
    let mut fts: Vec<String> = Vec::new();
    for c in rest.split(" or ") {
        if !c.trim().is_empty() {
            raws.push(format!("{} {}", subj, c.trim()));
            fts.push(clause_ft(&format!("{} {}", subj, c), k));
        }
    }
    cs_call("disjunctive_mandatory", &subj, &fts, &raws, m, srv)
}

// _h_equality
fn h_equality(
    g: &[Option<String>],
    m: &str,
    k: &Known,
    srv: &Srv,
) -> Result<(Asserts, Objs), String> {
    let g0 = g[0].as_deref().unwrap_or("");
    let g1 = g[1].as_deref().unwrap_or("");
    cs_call(
        "equality",
        "",
        &[clause_ft(g0, k), clause_ft(g1, k)],
        &[g0.to_string(), g1.to_string()],
        m,
        srv,
    )
}

// _ANAPHOR: \bthat ((?:[A-Z][\w-]*)(?: [A-Z][\w-]*)*) — finditer, the
// left-to-right non-overlapping scan
fn anaphors(text: &str) -> Vec<String> {
    let cs: Vec<(usize, char)> = text.char_indices().collect();
    let n = cs.len();
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let tokch = |c: char| c.is_alphanumeric() || c == '_' || c == '-';
    let mut out = Vec::new();
    let mut ci = 0usize;
    while ci < n {
        let (bi, _) = cs[ci];
        let boundary = ci == 0 || !word(cs[ci - 1].1);
        if boundary && text[bi..].starts_with("that ") {
            let mut j = ci + 5; // "that " is ascii: 5 chars
            if j < n && cs[j].1.is_uppercase() {
                let start_b = cs[j].0;
                j += 1;
                while j < n && tokch(cs[j].1) {
                    j += 1;
                }
                while j + 1 < n && cs[j].1 == ' ' && cs[j + 1].1.is_uppercase() {
                    j += 2;
                    while j < n && tokch(cs[j].1) {
                        j += 1;
                    }
                }
                let end_b = if j < n { cs[j].0 } else { text.len() };
                out.push(text[start_b..end_b].to_string());
                ci = j;
                continue;
            }
        }
        ci += 1;
    }
    out
}

// _h_subset_trailing (compiler.py): the sign picks subset vs exclusion; the
// projected builders ride as apply-SPECs over the cs_rows subset shape
fn h_subset_trailing(
    g: &[Option<String>],
    k: &Known,
    m: &str,
    sign: &str,
    srv: &Srv,
) -> Result<(Asserts, Objs), String> {
    // #34: groups are [marker, head, cond] (marker optional, mirroring Python).
    // A leading storage marker makes the head DERIVED — a value-headed stored
    // derivation, not a subset CHECK. Classify it here (not the catch-all) and
    // refuse LOUDLY until the value-headed derivation build lands.
    if g[0].is_some() {
        let head_txt = g[1].as_deref().unwrap_or("");
        return Err(format!(
            "marked (stored-derivation) trailing-if awaits the value-headed derivation build (#34): {}",
            take_chars(head_txt, 60)
        ));
    }
    let head_txt = g[1].as_deref().unwrap_or("");
    let cond_txt = g[2].as_deref().unwrap_or("");
    if head_txt.contains('\'') {
        return Err(format!(
            "value-restricted HEAD awaits its slice: {}",
            take_chars(head_txt, 60)
        ));
    }
    if cond_txt.contains(" and ") || cond_txt.contains(" or ") {
        return Err(format!(
            "compound subset condition awaits the join slice: {}",
            take_chars(cond_txt, 60)
        ));
    }
    let lits = quoted_findall(cond_txt);
    let mut filter_lit: Option<String> = None;
    let mut cond_ft_txt = cond_txt.to_string();
    if !lits.is_empty() {
        if lits.len() > 1 {
            return Err(format!(
                "multi-literal condition awaits its slice: {}",
                take_chars(cond_txt, 60)
            ));
        }
        filter_lit = Some(lits[0].clone());
        cond_ft_txt = ws_norm(&quoted_sub(cond_txt));
    }
    let (x_ft, x_roles) = clause_ft_roles(head_txt, k);
    if !k.fts.contains(&x_ft) {
        return Err(format!(
            "subset head does not resolve to a declared fact type: {}",
            take_chars(head_txt, 60)
        ));
    }
    if !k.plain.contains(&x_ft) {
        return Err(format!(
            "derived head: the rule path owns the implication clause: {}",
            take_chars(head_txt, 60)
        ));
    }
    let (y_ft, y_roles) = clause_ft_roles(&cond_ft_txt, k);
    if !k.fts.contains(&y_ft) || y_ft == x_ft {
        return Err(format!(
            "subset condition does not resolve to a distinct declared fact type: {}",
            take_chars(&cond_ft_txt, 60)
        ));
    }
    let filter_pos: Option<i64> = filter_lit.as_ref().map(|_| y_roles.len() as i64);
    let mut bound: Vec<String> = Vec::new();
    for mut name in anaphors(cond_txt) {
        while !name.is_empty() && !k.names.contains(&name) {
            name = match name.rsplit_once(' ') {
                Some((pre, _)) => pre.to_string(),
                None => String::new(),
            };
        }
        if !name.is_empty() && !bound.contains(&name) {
            bound.push(name);
        }
    }
    if bound.is_empty() {
        return Err(format!(
            "no anaphoric role binding in the subset condition: {}",
            take_chars(cond_txt, 60)
        ));
    }
    let mut proj_y: Vec<i64> = Vec::new();
    let mut proj_x: Vec<i64> = Vec::new();
    for n in &bound {
        if y_roles.iter().filter(|r| *r == n).count() != 1
            || x_roles.iter().filter(|r| *r == n).count() != 1
        {
            return Err(format!(
                "ambiguous role binding for {} (role-path work pending)",
                n
            ));
        }
        let yp = (y_roles.iter().position(|r| r == n).unwrap() + 1) as i64;
        if Some(yp) == filter_pos {
            return Err(format!(
                "anaphor binds the value role: {}",
                take_chars(cond_txt, 60)
            ));
        }
        proj_y.push(yp);
        proj_x.push((x_roles.iter().position(|r| r == n).unwrap() + 1) as i64);
    }
    let forbidden = sign == "negative";
    let crows = cook_cs(
        "subset",
        "",
        &[y_ft, x_ft.clone()],
        &[cond_txt.trim().to_string(), head_txt.trim().to_string()],
        srv,
    )?;
    let mut builder = if forbidden {
        "constraints:scoped_exclusion_projected".to_string()
    } else {
        "constraints:scoped_subset_projected".to_string()
    };
    let mut op = vec![
        vs(&x_ft),
        vt(proj_y.iter().map(|p| Val::I(*p)).collect()),
        vt(proj_x.iter().map(|p| Val::I(*p)).collect()),
    ];
    if let (Some(fl), Some(fp)) = (&filter_lit, filter_pos) {
        builder.push_str("_filtered");
        op.push(Val::I(fp));
        op.push(vs(fl));
    }
    let redirected = Crows {
        decl: crows.decl.clone(),
        mid: crows.mid.clone(),
        ospecs: crows
            .ospecs
            .iter()
            .map(|(cell, _b, _o)| (cell.clone(), builder.clone(), Val::T(op.clone())))
            .collect(),
    };
    h_crows(&redirected, m, srv)
}

// the deontic fact_type_reading transform (_plan, compiler.py): the inner
// proposition declares its fact type and one constraint row rides with the
// operator, the span, the quoted values, and the deontic tail
fn deontic_fact(
    g: &[Option<String>],
    k: &Known,
    m: &str,
    sign: &str,
    srv: &Srv,
) -> Result<(Asserts, Objs), String> {
    let g0 = g[0].as_deref().unwrap_or("");
    let mut rd = strip_derivation(g0).1;
    if rd.to_lowercase().starts_with("each ") {
        rd = rd[5..].to_string();
    }
    if rd.contains(" and that ") {
        // #34: a compound deontic ('It is {obligatory|forbidden} that X and that
        // Y and that Z') is a multi-fact-type JOIN constraint, NOT one fact type;
        // the catch-all would dequote the whole clause into a PHANTOM fact type
        // (silent deontic loss — Sherlock's core). Refuse LOUDLY, mirroring the
        // compiler.py _plan guard, until the join-exclusion translator lands.
        return Err(format!(
            "compound deontic (X and that Y ...) awaits the join-exclusion translator (#34): {}",
            take_chars(&rd, 70)
        ));
    }
    let ids = quoted_findall(&rd);
    let dequoted = if !ids.is_empty() {
        ws_norm(&quoted_sub(&rd))
    } else {
        rd.clone()
    };
    let (mut facts, mut objs) = h_crows(&cook_fact(&dequoted, k), m, srv)?;
    let (ft, _decl) = fact_type(&dequoted, k);
    let (op, prefix) = if sign == "positive" {
        ("deontic_obligatory", "It is obligatory that ")
    } else {
        ("deontic_forbidden", "It is forbidden that ")
    };
    let row_name = format!("{}{}", prefix, g0);
    let mut row = vec![vs(&row_name), vs(op), vs(&ft)];
    if !ids.is_empty() {
        row.push(vt(ids.iter().map(|i| vs(i)).collect()));
    }
    row.push(vs("deontic"));
    if sign != "positive" {
        let obj = if ids.is_empty() {
            atom(Leaf::S("id".to_string()))
        } else {
            canon_apply(
                srv,
                "constraints:deontic_forbidden",
                val_to_v(&vt(ids.iter().map(|i| vs(i)).collect())),
            )?
        };
        objs.push((format!("{}_df", row_name), obj));
    } else if !ids.is_empty() {
        objs.push((
            format!("{}_do", row_name),
            canon_apply(
                srv,
                "constraints:deontic_obligatory_value",
                val_to_v(&vt(ids.iter().map(|i| vs(i)).collect())),
            )?,
        ));
    } else {
        let subj = subject(&dequoted, k).0;
        let (mfacts, mobjs) = mandatory_parts(&ft, &subj, m, srv)?;
        facts.push(("constraint".to_string(), Val::T(row)));
        facts.extend(mfacts);
        objs.extend(mobjs);
        return Ok((facts, objs));
    }
    facts.push(("constraint".to_string(), Val::T(row)));
    Ok((facts, objs))
}

// ============================ the productions ================================
// The _CLASSIFY pattern table (compiler.py) WITH group extraction, hand-rolled
// with the exact lazy/greedy backtracking order. _productions(): a kind may
// carry several patterns, tried in table order.

fn strip_period(s: &str) -> Option<&str> {
    s.strip_suffix('.')
}

// entity/value type: ^(.+?)(?:\(\.(.+)\))? SUFFIX$
fn p_entity_like(s: &str, suffix: &str) -> Option<Vec<Option<String>>> {
    let head = s.strip_suffix(suffix)?;
    if head.is_empty() {
        return None;
    }
    if head.ends_with(')') {
        for (p, _) in head.match_indices("(.") {
            if p >= 1 && p + 2 < head.len() - 1 {
                return Some(vec![
                    Some(head[..p].to_string()),
                    Some(head[p + 2..head.len() - 1].to_string()),
                ]);
            }
        }
    }
    Some(vec![Some(head.to_string()), None])
}

// the sm family: ^PFX'(.+)'MID'(.+)'\.$ — greedy g1 = the LAST MID
fn p_sm(s: &str, pfx: &str, mid: &str) -> Option<Vec<Option<String>>> {
    let body = s.strip_prefix(pfx)?.strip_suffix("'.")?;
    let (a, b) = split_last(body, mid)?;
    Some(vec![Some(a.to_string()), Some(b.to_string())])
}

// lazy split over several separators: candidates in position order; for each,
// the caller validates the tail; the FIRST fully-matching candidate wins
fn lazy_splits<'a>(s: &'a str, seps: &[&'a str]) -> Vec<(usize, &'a str)> {
    let mut cands: Vec<(usize, &str)> = Vec::new();
    for sep in seps {
        for (p, _) in s.match_indices(sep) {
            if p >= 1 {
                cands.push((p, sep));
            }
        }
    }
    cands.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
    cands
}

fn p_value_constraint(s: &str) -> Option<Vec<Option<String>>> {
    for pfx in [
        "The possible values of ",
        "the possible values of ",
        "The possible value of ",
        "the possible value of ",
    ] {
        if let Some(body) = s.strip_prefix(pfx).and_then(strip_period) {
            for (p, sep) in lazy_splits(body, &[" are ", " is "]) {
                let rest = &body[p + sep.len()..];
                if !rest.is_empty() {
                    return Some(vec![
                        Some(body[..p].to_string()),
                        Some(rest.to_string()),
                    ]);
                }
            }
        }
    }
    None
}

fn p_spanning_uc(s: &str) -> Option<Vec<Option<String>>> {
    for pfx in ["In each population of ", "in each population of "] {
        if let Some(body) = s
            .strip_prefix(pfx)
            .and_then(|b| b.strip_suffix(" combination occurs at most once."))
        {
            if let Some((a, b)) = split_last(body, ", each ") {
                return Some(vec![Some(a.to_string()), Some(b.to_string())]);
            }
        }
    }
    None
}

fn p_spanning_uc2(s: &str) -> Option<Vec<Option<String>>> {
    for pfx in ["Each ", "each "] {
        if let Some(body) = s.strip_prefix(pfx).and_then(strip_period) {
            for (p, sep) in
                lazy_splits(body, &[" combination occurs at most once in the population of "])
            {
                let rest = &body[p + sep.len()..];
                if !rest.is_empty() {
                    return Some(vec![
                        Some(body[..p].to_string()),
                        Some(rest.to_string()),
                    ]);
                }
            }
        }
    }
    None
}

fn p_for_each_mandatory(s: &str) -> Option<Vec<Option<String>>> {
    let body = s.strip_prefix("For each ").and_then(strip_period)?;
    for (p, sep) in lazy_splits(body, &[", some "]) {
        let rest = &body[p + sep.len()..];
        if !rest.is_empty() {
            return Some(vec![Some(body[..p].to_string()), Some(rest.to_string())]);
        }
    }
    None
}

fn p_frequency(s: &str) -> Option<Vec<Option<String>>> {
    for pfx in ["In each population of ", "in each population of "] {
        let body = match s.strip_prefix(pfx).and_then(strip_period) {
            Some(t) => t,
            None => continue,
        };
        let body = match body
            .strip_suffix("times")
            .or_else(|| body.strip_suffix("time"))
        {
            Some(t) => t,
            None => continue,
        };
        let body = match body.strip_suffix(' ') {
            Some(t) => t,
            None => continue,
        };
        let trimmed = body.trim_end_matches(|c: char| c.is_ascii_digit());
        if trimmed.len() == body.len() {
            continue;
        }
        let digits = &body[trimmed.len()..];
        let body = match trimmed.strip_suffix(' ') {
            Some(t) => t,
            None => continue,
        };
        let (body, quant) = match ["at most", "at least", "exactly"]
            .iter()
            .find_map(|kw| body.strip_suffix(kw).map(|t| (t, *kw)))
        {
            Some(x) => x,
            None => continue,
        };
        let body = match body.strip_suffix(" combination occurs ") {
            Some(t) => t,
            None => continue,
        };
        if let Some((a, b)) = split_last(body, ", each ") {
            return Some(vec![
                Some(a.to_string()),
                Some(b.to_string()),
                Some(quant.to_string()),
                Some(digits.to_string()),
            ]);
        }
    }
    None
}

fn p_ring(s: &str) -> Option<Vec<Option<String>>> {
    for w in [
        "acyclic",
        "asymmetric",
        "antisymmetric",
        "intransitive",
        "irreflexive",
        "symmetric",
    ] {
        if let Some(head) = s.strip_suffix(&format!(" is {}.", w)) {
            if !head.is_empty() {
                return Some(vec![Some(head.to_string()), Some(w.to_string())]);
            }
        }
    }
    None
}

fn p_subtype_of(s: &str) -> Option<Vec<Option<String>>> {
    let body = strip_period(s)?;
    let (a, b) = split_last(body, " is a subtype of ")?;
    Some(vec![Some(a.to_string()), Some(b.to_string())])
}

fn p_brace_subtypes(s: &str) -> Option<Vec<Option<String>>> {
    let t = s.strip_prefix('{')?.strip_suffix('.')?;
    let hits: Vec<usize> = t.match_indices("} are ").map(|(p, _)| p).collect();
    for p in hits.into_iter().rev() {
        let g0 = &t[..p];
        if g0.is_empty() {
            continue;
        }
        let mut rest = &t[p + 6..];
        let mut g1: Option<String> = None;
        if let Some(r2) = rest.strip_prefix("mutually exclusive ") {
            g1 = Some("mutually exclusive ".to_string());
            rest = r2;
        }
        if let Some(g2) = rest.strip_prefix("subtypes of ") {
            if !g2.is_empty() {
                return Some(vec![Some(g0.to_string()), g1, Some(g2.to_string())]);
            }
        }
    }
    None
}

fn p_objectification(s: &str) -> Option<Vec<Option<String>>> {
    for pfx in ["This association with ", "this association with "] {
        if let Some(body) = s.strip_prefix(pfx).and_then(strip_period) {
            if let Some((a, b)) =
                split_last(body, " provides the preferred identification scheme for ")
            {
                return Some(vec![Some(a.to_string()), Some(b.to_string())]);
            }
        }
    }
    None
}

fn p_set_comparison(s: &str) -> Option<Vec<Option<String>>> {
    for pfx in ["For each ", "for each "] {
        if let Some(body) = s.strip_prefix(pfx).and_then(strip_period) {
            for (p, sep) in lazy_splits(
                body,
                &[
                    ", exactly one of the following holds: ",
                    ", at most one of the following holds: ",
                ],
            ) {
                let rest = &body[p + sep.len()..];
                if !rest.is_empty() {
                    let mode = if sep.starts_with(", exactly") {
                        "exactly"
                    } else {
                        "at most"
                    };
                    return Some(vec![
                        Some(body[..p].to_string()),
                        Some(mode.to_string()),
                        Some(rest.to_string()),
                    ]);
                }
            }
        }
    }
    None
}

// ^[Ff]or each (.+?), it is impossible that that .+? (.+)MID(.+)\.$
fn p_impossible_that_that(s: &str, mid: &str) -> Option<Vec<Option<String>>> {
    for pfx in ["For each ", "for each "] {
        let body = match s.strip_prefix(pfx).and_then(strip_period) {
            Some(b) => b,
            None => continue,
        };
        for (p, _) in body.match_indices(", it is impossible that that ") {
            if p == 0 {
                continue;
            }
            let g1 = &body[..p];
            let r = &body[p + 29..];
            // lazy .+? then ' ': iterate the spaces of r with ≥1 char before
            for (sp, _) in r.match_indices(' ') {
                if sp < 1 {
                    continue;
                }
                let r2 = &r[sp + 1..];
                // greedy (.+)MID(.+)$: the LAST MID with nonempty sides
                if let Some((g2, g3)) = split_last(r2, mid) {
                    return Some(vec![
                        Some(g1.to_string()),
                        Some(g2.to_string()),
                        Some(g3.to_string()),
                    ]);
                }
            }
        }
    }
    None
}

fn p_disjunctive_for_each(s: &str) -> Option<Vec<Option<String>>> {
    for pfx in ["For each ", "for each "] {
        if let Some(body) = s.strip_prefix(pfx).and_then(strip_period) {
            for (p, _) in body.match_indices(", ") {
                if p >= 1 {
                    let rest = &body[p + 2..];
                    if split_first(rest, " or ").is_some() {
                        return Some(vec![
                            Some(body[..p].to_string()),
                            Some(rest.to_string()),
                        ]);
                    }
                }
            }
        }
    }
    None
}

fn p_inverse_uc(s: &str) -> Option<Vec<Option<String>>> {
    for pfx in ["For each ", "for each "] {
        if let Some(body) = s.strip_prefix(pfx).and_then(strip_period) {
            for (p, sep) in lazy_splits(body, &[", at most one ", ", exactly one "]) {
                let quant = if sep.starts_with(", at most") {
                    "at most one"
                } else {
                    "exactly one"
                };
                let rest = &body[p + sep.len()..];
                // greedy (.+) (?:that|those) .+$ — the LAST separator
                let mut sp: Vec<(usize, usize)> = Vec::new();
                for (q, _) in rest.match_indices(" that ") {
                    sp.push((q, 6));
                }
                for (q, _) in rest.match_indices(" those ") {
                    sp.push((q, 7));
                }
                sp.sort();
                for (q, l) in sp.into_iter().rev() {
                    if q >= 1 && !rest[q + l..].is_empty() {
                        return Some(vec![
                            Some(body[..p].to_string()),
                            Some(quant.to_string()),
                            Some(rest[..q].to_string()),
                        ]);
                    }
                }
            }
        }
    }
    None
}

fn p_subset(s: &str) -> Option<Vec<Option<String>>> {
    for pfx in ["If ", "if "] {
        if let Some(body) = s.strip_prefix(pfx).and_then(strip_period) {
            if let Some((a, b)) = split_last(body, " then ") {
                return Some(vec![Some(a.to_string()), Some(b.to_string())]);
            }
        }
    }
    None
}

// class_rule: ^(?![*+])(\S[^']*?) has (\S[^']*?) '(.+?)' iff (.+)\.$
fn p_class_rule(s: &str) -> Option<Vec<Option<String>>> {
    if s.starts_with('*') || s.starts_with('+') {
        return None;
    }
    let body = strip_period(s)?;
    for (p, _) in body.match_indices(" has ") {
        let g0 = &body[..p];
        if g0.is_empty()
            || g0.chars().next().map_or(true, |c| c.is_whitespace())
            || g0.chars().skip(1).any(|c| c == '\'')
        {
            continue;
        }
        let right = &body[p + 5..];
        for (q, _) in right.match_indices(" '") {
            let g1 = &right[..q];
            if g1.is_empty()
                || g1.chars().next().map_or(true, |c| c.is_whitespace())
                || g1.chars().skip(1).any(|c| c == '\'')
            {
                continue;
            }
            let rest = &right[q + 2..];
            for (r, _) in rest.match_indices("' iff ") {
                if r >= 1 && !rest[r + 6..].is_empty() {
                    return Some(vec![
                        Some(g0.to_string()),
                        Some(g1.to_string()),
                        Some(rest[..r].to_string()),
                        Some(rest[r + 6..].to_string()),
                    ]);
                }
            }
        }
    }
    None
}

fn p_equality(s: &str) -> Option<Vec<Option<String>>> {
    let body = strip_period(s)?;
    let (a, b) = split_last(body, " if and only if ")?;
    Some(vec![Some(a.to_string()), Some(b.to_string())])
}

// rule_if: quote-aware lazy head carrying a digit outside literals, then the
// earliest outside-literals " iff "/" if " (" iff " preferred at a tie)
fn p_rule_if(s: &str) -> Option<Vec<Option<String>>> {
    if s.starts_with('*') || s.starts_with('+') || !s.ends_with('.') {
        return None;
    }
    let qs = quote_positions(s);
    let mut d: Option<usize> = None;
    for (i, b) in s.bytes().enumerate() {
        if b.is_ascii_digit() && even_before(&qs, i) {
            d = Some(i);
            break;
        }
    }
    let d = d?;
    let mut e = s.len();
    for (i, c) in s.char_indices() {
        if i > d && c.is_whitespace() {
            e = i;
            break;
        }
    }
    let mut cands: Vec<(usize, usize)> = Vec::new();
    for (p, _) in s.match_indices(" iff ") {
        if p >= e && even_before(&qs, p) {
            cands.push((p, 5));
        }
    }
    for (p, _) in s.match_indices(" if ") {
        if p >= e && even_before(&qs, p) {
            cands.push((p, 4));
        }
    }
    // position asc; at the same position " iff " (len 5) first — the regex's
    // greedy f? tries "iff" before "if"
    cands.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)));
    for (p, kw) in cands {
        let tail = &s[p + kw..];
        if tail.len() >= 2 && tail.ends_with('.') {
            return Some(vec![
                Some(s[..p].to_string()),
                Some(tail[..tail.len() - 1].to_string()),
            ]);
        }
    }
    None
}

// rule_iff: optional [*+]{1,2} marker, quote-aware head, earliest " iff "
fn p_rule_iff(s: &str) -> Option<Vec<Option<String>>> {
    if !s.ends_with('.') {
        return None;
    }
    let b = s.as_bytes();
    let marker_at = |n: usize| -> bool {
        s.len() > n && b[..n].iter().all(|c| *c == b'*' || *c == b'+') && b[n] == b' '
    };
    let mut starts: Vec<usize> = Vec::new();
    if s.len() > 2 && marker_at(2) {
        starts.push(3);
    }
    if s.len() > 1 && marker_at(1) {
        starts.push(2);
    }
    starts.push(0);
    for off in starts {
        let body = &s[off..];
        let qs = quote_positions(body);
        for (p, _) in body.match_indices(" iff ") {
            if !even_before(&qs, p) {
                continue;
            }
            let tail = &body[p + 5..];
            if tail.len() >= 2 && tail.ends_with('.') {
                let marker = if off > 0 {
                    Some(s[..off - 1].to_string())
                } else {
                    None
                };
                return Some(vec![
                    marker,
                    Some(body[..p].to_string()),
                    Some(tail[..tail.len() - 1].to_string()),
                ]);
            }
        }
    }
    None
}

fn p_derivation_rule(s: &str) -> Option<Vec<Option<String>>> {
    let body = s.strip_prefix("*Each ").and_then(strip_period)?;
    for (p, _) in body.match_indices(" is some ") {
        if p < 1 {
            continue;
        }
        let rest = &body[p + 9..];
        for (q, _) in rest.match_indices(" who ") {
            if q >= 1 && !rest[q + 5..].is_empty() {
                return Some(vec![
                    Some(body[..p].to_string()),
                    Some(rest[..q].to_string()),
                    Some(rest[q + 5..].to_string()),
                ]);
            }
        }
    }
    None
}

fn p_any_form(s: &str, mid: &str) -> Option<Vec<Option<String>>> {
    let body = s.strip_prefix("any ").and_then(strip_period)?;
    for (p, _) in body.match_indices(mid) {
        if p >= 1 && !body[p + mid.len()..].is_empty() {
            return Some(vec![
                Some(body[..p].to_string()),
                Some(body[p + mid.len()..].to_string()),
            ]);
        }
    }
    None
}

fn p_disjunctive_each(s: &str) -> Option<Vec<Option<String>>> {
    for pfx in ["Each ", "each "] {
        if let Some(body) = s.strip_prefix(pfx).and_then(strip_period) {
            if split_first(body, " or ").is_some() {
                return Some(vec![Some(body.to_string())]);
            }
        }
    }
    None
}

fn p_uniqueness(s: &str) -> Option<Vec<Option<String>>> {
    for pfx in ["Each ", "each "] {
        if let Some(body) = s.strip_prefix(pfx).and_then(strip_period) {
            for (p, sep) in lazy_splits(body, &[" at most one ", " exactly one "]) {
                let rest = &body[p + sep.len()..];
                if !rest.is_empty() {
                    let quant = if sep.starts_with(" at most") {
                        "at most one"
                    } else {
                        "exactly one"
                    };
                    return Some(vec![
                        Some(body[..p].to_string()),
                        Some(quant.to_string()),
                        Some(rest.to_string()),
                    ]);
                }
            }
        }
    }
    None
}

fn p_mandatory(s: &str) -> Option<Vec<Option<String>>> {
    for pfx in ["Each ", "each "] {
        if let Some(body) = s.strip_prefix(pfx).and_then(strip_period) {
            for (p, sep) in lazy_splits(body, &[" some "]) {
                let rest = &body[p + sep.len()..];
                if !rest.is_empty() {
                    return Some(vec![
                        Some(body[..p].to_string()),
                        Some(rest.to_string()),
                    ]);
                }
            }
        }
    }
    None
}

fn p_finality(s: &str) -> Option<Vec<Option<String>>> {
    let body = strip_period(s)?;
    let sp = body.find(' ')?;
    let tok0 = &body[..sp];
    if tok0.is_empty() || tok0.contains(char::is_whitespace) {
        return None;
    }
    let dg = body[sp + 1..].strip_prefix("becomes final at depth ")?;
    if dg.is_empty() || !dg.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(vec![Some(tok0.to_string()), Some(dg.to_string())])
}

fn p_neg_pair(s: &str) -> Option<Vec<Option<String>>> {
    let body = strip_period(s)?;
    let sp = body.find(' ')?;
    let tok0 = &body[..sp];
    if tok0.is_empty() {
        return None;
    }
    let rest = &body[sp + 1..];
    for neg in ["does not", "is not"] {
        if let Some(t) = rest.strip_prefix(neg) {
            if let Some(t2) = t.strip_prefix(' ') {
                if t2.chars().next().map_or(false, |c| !c.is_whitespace()) {
                    return Some(vec![
                        Some(tok0.to_string()),
                        Some(neg.to_string()),
                        Some(t2.to_string()),
                    ]);
                }
            }
        }
    }
    None
}

fn p_negation(s: &str) -> Option<Vec<Option<String>>> {
    let body = strip_period(s)?;
    let (a, b) = split_last(body, " ~")?;
    Some(vec![Some(a.to_string()), Some(b.to_string())])
}

fn p_subset_trailing(s: &str) -> Option<Vec<Option<String>>> {
    if !s.ends_with('.') {
        return None;
    }
    // #34: capture an optional leading storage marker (* ** + ++) instead of
    // refusing it, mirroring p_rule_iff and the Python subset_trailing recognizer.
    // A marked head is a value-headed stored derivation; the handler dispatches
    // on the marker. Refusing the marker sent '+ E has W "x" if Y.' to the
    // fact_type_reading catch-all (a silent phantom instance fact).
    let b = s.as_bytes();
    let marker_at = |n: usize| -> bool {
        s.len() > n && b[..n].iter().all(|c| *c == b'*' || *c == b'+') && b[n] == b' '
    };
    let mut starts: Vec<usize> = Vec::new();
    if s.len() > 2 && marker_at(2) {
        starts.push(3);
    }
    if s.len() > 1 && marker_at(1) {
        starts.push(2);
    }
    starts.push(0);
    for off in starts {
        let body = &s[off..];
        let qs = quote_positions(body);
        for (p, _) in body.match_indices(" if ") {
            if p >= 1 && even_before(&qs, p) {
                let tail = &body[p + 4..body.len() - 1];
                if !tail.is_empty() && quote_positions(tail).len() % 2 == 0 {
                    let marker = if off > 0 {
                        Some(s[..off - 1].to_string())
                    } else {
                        None
                    };
                    return Some(vec![
                        marker,
                        Some(body[..p].to_string()),
                        Some(tail.to_string()),
                    ]);
                }
            }
        }
    }
    None
}

fn p_fact_type_reading(s: &str) -> Option<Vec<Option<String>>> {
    let body = strip_period(s)?;
    if body.is_empty() {
        return None;
    }
    Some(vec![Some(body.to_string())])
}

// _productions()[kind] tried in _CLASSIFY order — a kind may carry several
// patterns (the negative/disjunctive twins)
pub fn production_groups(kind: &str, s: &str) -> Option<Vec<Option<String>>> {
    match kind {
        "entity_type" => p_entity_like(s, " is an entity type."),
        "value_type" => p_entity_like(s, " is a value type."),
        "ref_scheme" => {
            let body = s.strip_prefix("Reference Scheme: ").and_then(strip_period)?;
            let (a, b) = split_last(body, " has ")?;
            Some(vec![Some(a.to_string()), Some(b.to_string())])
        }
        "ref_mode" => {
            let body = s.strip_prefix("Reference Mode: ").and_then(strip_period)?;
            if body.is_empty() {
                return None;
            }
            Some(vec![Some(body.to_string())])
        }
        "data_type" => {
            let body = s.strip_prefix("Data Type: ").and_then(strip_period)?;
            if body.is_empty() {
                return None;
            }
            Some(vec![Some(body.to_string())])
        }
        "sm_def" => p_sm(s, "State Machine Definition '", "' is for Noun '"),
        "sm_initial" => p_sm(s, "Status '", "' is initial in State Machine Definition '"),
        "sm_from" => p_sm(s, "Transition '", "' is from Status '"),
        "sm_to" => p_sm(s, "Transition '", "' is to Status '"),
        "sm_trigger" => p_sm(s, "Transition '", "' is triggered by Fact Type '"),
        "sm_guard" => p_sm(s, "Transition '", "' is guarded by Fact Type '"),
        "sm_emit" => p_sm(s, "Transition '", "' emits '"),
        "sm_moore" => p_sm(s, "Status '", "' emits '"),
        "value_constraint" => p_value_constraint(s),
        "spanning_uc" => p_spanning_uc(s),
        "spanning_uc2" => p_spanning_uc2(s),
        "for_each_mandatory" => p_for_each_mandatory(s),
        "frequency" => p_frequency(s),
        "ring" => p_ring(s),
        "subtype_of" => p_subtype_of(s),
        "brace_subtypes" => p_brace_subtypes(s),
        "objectification" => p_objectification(s),
        "set_comparison" => p_set_comparison(s),
        "neg_uniqueness" => p_impossible_that_that(s, " more than one ")
            .or_else(|| p_any_form(s, " more than one ")),
        "neg_mandatory" => {
            p_impossible_that_that(s, " no ").or_else(|| p_any_form(s, " no "))
        }
        "disjunctive_mandatory" => {
            p_disjunctive_for_each(s).or_else(|| p_disjunctive_each(s))
        }
        "inverse_uc" => p_inverse_uc(s),
        "subset" => p_subset(s),
        "class_rule" => p_class_rule(s),
        "equality" => p_equality(s),
        "rule_if" => p_rule_if(s),
        "rule_iff" => p_rule_iff(s),
        "derivation_rule" => p_derivation_rule(s),
        "uniqueness" => p_uniqueness(s),
        "mandatory" => p_mandatory(s),
        "finality" => p_finality(s),
        "neg_pair" => p_neg_pair(s),
        "negation" => p_negation(s),
        "subset_trailing" => p_subset_trailing(s),
        "fact_type_reading" => p_fact_type_reading(s),
        _ => None,
    }
}

// ============================ _plan ==========================================
// _plan (compiler.py): cook when the kind is in _COOK (except the deontic
// fact_type_reading transform), then dispatch the handler.

const COOKED: [&str; 20] = [
    "sm_trigger",
    "sm_guard",
    "ring",
    "frequency",
    "value_constraint",
    "uniqueness",
    "mandatory",
    "neg_uniqueness",
    "neg_mandatory",
    "for_each_mandatory",
    "inverse_uc",
    "spanning_uc",
    "spanning_uc2",
    "negation",
    "subtype_of",
    "fact_type_reading",
    "derivation_rule",
    "class_rule",
    "neg_pair",
    "rule_if",
];

pub fn plan(
    kind: &str,
    g: &[Option<String>],
    k: &Known,
    m: &str,
    sign: &str,
    srv: &Srv,
) -> Result<(Asserts, Objs), String> {
    let deontic_fact_reading = m == "deontic" && kind == "fact_type_reading";
    if deontic_fact_reading {
        return deontic_fact(g, k, m, sign, srv);
    }
    // the _COOK boundary + the crows handlers
    if COOKED.contains(&kind) || kind == "rule_iff" {
        let crows: Crows = match kind {
            "sm_trigger" => {
                let ft = clause_ft(g[1].as_deref().unwrap_or(""), k);
                return sm_rows(
                    "is triggered by Fact Type",
                    "Transition",
                    g[0].as_deref().unwrap_or(""),
                    &ft,
                    srv,
                );
            }
            "sm_guard" => {
                let ft = clause_ft(g[1].as_deref().unwrap_or(""), k);
                return sm_rows(
                    "is guarded by Fact Type",
                    "Transition",
                    g[0].as_deref().unwrap_or(""),
                    &ft,
                    srv,
                );
            }
            "ring" => {
                let (decl, cid, kind_tag, ft, builder) = cook_ring(g, k);
                let mut rows: Asserts = decl;
                rows.push((
                    "constraint".to_string(),
                    vt(vec![vs(&cid), vs(&kind_tag), vs(&ft), vs(m)]),
                ));
                let obj = canon_apply(
                    srv,
                    &builder,
                    val_to_v(&vt(vec![Val::I(1), Val::I(2)])),
                )?;
                return Ok((rows, vec![(cid, obj)]));
            }
            "frequency" => {
                let (cid, ftn, roles, bop) = cook_frequency(g, k);
                let mut rows: Asserts = vec![(
                    "constraint".to_string(),
                    vt(vec![vs(&cid), vs("frequency"), vs(&ftn), vs(m)]),
                )];
                for p in &roles {
                    rows.push(("spans".to_string(), vt(vec![vs(&cid), Val::I(*p)])));
                }
                let obj = canon_apply(srv, "constraints:frequency", val_to_v(&bop))?;
                return Ok((rows, vec![(cid, obj)]));
            }
            "value_constraint" => {
                let (name, spec, cid, builder, bop) = cook_value_constraint(g);
                let rows: Asserts = vec![
                    (
                        "valueConstraint".to_string(),
                        vt(vec![vs(&name), vs(&spec), vs(m)]),
                    ),
                    (
                        "constraint".to_string(),
                        vt(vec![vs(&cid), vs("value"), vs(&name), vs(m)]),
                    ),
                ];
                let obj = canon_apply(srv, &builder, val_to_v(&bop))?;
                return Ok((rows, vec![(cid, obj)]));
            }
            "uniqueness" => cook_uniqueness(g, k),
            "mandatory" => cook_mandatory(g, k),
            "neg_uniqueness" => cook_neg_uniqueness(g, k),
            "neg_mandatory" => cook_neg_mandatory(g, k),
            "for_each_mandatory" => cook_for_each_mandatory(g, k),
            "inverse_uc" => cook_inverse_uc(g, k),
            "spanning_uc" => cook_spanning(g, k)?,
            "spanning_uc2" => cook_spanning_corpus(g, k)?,
            "negation" => cook_negation(g, k),
            "subtype_of" => cook_subtype(
                g[0].as_deref().unwrap_or(""),
                g[1].as_deref().unwrap_or(""),
            ),
            "fact_type_reading" => cook_fact(g[0].as_deref().unwrap_or(""), k),
            "derivation_rule" => cook_derivation_rule(g, k),
            "class_rule" => cook_class_rule(g),
            "neg_pair" => cook_neg_pair(g, k),
            "rule_if" => cook_rule_if(
                g[0].as_deref().unwrap_or(""),
                g[1].as_deref().unwrap_or(""),
                k,
                "fully-derived",
            ),
            "rule_iff" => cook_rule_if(
                g[1].as_deref().unwrap_or(""),
                g[2].as_deref().unwrap_or(""),
                k,
                marker_kind(g[0].as_deref().unwrap_or("*")),
            ),
            _ => unreachable!(),
        };
        return h_crows(&crows, m, srv);
    }
    match kind {
        "entity_type" | "value_type" => {
            let name = g[0].as_deref().unwrap_or("");
            let ot = if kind == "entity_type" {
                "ObjectType"
            } else {
                "ValueType"
            };
            let mut rows: Asserts = vec![(
                "instanceOf".to_string(),
                vt(vec![vs(name), vs(ot)]),
            )];
            if let Some(rm) = g.get(1).and_then(|x| x.as_deref()) {
                rows.push(("refMode".to_string(), vt(vec![vs(name), vs(rm)])));
            }
            Ok((rows, Vec::new()))
        }
        "ref_scheme" => {
            let g0 = g[0].as_deref().unwrap_or("");
            let g1 = g[1].as_deref().unwrap_or("");
            Ok((
                vec![
                    (
                        "instanceOf".to_string(),
                        vt(vec![vs(g0), vs("ObjectType")]),
                    ),
                    (
                        "instanceOf".to_string(),
                        vt(vec![vs(g1), vs("ValueType")]),
                    ),
                    ("refScheme".to_string(), vt(vec![vs(g0), vs(g1)])),
                ],
                Vec::new(),
            ))
        }
        "objectification" => {
            let g0 = g[0].as_deref().unwrap_or("");
            let g1 = g[1].as_deref().unwrap_or("");
            Ok((
                vec![
                    (
                        "instanceOf".to_string(),
                        vt(vec![vs(g1), vs("ObjectType")]),
                    ),
                    (
                        "objectification".to_string(),
                        vt(vec![vs(g1), vs(g0)]),
                    ),
                ],
                Vec::new(),
            ))
        }
        "data_type" | "ref_mode" => {
            let g0 = g[0].as_deref().unwrap_or("");
            Ok((
                vec![(kind.to_string(), vt(vec![vs(g0)]))],
                Vec::new(),
            ))
        }
        "brace_subtypes" => h_brace_subtypes(g, m, srv),
        "set_comparison" => h_set_comparison(g, m, k, srv),
        "disjunctive_mandatory" => h_disjunctive(g, m, k, srv),
        "subset" => Err(
            "subset translation awaits role projection (set-comparison arc)".to_string(),
        ),
        "subset_trailing" => h_subset_trailing(g, k, m, sign, srv),
        "equality" => h_equality(g, m, k, srv),
        "finality" => {
            let g0 = g[0].as_deref().unwrap_or("");
            let n: i64 = g[1].as_deref().unwrap_or("0").parse().unwrap_or(0);
            Ok((
                vec![(
                    "finality".to_string(),
                    vt(vec![vs(g0), Val::I(n)]),
                )],
                Vec::new(),
            ))
        }
        "sm_def" => sm_rows(
            "is for Noun",
            "State Machine Definition",
            g[0].as_deref().unwrap_or(""),
            g[1].as_deref().unwrap_or(""),
            srv,
        ),
        "sm_initial" => sm_rows(
            "is initial in State Machine Definition",
            "Status",
            g[0].as_deref().unwrap_or(""),
            g[1].as_deref().unwrap_or(""),
            srv,
        ),
        "sm_from" => sm_rows(
            "is from Status",
            "Transition",
            g[0].as_deref().unwrap_or(""),
            g[1].as_deref().unwrap_or(""),
            srv,
        ),
        "sm_to" => sm_rows(
            "is to Status",
            "Transition",
            g[0].as_deref().unwrap_or(""),
            g[1].as_deref().unwrap_or(""),
            srv,
        ),
        "sm_emit" => sm_rows(
            "emits",
            "Transition",
            g[0].as_deref().unwrap_or(""),
            g[1].as_deref().unwrap_or(""),
            srv,
        ),
        "sm_moore" => sm_rows(
            "emits",
            "Status",
            g[0].as_deref().unwrap_or(""),
            g[1].as_deref().unwrap_or(""),
            srv,
        ),
        // _PLAN.get(kind, ([], [])) — the python default
        _ => Ok((Vec::new(), Vec::new())),
    }
}

// ============================ the translator entry ===========================
// _stmt_translator_impl (compiler.py): per translator, the FIRST kind whose
// production matches plans the statement; no match is the refusal the
// dispatcher's except swallows (dispatch continues).

pub struct Fire {
    pub kind: &'static str,
    pub asserts: Asserts,
    pub objs: Objs,
}

pub fn translate(
    kinds: &[&'static str],
    inner: &str,
    mfield: &str,
    k: &Known,
    srv: &Srv,
) -> Result<Option<Fire>, String> {
    let (m, sign) = match mfield.split_once(':') {
        Some((a, b)) => (a, b),
        None => (mfield, ""),
    };
    for kind in kinds {
        if let Some(g) = production_groups(kind, inner) {
            let (asserts, objs) = plan(kind, &g, k, m, sign, srv)?;
            return Ok(Some(Fire {
                kind,
                asserts,
                objs,
            }));
        }
    }
    Ok(None)
}

// one fire as a report JSON fragment (the differential's row/obj dump)
pub fn fire_json(t: &str, fire: &Fire, out: &mut String) {
    out.push_str("{\"t\":");
    json_escape_into(t, out);
    out.push_str(",\"kind\":");
    json_escape_into(fire.kind, out);
    out.push_str(",\"asserts\":[");
    for (i, (cell, row)) in fire.asserts.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('[');
        json_escape_into(cell, out);
        out.push(',');
        val_json(row, out);
        out.push(']');
    }
    out.push_str("],\"objs\":[");
    for (i, (name, obj)) in fire.objs.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('[');
        json_escape_into(name, out);
        out.push(',');
        obj_json(obj, out);
        out.push(']');
    }
    out.push_str("]}");
}
