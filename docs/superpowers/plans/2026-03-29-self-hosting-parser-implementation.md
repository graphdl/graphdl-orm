# Self-Hosting FORML 2 Parser Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the TypeScript claims parser and Rust parse_rule.rs with a self-hosting FORML 2 parser — readings evaluated by the engine. One parser. One language. No unsourced primitives.

**Architecture:** Complete Backus's primitive set in ast.rs (add arithmetic, logic, missing sequence ops). Build a Rust FORML 2 parser that reads markdown and produces ConstraintIR directly. Express the grammar rules as readings in syntax.md. Export `load_readings(markdown)` from WASM. Delete the TypeScript parser. Document in graphdl skill.

**Tech Stack:** Rust (fol-engine crate), WASM (wasm-bindgen), FORML 2 (Halpin)

**Sources:** Backus 1978 (primitives/forms), Codd 1970 (relational ops), Halpin 2001/2008 (constraints/syntax)

---

## File Structure

### Create
- `crates/fol-engine/src/primitives.rs` — first-class arithmetic (+,-,*,div) and logic (and,or,not) as Func variants, replacing Native escape hatches
- `crates/fol-engine/src/parse_forml2.rs` — FORML 2 markdown parser (replaces parse_rule.rs)
- `readings/syntax.md` — FORML 2 grammar expressed as readings (self-hosting target)

### Modify
- `crates/fol-engine/src/ast.rs` — add ApndR, RotL, RotR, Add, Sub, Mul, Div, And, Or, Not as Func variants
- `crates/fol-engine/src/lib.rs` — add `load_readings` WASM export, add `pub mod primitives; pub mod parse_forml2;`
- `crates/fol-engine/src/compile.rs` — replace Native arithmetic/logic in constraint compilers with first-class Func variants
- `crates/fol-engine/src/evaluate.rs` — update forward_chain_ast to use first-class primitives
- `~/.claude/skills/graphdl/graphdl.md` — add derivation rules, combining forms, AREST execution model sections

### Delete
- `crates/fol-engine/src/parse_rule.rs` — replaced by parse_forml2.rs
- `src/claims/ingest.ts` — replaced by Rust parser
- `src/claims/tokenize.ts` — replaced by Rust parser
- `src/claims/constraints.ts` — replaced by Rust parser
- `src/claims/steps.ts` — replaced by Rust parser
- `src/claims/scope.ts` — replaced by Rust parser
- `src/claims/batch-builder.ts` — replaced by Rust parser
- All `src/claims/*.test.ts` — tests move to Rust

---

### Task 1: Add Missing Backus Primitives to ast.rs

**Files:**
- Modify: `crates/fol-engine/src/ast.rs`

Three sequence operations from Backus 11.2.3 are missing from the Func enum: ApndR (append right), RotL (rotate left), RotR (rotate right).

- [ ] **Step 1: Write failing tests for ApndR, RotL, RotR**

Add to the `mod tests` block in ast.rs:

```rust
#[test]
fn apndr_appends_to_right() {
    // apndr:<<y1,...,yn>, z> = <y1,...,yn, z>
    let x = Object::seq(vec![
        Object::seq(vec![Object::atom("a"), Object::atom("b")]),
        Object::atom("c"),
    ]);
    assert_eq!(
        apply(&Func::ApndR, &x, &defs()),
        Object::seq(vec![Object::atom("a"), Object::atom("b"), Object::atom("c")])
    );
}

#[test]
fn rotl_rotates_left() {
    // rotl:<a,b,c> = <b,c,a>
    let seq = Object::seq(vec![Object::atom("a"), Object::atom("b"), Object::atom("c")]);
    assert_eq!(
        apply(&Func::RotL, &seq, &defs()),
        Object::seq(vec![Object::atom("b"), Object::atom("c"), Object::atom("a")])
    );
}

#[test]
fn rotr_rotates_right() {
    // rotr:<a,b,c> = <c,a,b>
    let seq = Object::seq(vec![Object::atom("a"), Object::atom("b"), Object::atom("c")]);
    assert_eq!(
        apply(&Func::RotR, &seq, &defs()),
        Object::seq(vec![Object::atom("c"), Object::atom("a"), Object::atom("b")])
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd crates/fol-engine && cargo test apndr_appends -- --nocapture 2>&1 | head -20`
Expected: compilation error — `Func::ApndR` does not exist

- [ ] **Step 3: Add ApndR, RotL, RotR to Func enum and apply()**

In the Func enum (after `Reverse`), add:

```rust
/// Append right: apndr:<<y1,...,yn>, z> = <y1,...,yn, z>
ApndR,

/// Rotate left: rotl:<x1,...,xn> = <x2,...,xn, x1>
RotL,

/// Rotate right: rotr:<x1,...,xn> = <xn, x1,...,xn-1>
RotR,
```

In the `apply()` match (after the `Func::Reverse` arm), add:

```rust
Func::ApndR => {
    match x.as_seq() {
        Some(items) if items.len() == 2 => {
            match items[0].as_seq() {
                Some(ys) => {
                    let mut result = ys.to_vec();
                    result.push(items[1].clone());
                    Object::Seq(result)
                }
                _ => Object::Bottom,
            }
        }
        _ => Object::Bottom,
    }
}

Func::RotL => {
    match x.as_seq() {
        Some(items) if items.len() >= 2 => {
            let mut result = items[1..].to_vec();
            result.push(items[0].clone());
            Object::Seq(result)
        }
        Some(_) => x.clone(), // single element or phi — unchanged
        _ => Object::Bottom,
    }
}

Func::RotR => {
    match x.as_seq() {
        Some(items) if items.len() >= 2 => {
            let mut result = vec![items[items.len() - 1].clone()];
            result.extend_from_slice(&items[..items.len() - 1]);
            Object::Seq(result)
        }
        Some(_) => x.clone(),
        _ => Object::Bottom,
    }
}
```

Add Debug impl arms for the new variants.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd crates/fol-engine && cargo test apndr_appends rotl_rotates rotr_rotates -- --nocapture`
Expected: 3 PASS

- [ ] **Step 5: Commit**

```bash
cd C:/Users/lippe/Repos/graphdl-orm
git add crates/fol-engine/src/ast.rs
git commit -m "feat: add ApndR, RotL, RotR — complete Backus sequence primitives"
```

---

### Task 2: Add First-Class Arithmetic Primitives

**Files:**
- Modify: `crates/fol-engine/src/ast.rs`

Backus defines +, -, ×, ÷ as primitive functions on number-atoms (Section 11.2.3). Currently these are implemented as Native closures in compile.rs. They must be first-class Func variants.

- [ ] **Step 1: Write failing tests for arithmetic**

```rust
#[test]
fn add_numbers() {
    // +:<3,4> = 7
    let x = Object::seq(vec![Object::atom("3"), Object::atom("4")]);
    assert_eq!(apply(&Func::Add, &x, &defs()), Object::atom("7"));
}

#[test]
fn sub_numbers() {
    // -:<7,4> = 3
    let x = Object::seq(vec![Object::atom("7"), Object::atom("4")]);
    assert_eq!(apply(&Func::Sub, &x, &defs()), Object::atom("3"));
}

#[test]
fn mul_numbers() {
    // ×:<3,4> = 12
    let x = Object::seq(vec![Object::atom("3"), Object::atom("4")]);
    assert_eq!(apply(&Func::Mul, &x, &defs()), Object::atom("12"));
}

#[test]
fn div_numbers() {
    // ÷:<12,4> = 3
    let x = Object::seq(vec![Object::atom("12"), Object::atom("4")]);
    assert_eq!(apply(&Func::Div, &x, &defs()), Object::atom("3"));
}

#[test]
fn div_by_zero_is_bottom() {
    let x = Object::seq(vec![Object::atom("12"), Object::atom("0")]);
    assert_eq!(apply(&Func::Div, &x, &defs()), Object::Bottom);
}

#[test]
fn arithmetic_on_non_numbers_is_bottom() {
    let x = Object::seq(vec![Object::atom("hello"), Object::atom("4")]);
    assert_eq!(apply(&Func::Add, &x, &defs()), Object::Bottom);
}

#[test]
fn add_floats() {
    // +:<2.5, 1.5> = 4
    let x = Object::seq(vec![Object::atom("2.5"), Object::atom("1.5")]);
    assert_eq!(apply(&Func::Add, &x, &defs()), Object::atom("4"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd crates/fol-engine && cargo test add_numbers -- --nocapture 2>&1 | head -10`
Expected: compilation error — `Func::Add` does not exist

- [ ] **Step 3: Add arithmetic Func variants and apply() arms**

In the Func enum (after RotR), add:

```rust
// ── Arithmetic (Backus 11.2.3) ──────────────────────────────
/// Add: +:<y,z> = y+z where y,z are number atoms
Add,
/// Subtract: -:<y,z> = y-z
Sub,
/// Multiply: ×:<y,z> = y×z
Mul,
/// Divide: ÷:<y,z> = y÷z, bottom if z=0
Div,
```

Add a helper function:

```rust
/// Parse a pair of number atoms, apply an arithmetic operation.
fn apply_arithmetic(x: &Object, op: fn(f64, f64) -> Option<f64>) -> Object {
    match x.as_seq() {
        Some(items) if items.len() == 2 => {
            let a = items[0].as_atom().and_then(|s| s.parse::<f64>().ok());
            let b = items[1].as_atom().and_then(|s| s.parse::<f64>().ok());
            match (a, b) {
                (Some(a), Some(b)) => match op(a, b) {
                    Some(r) => {
                        // Emit integer if result is whole number
                        if r.fract() == 0.0 && r.abs() < i64::MAX as f64 {
                            Object::Atom((r as i64).to_string())
                        } else {
                            Object::Atom(r.to_string())
                        }
                    }
                    None => Object::Bottom,
                },
                _ => Object::Bottom,
            }
        }
        _ => Object::Bottom,
    }
}
```

In the `apply()` match, add:

```rust
Func::Add => apply_arithmetic(x, |a, b| Some(a + b)),
Func::Sub => apply_arithmetic(x, |a, b| Some(a - b)),
Func::Mul => apply_arithmetic(x, |a, b| Some(a * b)),
Func::Div => apply_arithmetic(x, |a, b| if b == 0.0 { None } else { Some(a / b) }),
```

Add Debug impl arms.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd crates/fol-engine && cargo test add_numbers sub_numbers mul_numbers div_numbers div_by_zero arithmetic_on_non add_floats -- --nocapture`
Expected: 7 PASS

- [ ] **Step 5: Commit**

```bash
git add crates/fol-engine/src/ast.rs
git commit -m "feat: add first-class arithmetic primitives (Backus +,-,×,÷)"
```

---

### Task 3: Add First-Class Logic Primitives

**Files:**
- Modify: `crates/fol-engine/src/ast.rs`

Backus defines and, or, not as primitive functions (Section 11.2.3).

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn and_logic() {
    assert_eq!(apply(&Func::And, &Object::seq(vec![Object::t(), Object::t()]), &defs()), Object::t());
    assert_eq!(apply(&Func::And, &Object::seq(vec![Object::t(), Object::f()]), &defs()), Object::f());
    assert_eq!(apply(&Func::And, &Object::seq(vec![Object::f(), Object::f()]), &defs()), Object::f());
}

#[test]
fn or_logic() {
    assert_eq!(apply(&Func::Or, &Object::seq(vec![Object::f(), Object::f()]), &defs()), Object::f());
    assert_eq!(apply(&Func::Or, &Object::seq(vec![Object::t(), Object::f()]), &defs()), Object::t());
    assert_eq!(apply(&Func::Or, &Object::seq(vec![Object::f(), Object::t()]), &defs()), Object::t());
}

#[test]
fn not_logic() {
    assert_eq!(apply(&Func::Not, &Object::t(), &defs()), Object::f());
    assert_eq!(apply(&Func::Not, &Object::f(), &defs()), Object::t());
    assert_eq!(apply(&Func::Not, &Object::atom("x"), &defs()), Object::Bottom);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd crates/fol-engine && cargo test and_logic -- --nocapture 2>&1 | head -10`
Expected: compilation error

- [ ] **Step 3: Add And, Or, Not to Func enum and apply()**

```rust
// ── Logic (Backus 11.2.3) ───────────────────────────────────
/// And: and:<T,T> = T, and:<T,F> = F, etc.
And,
/// Or: or:<F,F> = F, or:<T,F> = T, etc.
Or,
/// Not: not:T = F, not:F = T
Not,
```

In apply():

```rust
Func::And => {
    match x.as_seq() {
        Some(items) if items.len() == 2 => {
            match (items[0].as_atom(), items[1].as_atom()) {
                (Some("T"), Some("T")) => Object::t(),
                (Some("T"), Some("F")) | (Some("F"), Some("T")) | (Some("F"), Some("F")) => Object::f(),
                _ => Object::Bottom,
            }
        }
        _ => Object::Bottom,
    }
}

Func::Or => {
    match x.as_seq() {
        Some(items) if items.len() == 2 => {
            match (items[0].as_atom(), items[1].as_atom()) {
                (Some("F"), Some("F")) => Object::f(),
                (Some("T"), _) | (_, Some("T")) => Object::t(),
                (Some("F"), Some("F")) => Object::f(),
                _ => Object::Bottom,
            }
        }
        _ => Object::Bottom,
    }
}

Func::Not => {
    match x.as_atom() {
        Some("T") => Object::f(),
        Some("F") => Object::t(),
        _ => Object::Bottom,
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd crates/fol-engine && cargo test and_logic or_logic not_logic -- --nocapture`
Expected: 3 PASS

- [ ] **Step 5: Commit**

```bash
git add crates/fol-engine/src/ast.rs
git commit -m "feat: add first-class logic primitives (Backus and, or, not)"
```

---

### Task 4: Verify Backus's Inner Product Example

**Files:**
- Modify: `crates/fol-engine/src/ast.rs` (tests only)

Backus Section 5.2/11.3.2 defines inner product: `Def IP == (/+) . (alpha ×) . trans`. This is the canonical test that arithmetic, apply-to-all, insert, composition, and transpose all work together.

- [ ] **Step 1: Write the inner product test**

```rust
#[test]
fn inner_product_backus_example() {
    // Def IP ≡ (/+) ∘ (α×) ∘ trans
    // IP:<<1,2,3>,<6,5,4>> = 28
    //
    // trans:<<1,2,3>,<6,5,4>> = <<1,6>,<2,5>,<3,4>>
    // α×:<<1,6>,<2,5>,<3,4>> = <6, 10, 12>
    // /+:<6, 10, 12> = 28
    let ip = Func::compose(
        Func::insert(Func::Add),
        Func::compose(
            Func::apply_to_all(Func::Mul),
            Func::Trans,
        ),
    );
    let input = Object::seq(vec![
        Object::seq(vec![Object::atom("1"), Object::atom("2"), Object::atom("3")]),
        Object::seq(vec![Object::atom("6"), Object::atom("5"), Object::atom("4")]),
    ]);
    assert_eq!(apply(&ip, &input, &defs()), Object::atom("28"));
}

#[test]
fn insert_add_folds_sum() {
    // /+:<1,2,3> = 6
    let f = Func::insert(Func::Add);
    let seq = Object::seq(vec![Object::atom("1"), Object::atom("2"), Object::atom("3")]);
    assert_eq!(apply(&f, &seq, &defs()), Object::atom("6"));
}

#[test]
fn insert_add_singleton() {
    // /+:<7> = 7
    let f = Func::insert(Func::Add);
    let seq = Object::seq(vec![Object::atom("7")]);
    assert_eq!(apply(&f, &seq, &defs()), Object::atom("7"));
}
```

- [ ] **Step 2: Run test**

Run: `cd crates/fol-engine && cargo test inner_product_backus insert_add -- --nocapture`
Expected: 3 PASS (primitives from Tasks 2-3 must be in place)

- [ ] **Step 3: Commit**

```bash
git add crates/fol-engine/src/ast.rs
git commit -m "test: verify Backus inner product IP = (/+).(α×).trans"
```

---

### Task 5: Replace Native Escape Hatches in Constraint Compilers

**Files:**
- Modify: `crates/fol-engine/src/compile.rs`

The constraint compilers currently use `Func::Native(Arc::new(...))` for arithmetic, counting, and logic. Replace with first-class Func variants. This removes the Native escape hatch — all constraint compilation uses sourced primitives.

- [ ] **Step 1: Find all Native uses in compile.rs**

Run: `cd crates/fol-engine && grep -n "Func::Native" src/compile.rs | head -30`

Identify each Native closure and what it does (count facts, compare numbers, boolean logic).

- [ ] **Step 2: Replace each Native with composed first-class Funcs**

For each Native closure found:
- Counting facts: replace with `Func::compose(Func::Length, Func::filter(predicate))`
- Number comparison: replace with `Func::compose(Func::Sub, ...)` followed by sign check, or use a comparison constraint predicate per Halpin
- Boolean OR in insert: replace with `Func::insert(Func::Or)`
- Boolean AND: replace with `Func::insert(Func::And)`

The exact replacements depend on what `grep` finds. Each replacement must produce the same Object output as the Native it replaces.

- [ ] **Step 3: Run full test suite**

Run: `cd crates/fol-engine && cargo test`
Expected: All existing tests pass. The constraint evaluations must produce identical results.

- [ ] **Step 4: Commit**

```bash
git add crates/fol-engine/src/compile.rs
git commit -m "refactor: replace Native closures with first-class Backus primitives in constraint compilers"
```

---

### Task 6: FORML 2 Markdown Parser in Rust

**Files:**
- Create: `crates/fol-engine/src/parse_forml2.rs`
- Modify: `crates/fol-engine/src/lib.rs`

Build a Rust parser that reads FORML 2 markdown and produces a `ConstraintIR`. This replaces both `parse_rule.rs` (derivation rules) and `src/claims/*.ts` (everything else). The parser recognizes Halpin's established patterns.

- [ ] **Step 1: Write failing tests for entity type parsing**

Create `crates/fol-engine/src/parse_forml2.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_entity_type() {
        let input = "Customer(.Email) is an entity type.";
        let ir = parse_markdown(input).unwrap();
        assert!(ir.nouns.contains_key("Customer"));
        assert_eq!(ir.nouns["Customer"].object_type, "entity");
        assert_eq!(ir.nouns["Customer"].ref_scheme, Some(vec!["Email".to_string()]));
    }

    #[test]
    fn parse_value_type() {
        let input = "Gender is a value type.\n  The possible values of Gender are 'M', 'F'.";
        let ir = parse_markdown(input).unwrap();
        assert!(ir.nouns.contains_key("Gender"));
        assert_eq!(ir.nouns["Gender"].object_type, "value");
        assert_eq!(ir.nouns["Gender"].enum_values, Some(vec!["M".to_string(), "F".to_string()]));
    }

    #[test]
    fn parse_subtype() {
        let input = "Male is a subtype of Person.";
        let ir = parse_markdown(input).unwrap();
        assert_eq!(ir.nouns["Male"].super_type, Some("Person".to_string()));
    }

    #[test]
    fn parse_binary_fact_type() {
        let input = "# Test\n\n## Entity Types\n\nCustomer(.Email) is an entity type.\nCountry(.Code) is an entity type.\n\n## Fact Types\n\nCustomer was born in Country.";
        let ir = parse_markdown(input).unwrap();
        let ft_key = ir.fact_types.keys().find(|k| k.contains("born")).unwrap();
        let ft = &ir.fact_types[ft_key];
        assert_eq!(ft.roles.len(), 2);
        assert_eq!(ft.roles[0].noun_name, "Customer");
        assert_eq!(ft.roles[1].noun_name, "Country");
    }
}
```

- [ ] **Step 2: Implement minimal parse_markdown**

```rust
use crate::types::*;
use std::collections::HashMap;

pub fn parse_markdown(input: &str) -> Result<ConstraintIR, String> {
    let mut ir = ConstraintIR {
        domain: String::new(),
        nouns: HashMap::new(),
        fact_types: HashMap::new(),
        constraints: vec![],
        state_machines: HashMap::new(),
        derivation_rules: vec![],
    };

    let lines: Vec<&str> = input.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim();

        // Domain name from H1
        if line.starts_with("# ") && ir.domain.is_empty() {
            ir.domain = line[2..].trim().to_string();
        }

        // Entity type: "X(.Ref) is an entity type."
        if let Some(caps) = parse_entity_type_line(line) {
            ir.nouns.insert(caps.0.clone(), NounDef {
                object_type: "entity".to_string(),
                enum_values: None,
                value_type: None,
                super_type: None,
                world_assumption: WorldAssumption::default(),
                ref_scheme: caps.1,
            });
        }

        // Value type: "X is a value type."
        if line.ends_with("is a value type.") {
            let name = line.trim_end_matches(" is a value type.").trim().to_string();
            let mut enum_values = None;
            // Check next line for "The possible values of X are ..."
            if i + 1 < lines.len() {
                let next = lines[i + 1].trim();
                if next.starts_with("The possible values of") {
                    enum_values = parse_enum_values(next);
                    i += 1;
                }
            }
            ir.nouns.insert(name, NounDef {
                object_type: "value".to_string(),
                enum_values,
                value_type: None,
                super_type: None,
                world_assumption: WorldAssumption::default(),
                ref_scheme: None,
            });
        }

        // Subtype: "X is a subtype of Y."
        if line.contains("is a subtype of") && line.ends_with('.') {
            let parts: Vec<&str> = line.trim_end_matches('.').split(" is a subtype of ").collect();
            if parts.len() == 2 {
                let sub = parts[0].trim().to_string();
                let sup = parts[1].trim().to_string();
                ir.nouns.entry(sub).or_insert(NounDef {
                    object_type: "entity".to_string(),
                    enum_values: None,
                    value_type: None,
                    super_type: Some(sup),
                    world_assumption: WorldAssumption::default(),
                    ref_scheme: None,
                }).super_type = Some(parts[1].trim().to_string());
            }
        }

        // Fact types: lines containing two known nouns connected by a verb phrase
        // (parsed after all nouns are collected — second pass)

        i += 1;
    }

    // Second pass: parse fact types, constraints, derivation rules
    parse_fact_types_and_constraints(&mut ir, &lines);

    Ok(ir)
}

fn parse_entity_type_line(line: &str) -> Option<(String, Option<Vec<String>>)> {
    if !line.ends_with("is an entity type.") { return None; }
    let prefix = line.trim_end_matches(" is an entity type.").trim();
    if let Some(paren_start) = prefix.find("(.") {
        let name = prefix[..paren_start].trim().to_string();
        let ref_str = &prefix[paren_start + 2..prefix.len() - 1];
        let refs: Vec<String> = ref_str.split(", ").map(|s| s.trim().to_string()).collect();
        Some((name, Some(refs)))
    } else {
        Some((prefix.to_string(), None))
    }
}

fn parse_enum_values(line: &str) -> Option<Vec<String>> {
    // "The possible values of X are 'A', 'B', 'C'."
    let after_are = line.split(" are ").nth(1)?;
    let trimmed = after_are.trim_end_matches('.');
    let values: Vec<String> = trimmed.split(", ")
        .map(|s| s.trim().trim_matches('\'').to_string())
        .collect();
    Some(values)
}

fn parse_fact_types_and_constraints(ir: &mut ConstraintIR, lines: &[&str]) {
    let noun_names: Vec<&str> = ir.nouns.keys().map(|s| s.as_str()).collect();
    // Implementation continues in subsequent tasks
}
```

- [ ] **Step 3: Add module to lib.rs**

In `crates/fol-engine/src/lib.rs`, add:
```rust
pub mod parse_forml2;
```

- [ ] **Step 4: Run tests**

Run: `cd crates/fol-engine && cargo test parse_forml2 -- --nocapture`
Expected: entity type, value type, subtype tests PASS

- [ ] **Step 5: Commit**

```bash
git add crates/fol-engine/src/parse_forml2.rs crates/fol-engine/src/lib.rs
git commit -m "feat: FORML 2 markdown parser — entity types, value types, subtypes"
```

---

### Task 7: Parse Fact Types and Constraint Patterns

**Files:**
- Modify: `crates/fol-engine/src/parse_forml2.rs`

Extend the parser to recognize binary/n-ary fact types and Halpin's constraint verbalization patterns (UC, MC, FC, combined).

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn parse_uniqueness_constraint() {
    let input = "# T\n\n## Entity Types\n\nPerson(.Name) is an entity type.\nCountry(.Code) is an entity type.\n\n## Fact Types\n\nPerson was born in Country.\n\n## Constraints\n\nEach Person was born in at most one Country.";
    let ir = parse_markdown(input).unwrap();
    assert!(ir.constraints.iter().any(|c| c.kind == "UC"));
}

#[test]
fn parse_mandatory_constraint() {
    let input = "# T\n\n## Entity Types\n\nPerson(.Name) is an entity type.\nCountry(.Code) is an entity type.\n\n## Fact Types\n\nPerson was born in Country.\n\n## Constraints\n\nEach Person was born in some Country.";
    let ir = parse_markdown(input).unwrap();
    assert!(ir.constraints.iter().any(|c| c.kind == "MC"));
}

#[test]
fn parse_exactly_one_as_uc_plus_mc() {
    let input = "# T\n\n## Entity Types\n\nPerson(.Name) is an entity type.\nCountry(.Code) is an entity type.\n\n## Fact Types\n\nPerson was born in Country.\n\n## Constraints\n\nEach Person was born in exactly one Country.";
    let ir = parse_markdown(input).unwrap();
    assert!(ir.constraints.iter().any(|c| c.kind == "UC"));
    assert!(ir.constraints.iter().any(|c| c.kind == "MC"));
}

#[test]
fn parse_frequency_constraint() {
    let input = "# T\n\n## Entity Types\n\nCustomer(.Id) is an entity type.\nRequest(.Id) is an entity type.\n\n## Fact Types\n\nCustomer submits Request.\n\n## Constraints\n\nEach Customer submits at least 1 and at most 5 Request.";
    let ir = parse_markdown(input).unwrap();
    assert!(ir.constraints.iter().any(|c| c.kind == "FC"));
}

#[test]
fn parse_deontic_forbidden() {
    let input = "# T\n\n## Entity Types\n\nPerson(.Name) is an entity type.\nCountry(.Code) is an entity type.\n\n## Fact Types\n\nPerson was born in Country.\n\n## Deontic Constraints\n\nIt is forbidden that the same Person was born in more than one Country.";
    let ir = parse_markdown(input).unwrap();
    assert!(ir.constraints.iter().any(|c| c.modality == "deontic"));
}
```

- [ ] **Step 2: Implement fact type and constraint parsing**

Extend `parse_fact_types_and_constraints` to recognize:
- Binary fact types: lines with exactly two known nouns connected by verb text
- "Each X R at most one Y" → UC
- "Each X R some Y" → MC
- "Each X R exactly one Y" → UC + MC
- "at least N and at most M" → FC
- "It is forbidden/obligatory that" → deontic modality
- Constraint spans linking to fact type IDs

- [ ] **Step 3: Run tests**

Run: `cd crates/fol-engine && cargo test parse_forml2 -- --nocapture`
Expected: All constraint pattern tests PASS

- [ ] **Step 4: Commit**

```bash
git add crates/fol-engine/src/parse_forml2.rs
git commit -m "feat: parse fact types and constraint patterns (UC, MC, FC, deontic)"
```

---

### Task 8: Parse FORML 2 Derivation Rules

**Files:**
- Modify: `crates/fol-engine/src/parse_forml2.rs`

Replace the `:=` syntax from parse_rule.rs with Halpin's established forms.

- [ ] **Step 1: Write failing tests for iff/if derivation rules**

```rust
#[test]
fn parse_iff_derivation_rule() {
    let input = "# T\n\n## Entity Types\n\nPerson(.Name) is an entity type.\n\n## Derivation Rules\n\nPerson1 is an uncle of Person2 iff Person1 is a brother of some Person3 who is a parent of Person2.";
    let ir = parse_markdown(input).unwrap();
    assert_eq!(ir.derivation_rules.len(), 1);
    assert!(ir.derivation_rules[0].text.contains("iff"));
}

#[test]
fn parse_subtype_derivation() {
    let input = "# T\n\n## Entity Types\n\nPerson(.Name) is an entity type.\nCountry(.Code) is an entity type.\nAustralian(.Name) is an entity type.\n\n## Derivation Rules\n\nEach Australian is a Person who was born in Country 'AU'.";
    let ir = parse_markdown(input).unwrap();
    assert_eq!(ir.derivation_rules.len(), 1);
}

#[test]
fn parse_aggregate_count() {
    let input = "# T\n\n## Entity Types\n\nDept(.Name) is an entity type.\nRank(.Code) is an entity type.\nAcademic(.Id) is an entity type.\nQuantity is a value type.\n\n## Derivation Rules\n\nQuantity = count each Academic who has Rank and works for Dept.";
    let ir = parse_markdown(input).unwrap();
    assert_eq!(ir.derivation_rules.len(), 1);
}

#[test]
fn parse_attribute_style_derivation() {
    let input = "# T\n\n## Entity Types\n\nPerson(.Name) is an entity type.\n\n## Derivation Rules\n\nFor each Person: uncle = brother of parent.";
    let ir = parse_markdown(input).unwrap();
    assert_eq!(ir.derivation_rules.len(), 1);
}
```

- [ ] **Step 2: Implement derivation rule parsing**

Recognize patterns:
- `X iff Y` — full biconditional derivation
- `X if Y` — partial derivation (sufficient condition)
- `Each X is a Y who Z` — subtype derivation
- `count each X who Y` — aggregation with count
- `sum(roleName)` — aggregation with sum
- `For each X: y = expr` — attribute style
- Variable binding: `who`, `that`, `some`, subscripts (Person1, Person2)

- [ ] **Step 3: Run tests**

Run: `cd crates/fol-engine && cargo test parse_forml2 -- --nocapture`
Expected: All derivation rule tests PASS

- [ ] **Step 4: Commit**

```bash
git add crates/fol-engine/src/parse_forml2.rs
git commit -m "feat: parse FORML 2 derivation rules (iff, if, subtype, aggregate, attribute style)"
```

---

### Task 9: Parse State Machines

**Files:**
- Modify: `crates/fol-engine/src/parse_forml2.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn parse_state_machine() {
    let input = "# T\n\n## Entity Types\n\nOrder(.Id) is an entity type.\n\n## Instance Facts\n\nState Machine Definition 'Order' is for Noun 'Order'.\nStatus 'Draft' is initial in State Machine Definition 'Order'.\nTransition 'place' is defined in State Machine Definition 'Order'.\n  Transition 'place' is from Status 'Draft'.\n  Transition 'place' is to Status 'Placed'.";
    let ir = parse_markdown(input).unwrap();
    assert!(ir.state_machines.contains_key("Order"));
    let sm = &ir.state_machines["Order"];
    assert_eq!(sm.noun_name, "Order");
    assert!(sm.transitions.iter().any(|t| t.event == "place" && t.from == "Draft" && t.to == "Placed"));
}
```

- [ ] **Step 2: Implement state machine parsing from instance facts**

Recognize the reading-based state machine patterns:
- `State Machine Definition 'X' is for Noun 'Y'.`
- `Status 'S' is initial in State Machine Definition 'X'.`
- `Transition 'E' is defined in State Machine Definition 'X'.`
- `Transition 'E' is from Status 'S1'.`
- `Transition 'E' is to Status 'S2'.`

- [ ] **Step 3: Run tests**

Run: `cd crates/fol-engine && cargo test parse_state_machine -- --nocapture`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add crates/fol-engine/src/parse_forml2.rs
git commit -m "feat: parse state machines from instance facts"
```

---

### Task 10: WASM Export — load_readings

**Files:**
- Modify: `crates/fol-engine/src/lib.rs`

Add the `load_readings(markdown)` WASM export that replaces `load_ir(ir_json)`.

- [ ] **Step 1: Add load_readings export**

```rust
/// Load domain from FORML 2 markdown. Parses readings directly — no JSON IR intermediary.
/// This is the self-hosting path: readings → engine → compiled model.
#[wasm_bindgen]
pub fn load_readings(markdown: &str) -> Result<(), JsValue> {
    let ir = parse_forml2::parse_markdown(markdown)
        .map_err(|e| JsValue::from_str(&format!("Failed to parse readings: {}", e)))?;
    let model = compile::compile(&ir);
    let mut store = state_store().lock().unwrap();
    *store = Some(CompiledState { ir, model });
    Ok(())
}
```

- [ ] **Step 2: Write a Rust test that round-trips readings**

```rust
#[test]
fn load_readings_roundtrip() {
    let markdown = r#"# Orders

## Entity Types

Order(.Order Number) is an entity type.
Customer(.Name) is an entity type.

## Fact Types

Order was placed by Customer.

## Constraints

Each Order was placed by exactly one Customer.

## Instance Facts

State Machine Definition 'Order' is for Noun 'Order'.
Status 'Draft' is initial in State Machine Definition 'Order'.
Transition 'place' is defined in State Machine Definition 'Order'.
  Transition 'place' is from Status 'Draft'.
  Transition 'place' is to Status 'Placed'.
"#;
    let ir = parse_forml2::parse_markdown(markdown).unwrap();
    assert_eq!(ir.nouns.len(), 2);
    assert!(ir.constraints.len() >= 2); // UC + MC from "exactly one"
    assert!(ir.state_machines.contains_key("Order"));

    let model = compile::compile(&ir);
    assert!(!model.state_machines.is_empty());
}
```

- [ ] **Step 3: Run test**

Run: `cd crates/fol-engine && cargo test load_readings_roundtrip -- --nocapture`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add crates/fol-engine/src/lib.rs
git commit -m "feat: add load_readings WASM export — readings in, compiled model out"
```

---

### Task 11: Validate Against Existing Readings

**Files:**
- Modify: `crates/fol-engine/src/parse_forml2.rs` (tests)

Test the parser against the actual readings files in the repo.

- [ ] **Step 1: Write tests that parse real readings**

```rust
#[test]
fn parse_core_readings() {
    let core = include_str!("../../../readings/core.md");
    let ir = parse_markdown(core).unwrap();
    assert!(ir.nouns.contains_key("Noun"));
    assert!(ir.nouns.contains_key("Reading"));
    assert!(ir.nouns.contains_key("Constraint"));
}

#[test]
fn parse_state_readings() {
    let state = include_str!("../../../readings/state.md");
    let ir = parse_markdown(state).unwrap();
    assert!(ir.nouns.contains_key("Status"));
    assert!(ir.nouns.contains_key("Transition"));
}
```

- [ ] **Step 2: Fix parser issues found by real readings**

The real readings will expose edge cases: multi-word nouns, cross-references, subsection headers, indented constraints. Fix them iteratively.

- [ ] **Step 3: Run all tests**

Run: `cd crates/fol-engine && cargo test -- --nocapture`
Expected: All tests PASS including real readings

- [ ] **Step 4: Commit**

```bash
git add crates/fol-engine/src/parse_forml2.rs
git commit -m "test: validate FORML 2 parser against core.md and state.md readings"
```

---

### Task 12: Delete parse_rule.rs and TypeScript Parser

**Files:**
- Delete: `crates/fol-engine/src/parse_rule.rs`
- Modify: `crates/fol-engine/src/lib.rs` — remove `pub mod parse_rule`
- Delete: `src/claims/ingest.ts`, `src/claims/tokenize.ts`, `src/claims/constraints.ts`, `src/claims/steps.ts`, `src/claims/scope.ts`, `src/claims/batch-builder.ts`
- Delete: all `src/claims/*.test.ts`
- Modify: `src/api/router.ts` — update imports to use WASM `load_readings` instead of TS parser

- [ ] **Step 1: Remove parse_rule.rs**

Delete the file. Remove `pub mod parse_rule;` from lib.rs. Update any imports in compile.rs or evaluate.rs that reference parse_rule types.

- [ ] **Step 2: Delete TypeScript claims parser**

```bash
rm -rf src/claims/
```

- [ ] **Step 3: Update router.ts imports**

Replace any `import ... from '../claims/...'` with calls to the WASM `load_readings` export. The router receives markdown text and passes it to WASM.

- [ ] **Step 4: Run all tests**

Run: `cd crates/fol-engine && cargo test` and `yarn test`
Expected: Rust tests PASS. TS tests that depended on claims/ will need updating or removal.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "refactor: delete TypeScript parser and parse_rule.rs — readings parsed by Rust engine"
```

---

### Task 13: Write Syntax Readings (Self-Hosting Target)

**Files:**
- Create: `readings/syntax.md`

Express the FORML 2 grammar as readings that the engine can evaluate. This is the self-hosting milestone: the grammar IS data, not code.

- [ ] **Step 1: Write syntax.md**

```markdown
# FORML 2 Syntax

This domain defines the grammar of FORML 2 as derivation rules.
The engine evaluates these rules against input text to parse readings.

## Entity Types

Line(.Number) is an entity type.
Token(.Text) is an entity type.
Section(.Name) is an entity type.

## Fact Types

Line has Text.
Line is in Section.
Line has Token at Position.

## Derivation Rules

Line declares Entity Type iff that Line has Text that contains
  sequence 'is an entity type'.

Line declares Value Type iff that Line has Text that contains
  sequence 'is a value type'.

Line declares Subtype iff that Line has Text that contains
  sequence 'is a subtype of'.

Line declares Fact Type iff that Line is in Section 'Fact Types'
  and that Line has Text that contains at least two known Nouns.

Line declares Uniqueness Constraint iff that Line has Text that
  starts with 'Each' and contains 'at most one'.

Line declares Mandatory Constraint iff that Line has Text that
  starts with 'Each' and contains 'some' and does not contain
  'at most'.

Line declares Combined Constraint iff that Line has Text that
  starts with 'Each' and contains 'exactly one'.

Line declares Derivation Rule iff that Line has Text that contains 'iff'
  or that Line has Text that contains 'is derived as'.
```

- [ ] **Step 2: Write a test that loads syntax.md**

```rust
#[test]
fn parse_syntax_readings() {
    let syntax = include_str!("../../../readings/syntax.md");
    let ir = parse_markdown(syntax).unwrap();
    assert!(ir.nouns.contains_key("Line"));
    assert!(ir.nouns.contains_key("Token"));
    assert!(ir.derivation_rules.len() > 0);
}
```

- [ ] **Step 3: Run test**

Run: `cd crates/fol-engine && cargo test parse_syntax_readings -- --nocapture`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add readings/syntax.md crates/fol-engine/src/parse_forml2.rs
git commit -m "feat: express FORML 2 grammar as readings (self-hosting target)"
```

---

### Task 14: Update GraphDL Skill

**Files:**
- Modify: `~/.claude/skills/graphdl/graphdl.md`

Add sections documenting: combining forms, derivation rule syntax, AREST execution model, primitive functions reference.

- [ ] **Step 1: Add Combining Forms section**

After the "Derivation Rules" section, add:

```markdown
## Combining Forms (Backus FP Algebra)

AREST maps Backus's combining forms to domain primitives:

| Backus Form | AREST Interpretation | Example |
|---|---|---|
| Construction [f1,...,fn] | Graph Schema (CONS of Roles) | [Sel(Customer), Sel(Email)] |
| Selector s | Role at position s | Sel(2) = second role |
| Composition f . g | Derivation rule chain | uncle = brother . parent |
| Condition p -> f; g | Constraint evaluation, guards | (valid -> accept; reject) |
| Apply to all (alpha f) | Population traversal | alpha(Sel(1)) over facts |
| Insert /f | Aggregation (sum, count, exists) | /+ = sum, /or = exists |
| Binary to unary (bu f x) | Partial application (query) | bu(eq, "org-1") |
| Filter(p) | Query: select matching facts | Filter(bu eq "owner") |
| While (while p f) | Bounded iteration | (while not-done step) |
| Constant x-bar | Literal value in a reading | "Draft"-bar |
```

- [ ] **Step 2: Add FORML 2 Derivation Rule Syntax section**

```markdown
## FORML 2 Derivation Rule Syntax

### Full Derivation (iff-rules)

Relational style:
\```
Person1 is an uncle of Person2 iff Person1 is a brother of
  some Person3 who is a parent of Person2.
\```

Attribute style:
\```
For each Person: uncle = brother of parent.
\```

### Partial Derivation (if-rules)

\```
Person1 is a Grandparent if Person1 is a parent of some Person2
  who is a parent of some Person3.
\```

### Subtype Derivation

\```
Each Australian is a Person who was born in Country 'AU'.
\```

### Aggregation

\```
Quantity = count each Academic who has Rank and works for Dept.
For each PublishedBook, totalCopiesSold = sum(copiesSoldInYear).
\```

### Variable Binding

- Pronouns: `who` (personal), `that` (impersonal), `some` (existential)
- Subscripts: Person1, Person2 (when same type in multiple roles)
- Head variables implicitly universally quantified
- Body-only variables implicitly existentially quantified
```

- [ ] **Step 3: Add AREST Execution Model section**

```markdown
## AREST Execution Model

Command : Population -> (Population', Representation)

### Create Pipeline

create = emit . validate . derive . resolve

- resolve: apply reference scheme selector to determine entity identity
- derive: forward-chain derivation rules to fixed point (including SM initialization)
- validate: evaluate constraints against complete population (base + derived)
- emit: produce representation with HATEOAS links

### HATEOAS as Projection

links(s) = Filter(p) : T where T is subset of P

Transitions are facts in the population. The API renders the population. Valid next actions ARE the representation. No URL routing table consulted.

### State Machines as Fold

run_machine = /transition : <e1, ..., en>

Each transition is a Condition. The first matching fires; rest fall through to identity.
```

- [ ] **Step 4: Commit**

```bash
git add ~/.claude/skills/graphdl/graphdl.md
git commit -m "docs: add combining forms, derivation syntax, and AREST model to graphdl skill"
```

---

### Task 15: End-to-End Verification

**Files:**
- Modify: `crates/fol-engine/src/parse_forml2.rs` (tests)

The whitepaper's order example, from readings to HATEOAS response.

- [ ] **Step 1: Write the end-to-end test**

```rust
#[test]
fn whitepaper_order_example_end_to_end() {
    // The whitepaper's example (Section 4)
    let markdown = r#"# Orders

## Entity Types

Order(.Order Number) is an entity type.
Customer(.Name) is an entity type.

## Fact Types

Order was placed by Customer.

## Constraints

Each Order was placed by exactly one Customer.

## Instance Facts

State Machine Definition 'Order' is for Noun 'Order'.
Status 'Draft' is initial in State Machine Definition 'Order'.
Transition 'place' is defined in State Machine Definition 'Order'.
  Transition 'place' is from Status 'Draft'.
  Transition 'place' is to Status 'Placed'.
Transition 'ship' is defined in State Machine Definition 'Order'.
  Transition 'ship' is from Status 'Placed'.
  Transition 'ship' is to Status 'Shipped'.
Transition 'cancel' is defined in State Machine Definition 'Order'.
  Transition 'cancel' is from Status 'Order'.
  Transition 'cancel' is to Status 'Cancelled'.
"#;

    let ir = parse_forml2::parse_markdown(markdown).unwrap();
    let model = compile::compile(&ir);

    // Create entity: POST /orders {"customer":"acme"}
    let pop = Population { facts: std::collections::HashMap::new() };
    let mut fields = std::collections::HashMap::new();
    fields.insert("customer".to_string(), "acme".to_string());
    fields.insert("orderNumber".to_string(), "ORD-1".to_string());

    let cmd = arest::Command::CreateEntity {
        noun: "Order".to_string(),
        domain: "orders".to_string(),
        id: Some("ORD-1".to_string()),
        fields,
    };

    let result = arest::apply_command(&model, &cmd, &pop);

    assert!(!result.rejected);
    assert_eq!(result.status.as_deref(), Some("Draft"));
    assert!(result.transitions.iter().any(|t| t.event == "place"));
    assert_eq!(result.entities[0].data["customer"], "acme");

    // Transition: place
    let transition = arest::Command::Transition {
        entity_id: "ORD-1".to_string(),
        event: "place".to_string(),
        domain: "orders".to_string(),
        current_status: Some("Draft".to_string()),
    };
    let result2 = arest::apply_command(&model, &transition, &result.population);

    assert_eq!(result2.status.as_deref(), Some("Placed"));
    assert!(result2.transitions.iter().any(|t| t.event == "ship"));
    assert!(!result2.transitions.iter().any(|t| t.event == "place"));
}
```

- [ ] **Step 2: Run test**

Run: `cd crates/fol-engine && cargo test whitepaper_order_example -- --nocapture`
Expected: PASS — the full pipeline from markdown readings to HATEOAS transitions works

- [ ] **Step 3: Run full test suite**

Run: `cd crates/fol-engine && cargo test`
Expected: All tests PASS

- [ ] **Step 4: Commit**

```bash
git add crates/fol-engine/src/parse_forml2.rs
git commit -m "test: end-to-end whitepaper order example — readings to HATEOAS"
```
