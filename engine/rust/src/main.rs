//! The lambda-calculus substrate of pyarest, carried on Rust closures — the polyglot
//! kernel. The object union ATOM | SEQ | BOT is Scott-encoded on `Rc<dyn Fn(V) -> V>`,
//! mu is built by the Y combinator (mu = lfp tau, a fact about the code here as in
//! reduce.py), metacomposition is Backus's (rho <x1..xn>):y = (rho x1):<<x1..xn>, y>,
//! and the ONLY host-native parts mirror lam.py's own boundary walkers (native list
//! walks, NATEQ on leaf values, the definition frame). Everything above this file —
//! theta-1, the constraints, the machines, M — arrives as exported VALUES and reduces
//! unchanged: the port surface is exactly prims.BASE plus DEFS and cellkey, the
//! enumerable boundary of Cor. boundary.
//!
//! Protocol: stdin carries one JSON scenario
//!   {"d": V, "process": [[name, V], ...], "cases": [{"f": V, "x": V, "fuel": N}, ...]}
//! where V is string (a string atom) | integer | float | array (a sequence); fuel 0
//! means unbounded. One JSON per case on stdout: the reduced value, or null for bottom.
//! Under --serve the same scenarios stream one per line against a RETAINED store
//! (one JSON array of case results per line), and a line carrying "op" is a verb
//! request instead — the system's verb surface (python/protocol.py's table),
//! answered as one {"op", "result"|"error"} object; see "the verb surface" below.
//! Under --mcp the kernel serves the Model Context Protocol instead: it reads
//! newline-delimited JSON-RPC 2.0 over stdio against an apps directory of
//! persisted stores; see "the MCP binding" below.

mod cooks;
mod uilayout;

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Read;
use std::rc::Rc;

// ============================ leaves (the ORM value boundary) =================
#[derive(Clone, Debug)]
enum Leaf {
    S(String),
    I(i64),
    F(f64),
    AppTag, // the reserved application sentinel (identity in Python; a variant here)
}

impl Leaf {
    // NATEQ: same ORM value type AND equal (int vs float are DIFFERENT types here,
    // mirroring Python's `type(a) is type(b) and a == b`)
    fn nateq(&self, other: &Leaf) -> bool {
        match (self, other) {
            (Leaf::S(a), Leaf::S(b)) => a == b,
            (Leaf::I(a), Leaf::I(b)) => a == b,
            (Leaf::F(a), Leaf::F(b)) => a == b,
            (Leaf::AppTag, Leaf::AppTag) => true,
            _ => false,
        }
    }
}

// ============================ the value: closures or a leaf ===================
#[derive(Clone)]
enum V {
    F(Rc<dyn Fn(V) -> V>),
    L(Rc<Leaf>),
}

fn vf(f: impl Fn(V) -> V + 'static) -> V {
    V::F(Rc::new(f))
}

impl V {
    fn app(&self, x: V) -> V {
        match self {
            V::F(f) => f(x),
            V::L(_) => bot(), // applying a bare leaf is a host error; total here as bottom
        }
    }
    fn leaf(&self) -> Option<Rc<Leaf>> {
        match self {
            V::L(l) => Some(l.clone()),
            V::F(_) => None,
        }
    }
}

// ---- Church booleans and IF (branches are thunks, forced with a unit) ----
fn tru() -> V {
    vf(|a| vf(move |_b| a.clone()))
}
fn fls() -> V {
    vf(|_a| vf(move |b| b))
}
fn unit() -> V {
    V::L(Rc::new(Leaf::I(0)))
}
fn tobool(b: &V) -> bool {
    matches!(
        b.app(V::L(Rc::new(Leaf::I(1)))).app(V::L(Rc::new(Leaf::I(0)))).leaf().as_deref(),
        Some(Leaf::I(1))
    )
}

// ============================ Scott lists ====================================
fn nil() -> V {
    vf(|n| vf(move |_c| n.clone()))
}
fn cons(h: V, t: V) -> V {
    vf(move |_n| {
        let (h, t) = (h.clone(), t.clone());
        vf(move |c| c.app(h.clone()).app(t.clone()))
    })
}
fn lmatch(l: &V, on_nil: V, on_cons: V) -> V {
    l.app(on_nil).app(on_cons)
}
fn head(l: &V) -> V {
    lmatch(l, bot(), vf(|h| vf(move |_t| h.clone())))
}
fn tail(l: &V) -> V {
    lmatch(l, nil(), vf(|_h| vf(move |t| t)))
}
fn lnull(l: &V) -> V {
    lmatch(l, tru(), vf(|_h| vf(move |_t| fls())))
}

// ---- lambda combinators over Scott lists (ports of lam.py's Y-built terms: FOLDR,
// MAPL, APPEND, REVL, ANYBOT/SEQC's fold). Rust's named recursion stands in for the Y
// self-application with the identical unfolding; the bodies compose ONLY Scott-list
// operations (head/tail/cons/lnull), never native containers — one level down, not two. ----
fn foldr(f: &dyn Fn(V, V) -> V, z: V, l: &V) -> V {
    if tobool(&lnull(l)) {
        z
    } else {
        let rest = foldr(f, z, &tail(l));
        f(head(l), rest)
    }
}
fn mapl(f: &dyn Fn(V) -> V, l: &V) -> V {
    if tobool(&lnull(l)) {
        nil()
    } else {
        cons(f(head(l)), mapl(f, &tail(l)))
    }
}
fn lappend(p: &V, q: &V) -> V {
    if tobool(&lnull(p)) {
        q.clone()
    } else {
        cons(head(p), lappend(&tail(p), q))
    }
}
fn revl(l: &V) -> V {
    foldr(&|h, a| lappend(&a, &cons(h, nil())), nil(), l)
}
fn trans_rows(rl: &V) -> V {
    // lam.py's _trans_rows, term for term: atom row → ⊥; no rows → φ; all rows spent
    // → φ; ragged → ⊥; else CONS the head column onto the transpose of the tails
    let anybad = foldr(&|h, a| if !is_seq(&h) { tru() } else { a }, fls(), rl);
    if tobool(&anybad) { return bot(); }
    if tobool(&lnull(rl)) { return phi(); }
    let allspent = foldr(&|h, a| if tobool(&lnull(&list_of(&h))) { a } else { fls() }, tru(), rl);
    if tobool(&allspent) { return phi(); }
    let anyspent = foldr(&|h, a| if tobool(&lnull(&list_of(&h))) { tru() } else { a }, fls(), rl);
    if tobool(&anyspent) { return bot(); }
    let col = seq(mapl(&|r| head(&list_of(&r)), rl));
    let rest = trans_rows(&mapl(&|r| seq(tail(&list_of(&r))), rl));
    match shape(&rest) {
        Shape::Seq(restl) => seq(cons(col, restl)),
        _ => bot(),
    }
}

fn seqc_l(l: V) -> V {
    // SEQC: the ⊥-collapsing constructor via the ANYBOT fold (§11.2.1)
    let any = foldr(&|h, a| if isbot(&h) { tru() } else { a }, fls(), &l);
    if tobool(&any) { bot() } else { seq(l) }
}

// ============================ the object union ===============================
fn atom(l: Leaf) -> V {
    let l = Rc::new(l);
    vf(move |a| {
        let l = l.clone();
        vf(move |_s| {
            let (a, l) = (a.clone(), l.clone());
            vf(move |_b| a.app(V::L(l.clone())))
        })
    })
}
fn seq(l: V) -> V {
    vf(move |_a| {
        let l = l.clone();
        vf(move |s| {
            let l = l.clone();
            vf(move |_b| s.app(l.clone()))
        })
    })
}
fn bot() -> V {
    vf(|_a| vf(|_s| vf(|b| b)))
}
fn omatch(o: &V, on_atom: V, on_seq: V, on_bot: V) -> V {
    o.app(on_atom).app(on_seq).app(on_bot)
}

fn phi() -> V {
    seq(nil())
}

// ---- native boundary walkers (lam.py's _list/_aval/_items/from_lam style) ----
#[derive(Clone)]
enum Shape {
    Atom(Rc<Leaf>),
    Seq(V), // the Scott list payload
    Bot,
}
fn shape(o: &V) -> Shape {
    let box_: Rc<RefCell<Option<Shape>>> = Rc::new(RefCell::new(None));
    let b1 = box_.clone();
    let on_a = vf(move |v: V| {
        *b1.borrow_mut() = Some(Shape::Atom(v.leaf().unwrap_or_else(|| Rc::new(Leaf::I(0)))));
        unit()
    });
    let b2 = box_.clone();
    let on_s = vf(move |l: V| {
        *b2.borrow_mut() = Some(Shape::Seq(l));
        unit()
    });
    let _ = omatch(o, on_a, on_s, unit());
    let taken = box_.borrow_mut().take();
    taken.unwrap_or(Shape::Bot)
}
fn list_of(o: &V) -> V {
    // the Scott list inside a SEQ; NIL for an atom (mirrors lam.py's _list)
    match shape(o) {
        Shape::Seq(l) => l,
        _ => nil(),
    }
}
fn items(l: &V) -> Vec<V> {
    let mut out = Vec::new();
    let mut cur = l.clone();
    while !tobool(&lnull(&cur)) {
        out.push(head(&cur));
        cur = tail(&cur);
    }
    out
}
fn from_vec(xs: Vec<V>) -> V {
    let mut l = nil();
    for x in xs.into_iter().rev() {
        l = cons(x, l);
    }
    l
}
fn isbot(o: &V) -> bool {
    matches!(shape(o), Shape::Bot)
}
fn aval(o: &V) -> Option<Rc<Leaf>> {
    match shape(o) {
        Shape::Atom(l) => Some(l),
        _ => None,
    }
}

// structural object equality with NATEQ at the leaves (EQOBJ)
fn eqobj(a: &V, b: &V) -> bool {
    match (shape(a), shape(b)) {
        (Shape::Atom(x), Shape::Atom(y)) => x.nateq(&y),
        (Shape::Seq(x), Shape::Seq(y)) => {
            let (xi, yi) = (items(&x), items(&y));
            xi.len() == yi.len() && xi.iter().zip(yi.iter()).all(|(p, q)| eqobj(p, q))
        }
        _ => false,
    }
}

// SEQC: the bottom-collapsing constructor (§11.2.1 — a sequence containing ⊥ IS ⊥)
fn seqc(xs: Vec<V>) -> V {
    if xs.iter().any(isbot) {
        bot()
    } else {
        seq(from_vec(xs))
    }
}

// ============================ the definition frame ===========================
struct Frame {
    cells: Vec<(Leaf, V)>, // first match wins, mirrors defs._cells_of
    d: V,
    fuel: Option<i64>,
}

thread_local! {
    static FRAME: RefCell<Vec<Frame>> = RefCell::new(Vec::new());
    static REGISTRY: RefCell<HashMap<String, Rc<dyn Fn(V, V) -> V>>> = RefCell::new(HashMap::new());
    // the universal override interface: host-optimized twins of canonical definitions;
    // resolution prefers a twin, absence degrades to the canonical term (same result)
    static FAST: RefCell<HashMap<String, Rc<dyn Fn(V, V) -> V>>> = RefCell::new(HashMap::new());
    static PROCESS: RefCell<Vec<(String, V)>> = RefCell::new(Vec::new());
    // the INTERSECTION SOURCE definitions (shared/*.py, include!d verbatim below):
    // loaded once at startup, resolved by name like any compiled definition
    static CANON: RefCell<Vec<(String, V)>> = RefCell::new(Vec::new());
    // the native mirror of CANON: the same intersection-source definitions
    // converted to N once at startup, so the native carrier NEval resolves a
    // canon def (theta/ast/system/constraints) the same way the Scott mu does
    // when a partial process list does not carry it. It is the native twin of
    // the Scott path's CANON fallback, never a Scott fallback in disguise.
    static NCANON: RefCell<Vec<(String, N)>> = RefCell::new(Vec::new());
}

fn leaf_key(l: &Leaf) -> Option<String> {
    match l {
        Leaf::S(s) => Some(s.clone()),
        Leaf::I(i) => Some(format!("#i#{}", i)),
        _ => None,
    }
}

fn cells_of(d: &V) -> Vec<(Leaf, V)> {
    let mut out: Vec<(Leaf, V)> = Vec::new();
    for c in items(&list_of(d)) {
        let it = items(&list_of(&c));
        if it.len() == 3 {
            if let Some(l0) = aval(&it[0]) {
                if matches!(&*l0, Leaf::S(s) if s == "CELL") {
                    if let Some(k) = aval(&it[1]) {
                        if !out.iter().any(|(e, _)| e.nateq(&k)) {
                            out.push(((*k).clone(), it[2].clone()));
                        }
                    }
                }
            }
        }
    }
    out
}

fn step_get(key: &Leaf) -> Option<V> {
    FRAME.with(|f| {
        f.borrow().last().and_then(|fr| {
            fr.cells.iter().find(|(k, _)| k.nateq(key)).map(|(_, v)| v.clone())
        })
    })
}

fn step_d() -> Option<V> {
    FRAME.with(|f| f.borrow().last().map(|fr| fr.d.clone()))
}

fn consume_fuel() -> bool {
    FRAME.with(|f| {
        let mut b = f.borrow_mut();
        match b.last_mut() {
            None => true,
            Some(fr) => match fr.fuel {
                None => true,
                Some(ref mut n) => {
                    *n -= 1;
                    *n > 0
                }
            },
        }
    })
}

// ============================ the application node ===========================
fn mkapp(f: V, x: V) -> V {
    seq(cons(atom(Leaf::AppTag), cons(f, cons(x, nil()))))
}
fn isapp(o: &V) -> bool {
    if let Shape::Seq(l) = shape(o) {
        if !tobool(&lnull(&l)) {
            if let Some(h) = aval(&head(&l)) {
                return matches!(&*h, Leaf::AppTag);
            }
        }
    }
    false
}

// ============================ mu = Y(tau) ====================================
fn y(f: V) -> V {
    let fc = f.clone();
    let half = vf(move |x: V| {
        let xc = x.clone();
        fc.app(vf(move |v: V| xc.app(xc.clone()).app(v)))
    });
    half.clone().app(half)
}

fn make_mu() -> V {
    let tau = vf(|mu: V| {
        vf(move |e: V| {
            if !isapp(&e) {
                return e; // a value is its own meaning
            }
            if !consume_fuel() {
                return bot(); // supervision: exhaustion bottoms
            }
            let it = items(&list_of(&e));
            let (op, arg) = (it[1].clone(), it[2].clone());
            let fr = mu.app(op); // reduce the operator via mu
            let x = mu.app(arg); // reduce the operand once (call-by-value)
            if isbot(&x) {
                return bot(); // ⊥-preservation short-circuit (§11.2.1)
            }
            match shape(&fr) {
                Shape::Atom(a) => {
                    if let Some(sd) = step_get(&a) {
                        return mu.app(mkapp(sd, x)); // the step's DEFS cell first
                    }
                    if let Some(key) = leaf_key(&a) {
                        let tw = FAST.with(|r| r.borrow().get(&key).cloned());
                        if let Some(impl_) = tw {
                            return mu.app(impl_(mu.clone(), x)); // the override twin first
                        }
                        let reg = REGISTRY
                            .with(|r| r.borrow().get(&key).cloned());
                        if let Some(impl_) = reg {
                            return mu.app(impl_(mu.clone(), x)); // the canonical term
                        }
                        let proc_ = PROCESS.with(|p| {
                            p.borrow().iter().rev().find(|(n, _)| *n == key).map(|(_, v)| v.clone())
                        });
                        if let Some(obj) = proc_ {
                            return mu.app(mkapp(obj, x)); // compiled process def: mu(o : x)
                        }
                        let can = CANON.with(|p| {
                            p.borrow().iter().rev().find(|(n, _)| *n == key).map(|(_, v)| v.clone())
                        });
                        if let Some(obj) = can {
                            return mu.app(mkapp(obj, x)); // intersection-source def: mu(o : x)
                        }
                    }
                    bot()
                }
                Shape::Seq(l) => {
                    // metacomposition on the head: (rho <x1..>):y = (rho x1):<<x1..>, y>
                    let pair = seq(cons(fr.clone(), cons(x, nil())));
                    mu.app(mkapp(head(&l), pair))
                }
                Shape::Bot => bot(),
            }
        })
    });
    y(tau)
}

// ============================ the base primitives ============================
fn at() -> V {
    atom(Leaf::S("T".into()))
}
fn af() -> V {
    atom(Leaf::S("F".into()))
}
fn bool2a(b: bool) -> V {
    if b { at() } else { af() }
}

fn nth(o: &V, i: usize) -> V {
    let it = items(&list_of(o));
    if i < it.len() { it[i].clone() } else { bot() }
}

fn pair_b(o: &V) -> bool {
    matches!(shape(o), Shape::Seq(_)) && items(&list_of(o)).len() == 2
}
fn is_seq(o: &V) -> bool {
    matches!(shape(o), Shape::Seq(_))
}

fn num(l: &Leaf) -> Option<f64> {
    match l {
        Leaf::I(i) => Some(*i as f64),
        Leaf::F(f) => Some(*f),
        _ => None,
    }
}

// Coercion for arithmetic AND comparison (mirrors delta._tonum /
// prims._tonum: int first, then float): the store carries lexical atoms, so
// a numeric-looking string is a number to + and kin — and to le/ge/lt/gt,
// which order numerically wherever BOTH sides parse (the claude analytics
// family: a singleton sum's string '11000' beside the int 4997) and fall
// back to lexical only for non-numeric string pairs.
fn cint(l: &Leaf) -> Option<i64> {
    match l {
        Leaf::I(i) => Some(*i),
        Leaf::S(s) => s.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn cnum(l: &Leaf) -> Option<f64> {
    match l {
        Leaf::I(i) => Some(*i as f64),
        Leaf::F(f) => Some(*f),
        Leaf::S(s) => s.trim().parse::<f64>().ok(),
        Leaf::AppTag => None,
    }
}

type Prim = Rc<dyn Fn(V, V) -> V>;

fn register(name: &str, p: Prim) {
    REGISTRY.with(|r| {
        r.borrow_mut().insert(name.to_string(), p);
    });
}

fn register_base() {
    // selectors 1..32 (a number is a selector; key format mirrors leaf_key)
    for i in 1..=32i64 {
        let idx = (i - 1) as usize;
        register(
            &format!("#i#{}", i),
            Rc::new(move |_mu, o| match shape(&o) {
                Shape::Seq(_) => nth(&o, idx),
                _ => bot(),
            }),
        );
    }
    register("tl", Rc::new(|_mu, o| match shape(&o) {
        Shape::Seq(l) => {
            if tobool(&lnull(&l)) { bot() } else { seq(tail(&l)) }
        }
        _ => bot(),
    }));
    register("id", Rc::new(|_mu, o| o));
    register("atom", Rc::new(|_mu, o| match shape(&o) {
        Shape::Atom(_) => at(),
        Shape::Seq(l) => bool2a(tobool(&lnull(&l))), // PHI is both atom and sequence
        Shape::Bot => bot(),
    }));
    register("null", Rc::new(|_mu, o| match shape(&o) {
        Shape::Atom(_) => af(),
        Shape::Seq(l) => bool2a(tobool(&lnull(&l))),
        Shape::Bot => bot(),
    }));
    register("eq", Rc::new(|_mu, o| {
        if !pair_b(&o) { return bot(); }
        bool2a(eqobj(&nth(&o, 0), &nth(&o, 1)))
    }));
    register("apndl", Rc::new(|_mu, o| {
        if !(pair_b(&o) && is_seq(&nth(&o, 1))) { return bot(); }
        // APNDL = SEQ(CONS(HEAD(_list o))(_list(HEAD(TAIL(_list o)))))
        let l = list_of(&o);
        seq(cons(head(&l), list_of(&head(&tail(&l)))))
    }));
    register("apndr", Rc::new(|_mu, o| {
        if !(pair_b(&o) && is_seq(&nth(&o, 0))) { return bot(); }
        // APNDR = SEQ(APPEND(_list(HEAD(_list o)))(CONS(HEAD(TAIL(_list o)))(NIL)))
        let l = list_of(&o);
        seq(lappend(&list_of(&head(&l)), &cons(head(&tail(&l)), nil())))
    }));
    register("distl", Rc::new(|_mu, o| {
        if !(pair_b(&o) && is_seq(&nth(&o, 1))) { return bot(); }
        // DISTL = SEQ(MAPL(λy. SEQ⟨x,y⟩)(ys))
        let l = list_of(&o);
        let x = head(&l);
        seq(mapl(&|yv| seq(cons(x.clone(), cons(yv, nil()))), &list_of(&head(&tail(&l)))))
    }));
    register("distr", Rc::new(|_mu, o| {
        if !(pair_b(&o) && is_seq(&nth(&o, 0))) { return bot(); }
        // DISTR = SEQ(MAPL(λx. SEQ⟨x,y⟩)(xs))
        let l = list_of(&o);
        let yv = head(&tail(&l));
        seq(mapl(&|x| seq(cons(x, cons(yv.clone(), nil()))), &list_of(&head(&l))))
    }));
    register("length", Rc::new(|_mu, o| match shape(&o) {
        Shape::Seq(l) => atom(Leaf::I(items(&l).len() as i64)),
        _ => bot(),
    }));
    register("reverse", Rc::new(|_mu, o| match shape(&o) {
        Shape::Seq(l) => seq(revl(&l)),                      // FOLDR-built REVL (lam.py)
        _ => bot(),
    }));
    register("cat", Rc::new(|_mu, o| {
        if !(pair_b(&o) && is_seq(&nth(&o, 0)) && is_seq(&nth(&o, 1))) { return bot(); }
        // cat = SEQ(APPEND(_list 1)(_list 2))
        let l = list_of(&o);
        seq(lappend(&list_of(&head(&l)), &list_of(&head(&tail(&l)))))
    }));
    register("not", Rc::new(|_mu, o| {
        if eqobj(&o, &at()) { af() } else if eqobj(&o, &af()) { at() } else { bot() }
    }));
    register("and", Rc::new(|_mu, o| {
        if !pair_b(&o) { return bot(); }
        let (p, q) = (nth(&o, 0), nth(&o, 1));
        let tf = |v: &V| eqobj(v, &at()) || eqobj(v, &af());
        if !(tf(&p) && tf(&q)) { return bot(); }
        bool2a(eqobj(&p, &at()) && eqobj(&q, &at()))
    }));
    register("or", Rc::new(|_mu, o| {
        if !pair_b(&o) { return bot(); }
        let (p, q) = (nth(&o, 0), nth(&o, 1));
        let tf = |v: &V| eqobj(v, &at()) || eqobj(v, &af());
        if !(tf(&p) && tf(&q)) { return bot(); }
        bool2a(eqobj(&p, &at()) || eqobj(&q, &at()))
    }));
    register("1r", Rc::new(|_mu, o| match shape(&o) {
        Shape::Seq(l) => head(&revl(&l)),                     // 1r = HEAD ∘ REVL (lam.py)
        _ => bot(),
    }));
    register("tlr", Rc::new(|_mu, o| match shape(&o) {
        Shape::Seq(l) => {
            if tobool(&lnull(&l)) { return bot(); }           // tlr:φ = ⊥
            seq(revl(&tail(&revl(&l))))                       // REVL∘TAIL∘REVL (lam.py)
        }
        _ => bot(),
    }));
    register("trans", Rc::new(|_mu, o| match shape(&o) {
        Shape::Seq(rl) => trans_rows(&rl),                    // the Y-recursive term (lam.py)
        _ => bot(),
    }));
    register("rotl", Rc::new(|_mu, o| match shape(&o) {
        Shape::Seq(l) => {
            if tobool(&lnull(&l)) { return phi(); }
            seq(lappend(&tail(&l), &cons(head(&l), nil())))   // APPEND(t)(⟨h⟩) (lam.py)
        }
        _ => bot(),
    }));
    register("rotr", Rc::new(|_mu, o| match shape(&o) {
        Shape::Seq(l) => {
            if tobool(&lnull(&l)) { return phi(); }
            let r = revl(&l);                                 // ⟨CONS(HEAD r)(REVL(TAIL r))⟩
            seq(cons(head(&r), revl(&tail(&r))))
        }
        _ => bot(),
    }));
    // arithmetic and comparison at the value boundary (same-type or numeric domain,
    // int/float one numeric domain for arithmetic; int+int stays int; ÷ is always
    // float and ÷0 = ⊥ — mirroring Python semantics exactly)
    fn arith(f_i: fn(i64, i64) -> i64, f_f: fn(f64, f64) -> f64) -> Prim {
        Rc::new(move |_mu, o| {
            if !pair_b(&o) { return bot(); }
            match (aval(&nth(&o, 0)), aval(&nth(&o, 1))) {
                (Some(a), Some(b)) => match (cint(&a), cint(&b)) {
                    // int first (lexical ints included) — mirrors _tonum exactly
                    (Some(x), Some(y)) => atom(Leaf::I(f_i(x, y))),
                    _ => match (cnum(&a), cnum(&b)) {
                        (Some(x), Some(y)) => atom(Leaf::F(f_f(x, y))),
                        _ => bot(),
                    },
                },
                _ => bot(),
            }
        })
    }
    register("+", arith(|a, b| a + b, |a, b| a + b));
    register("-", arith(|a, b| a - b, |a, b| a - b));
    register("*", arith(|a, b| a * b, |a, b| a * b));
    register("div", Rc::new(|_mu, o| {
        if !pair_b(&o) { return bot(); }
        match (aval(&nth(&o, 0)).and_then(|l| num(&l)), aval(&nth(&o, 1)).and_then(|l| num(&l))) {
            (Some(a), Some(b)) if b != 0.0 => atom(Leaf::F(a / b)), // Python / is float
            _ => bot(),
        }
    }));
    fn cmp(rel_n: fn(f64, f64) -> bool, rel_s: fn(&str, &str) -> bool) -> Prim {
        Rc::new(move |_mu, o| {
            if !pair_b(&o) { return bot(); }
            match (aval(&nth(&o, 0)), aval(&nth(&o, 1))) {
                (Some(a), Some(b)) => match (cnum(&a), cnum(&b)) {
                    (Some(x), Some(y)) => bool2a(rel_n(x, y)),
                    _ => match (&*a, &*b) {
                        (Leaf::S(x), Leaf::S(y)) => bool2a(rel_s(x, y)),
                        _ => bot(),
                    },
                },
                _ => bot(),
            }
        })
    }
    register("ge", cmp(|a, b| a >= b, |a, b| a >= b));
    register("gt", cmp(|a, b| a > b, |a, b| a > b));
    register("le", cmp(|a, b| a <= b, |a, b| a <= b));
    register("lt", cmp(|a, b| a < b, |a, b| a < b));
    register("apply", Rc::new(|mu, o| {
        if !pair_b(&o) { return bot(); }
        mu.app(mkapp(nth(&o, 0), nth(&o, 1)))
    }));

    // controlling operators: impl(mu)(<<OP, params>, y>) per metacomposition
    fn params(a: &V) -> Vec<V> {
        let whole = nth(a, 0);
        let mut it = items(&list_of(&whole));
        if it.is_empty() { Vec::new() } else { it.split_off(1) }
    }
    register("COMP", Rc::new(|_mu, a| {
        let y = nth(&a, 1);
        params(&a).into_iter().rev().fold(y, |acc, f| mkapp(f, acc))
    }));
    register("CONS", Rc::new(|mu, a| {
        let y = nth(&a, 1);
        seqc(params(&a).into_iter().map(|f| mu.app(mkapp(f, y.clone()))).collect())
    }));
    register("CONST", Rc::new(|_mu, a| {
        let quoted = nth(&nth(&a, 0), 1);
        match shape(&nth(&a, 1)) {
            Shape::Bot => bot(), // ⊥-preserving: x̄ : ⊥ = ⊥
            _ => quoted,
        }
    }));
    register("ALPHA", Rc::new(|mu, a| {
        let f = nth(&nth(&a, 0), 1);
        match shape(&nth(&a, 1)) {
            Shape::Seq(l) => {
                seqc(items(&l).into_iter().map(|yi| mu.app(mkapp(f.clone(), yi))).collect())
            }
            _ => bot(),
        }
    }));
    register("COND", Rc::new(|mu, a| {
        let whole = nth(&a, 0);
        let (p, f, g, yv) = (nth(&whole, 1), nth(&whole, 2), nth(&whole, 3), nth(&a, 1));
        let pv = mu.app(mkapp(p, yv.clone()));
        if eqobj(&pv, &at()) {
            mu.app(mkapp(f, yv))
        } else if eqobj(&pv, &af()) {
            mu.app(mkapp(g, yv))
        } else {
            bot()
        }
    }));
    register("INSERT", Rc::new(|mu, a| {
        let (whole, yv) = (nth(&a, 0), nth(&a, 1));
        let f = nth(&whole, 1);
        let yl = items(&list_of(&yv));
        if yl.is_empty() { return bot(); }
        if yl.len() == 1 { return yl[0].clone(); }
        let rest = mu.app(mkapp(whole, seq(from_vec(yl[1..].to_vec()))));
        mkapp(f, seqc(vec![yl[0].clone(), rest]))
    }));
    register("WHILE", Rc::new(|mu, a| {
        let (whole, yv) = (nth(&a, 0), nth(&a, 1));
        let (p, f) = (nth(&whole, 1), nth(&whole, 2));
        let pv = mu.app(mkapp(p, yv.clone()));
        if eqobj(&pv, &at()) {
            mkapp(whole, mkapp(f, yv))
        } else if eqobj(&pv, &af()) {
            yv
        } else {
            bot()
        }
    }));
    register("BU", Rc::new(|mu, a| {
        let whole = nth(&a, 0);
        mu.app(mkapp(nth(&whole, 1), seqc(vec![nth(&whole, 2), nth(&a, 1)])))
    }));

    // beyond the base: the enumerable boundary registrations pyarest carries
    register("DEFS", Rc::new(|_mu, _o| step_d().unwrap_or_else(bot))); // §14.3.3
    register("cellkey", Rc::new(|_mu, o| {
        let it = items(&list_of(&o));
        if it.len() != 2 { return bot(); }
        match (aval(&it[0]), aval(&it[1])) {
            (Some(a), Some(b)) => {
                let s = |l: &Leaf| match l {
                    Leaf::S(s) => Some(s.clone()),
                    Leaf::I(i) => Some(i.to_string()),
                    _ => None,
                };
                match (s(&a), s(&b)) {
                    (Some(x), Some(y)) => atom(Leaf::S(format!("{}:{}", x, y))),
                    _ => bot(),
                }
            }
            _ => bot(),
        }
    }));
    // the skolem boundary op (task-970 mapped to 0.9.0): an existential
    // head's fresh id as a PURE function of its frontier — 've_' +
    // fnv1a64_hex(values joined '|'). Determinism is the idempotence crux
    // (same frontier, same id: the owned sweep dedups re-derivations).
    // str/int atoms only; empty or non-sequence input answers ⊥.
    register("skolem", Rc::new(|_mu, o| {
        let it = items(&list_of(&o));
        if it.is_empty() {
            return bot();
        }
        let mut vals: Vec<String> = Vec::new();
        for x in &it {
            match aval(x) {
                Some(l) => match &*l {
                    Leaf::S(s) => vals.push(s.clone()),
                    Leaf::I(i) => vals.push(i.to_string()),
                    _ => return bot(),
                },
                None => return bot(),
            }
        }
        let mut h: u64 = 14695981039346656037;
        for b in vals.join("|").as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(1099511628211);
        }
        atom(Leaf::S(format!("ve_{:016x}", h)))
    }));
    // the prefix-strip base op — generic string algebra beside implode
    // and slug (spec D5): ⟨prefix, s⟩ answers s with a leading prefix
    // removed, or s unchanged. No policy — the CHOICE of what to strip
    // is canon's (system:sqlcol_base). Mirrors the four kernels.
    register("strip_prefix", Rc::new(|_mu, o| {
        let it = items(&list_of(&o));
        if it.len() != 2 {
            return bot();
        }
        let p = match aval(&it[0]).and_then(|l| leaf_str(&l)) {
            Some(s) => s,
            None => return bot(),
        };
        let s = match aval(&it[1]).and_then(|l| leaf_str(&l)) {
            Some(s) => s,
            None => return bot(),
        };
        let t = s.strip_prefix(&p).map(|t| t.to_string()).unwrap_or(s);
        atom(Leaf::S(t))
    }));
    // stage-1 field extraction at the lex boundary (spec D5; the
    // 2026-07-07 ruling). Text and sid must be string atoms exactly as
    // the python twin checks; vocabulary pairs and nouns stringify.
    register("stage1_fields", Rc::new(|_mu, o| {
        let it = items(&list_of(&o));
        if it.len() != 4 {
            return bot();
        }
        let strv = |x: &V| -> Option<String> {
            match aval(x) {
                Some(l) => match &*l { Leaf::S(s) => Some(s.clone()), _ => None },
                None => None,
            }
        };
        let text = match strv(&it[0]) { Some(s) => s, None => return bot() };
        let sid = match strv(&it[3]) { Some(s) => s, None => return bot() };
        let mut vocab: Vec<(String, String)> = Vec::new();
        for p in items(&list_of(&it[1])) {
            let pi = items(&list_of(&p));
            if pi.len() >= 2 {
                if let (Some(a), Some(b)) = (
                    aval(&pi[0]).and_then(|l| leaf_str(&l)),
                    aval(&pi[1]).and_then(|l| leaf_str(&l)),
                ) {
                    vocab.push((a, b));
                }
            }
        }
        let mut nouns: Vec<String> = Vec::new();
        for nx in items(&list_of(&it[2])) {
            match aval(&nx).and_then(|l| leaf_str(&l)) {
                Some(s) => nouns.push(s),
                None => return bot(),
            }
        }
        let rows = stage1_rows_of(&text, &vocab, &nouns, &sid);
        seqc(rows
            .into_iter()
            .map(|(ft, s, v)| {
                seqc(vec![
                    atom(Leaf::S(ft)),
                    seqc(vec![atom(Leaf::S(s)), atom(Leaf::S(v))]),
                ])
            })
            .collect())
    }));
    // the html escape transducer (the render's ONE boundary piece; the
    // doctrine correction 2026-07-08): & < > " to entities, ints
    // stringify, sequences bottom. Mirrors the Python/Java/C# twins.
    register("escape_html", Rc::new(|_mu, o| {
        match aval(&o) {
            Some(l) => {
                let s = match &*l {
                    Leaf::S(s) => s.clone(),
                    Leaf::I(i) => i.to_string(),
                    _ => return bot(),
                };
                let e = s.replace('&', "&amp;").replace('<', "&lt;")
                    .replace('>', "&gt;").replace('"', "&quot;");
                atom(Leaf::S(e))
            }
            None => bot(),
        }
    }));
    register("lex", Rc::new(|_mu, o| {
        let t = match aval(&o).and_then(|l| leaf_str(&l)) {
            Some(t) => t,
            None => return bot(),
        };
        let rows: Vec<V> = lex_rows(&t)
            .into_iter()
            .map(|r| {
                seqc(vec![
                    atom(Leaf::S(r.0)),
                    atom(Leaf::S(r.1)),
                    atom(Leaf::S(r.2)),
                    atom(Leaf::S(r.3)),
                    atom(Leaf::S(r.4)),
                    atom(Leaf::S(r.5)),
                    atom(Leaf::S(if r.6 { "T".into() } else { "F".into() })),
                    atom(Leaf::S(r.7)),
                    atom(Leaf::S(if r.8 { "T".into() } else { "F".into() })),
                    atom(Leaf::I(r.9)),
                ])
            })
            .collect();
        seqc(rows)
    }));
    register("implode", Rc::new(|_mu, o| {
        let it = items(&list_of(&o));
        if it.len() != 2 {
            return bot();
        }
        let sep = match aval(&it[0]).and_then(|l| leaf_str(&l)) {
            Some(s) => s,
            None => return bot(),
        };
        let mut parts: Vec<String> = Vec::new();
        for w in items(&list_of(&it[1])) {
            match aval(&w).and_then(|l| leaf_str(&l)) {
                Some(s) => parts.push(s),
                None => return bot(),
            }
        }
        atom(Leaf::S(parts.join(&sep)))
    }));
    // the JSON view emitter (react/Worker target): the element tree
    // itself, compact JSON. Mirrors python/java/C#.
    register("render:json", Rc::new(|_mu, o| {
        let mut out = String::new();
        if v_json(&o, &mut out) {
            atom(Leaf::S(out))
        } else {
            bot()
        }
    }));
    register("slug", Rc::new(|_mu, o| {
        match aval(&o).and_then(|l| leaf_str(&l)) {
            Some(t) => atom(Leaf::S(slug_str(&t))),
            None => bot(),
        }
    }));
}

// the tokenizer boundary (spec D5's slot, beside cellkey): ONE lexing
// transducer shared by the scott and native worlds — per-word lexical
// attributes only, no grammar knowledge (the vocabulary matching above it is
// canonical sequence algebra). Mirrors python/engine.py _lex_impl exactly.
fn leaf_str(l: &Leaf) -> Option<String> {
    match l {
        Leaf::S(s) => Some(s.clone()),
        Leaf::I(i) => Some(i.to_string()),
        _ => None,
    }
}

// a token's TEMPLATE form under NORMA hyphen binding (#24, lex field 8 —
// mirrors compiler.py _hyphen_tpl / engine.py _lex_impl): a one-sided
// touching hyphen is the bind marker and is consumed ('adj-'/'-adj' -> the
// word), the doubled hyphen escapes to one literal hyphen ('FORE--'->'FORE-',
// '--W'->'-W'), anything else (incl. the retired touching bind 'from-Status')
// is as written.
fn hyphen_tpl(tok: &str) -> String {
    let n = tok.chars().count();
    if n > 2 && tok.ends_with("--") {
        return tok[..tok.len() - 1].to_string();
    }
    if n > 2 && tok.starts_with("--") {
        return tok[1..].to_string();
    }
    if n > 1 && tok.ends_with('-') && !tok.ends_with("--") {
        return tok[..tok.len() - 1].to_string();
    }
    if n > 1 && tok.starts_with('-') && !tok.starts_with("--") {
        return tok[1..].to_string();
    }
    tok.to_string()
}

#[allow(clippy::type_complexity)]
fn lex_rows(text: &str) -> Vec<(String, String, String, String, String, String, bool, String, bool, i64)> {
    let cs: Vec<char> = text.chars().collect();
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut open: Option<usize> = None;
    for (i, c) in cs.iter().enumerate() {
        if *c == '\'' {
            match open {
                None => open = Some(i),
                Some(a) => {
                    spans.push((a, i + 1));
                    open = None;
                }
            }
        }
    }
    let mut rows = Vec::new();
    let mut i = 0usize;
    while i < cs.len() {
        if cs[i].is_whitespace() {
            i += 1;
            continue;
        }
        let s = i;
        while i < cs.len() && !cs[i].is_whitespace() {
            i += 1;
        }
        let e = i;
        let tok: String = cs[s..e].iter().collect();
        let k = spans
            .iter()
            .position(|&(a, b)| s < b && a < e)
            .map(|p| p + 1)
            .unwrap_or(0);
        let qtext: String = if k > 0 {
            let (a, b) = spans[k - 1];
            let (lo, hi) = (s.max(a + 1), e.min(b - 1));
            if lo < hi { cs[lo..hi].iter().collect() } else { String::new() }
        } else {
            String::new()
        };
        let nopunct: String = tok.trim_matches(|c: char| ".;:,".contains(c)).to_string();
        let base: String = nopunct.trim_end_matches(|c: char| c.is_ascii_digit()).to_string();
        let subscript = nopunct[base.len()..].to_string();
        let lower = tok.to_lowercase();
        let title = base.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
        let tpl = hyphen_tpl(&tok);
        rows.push((tok, nopunct, base, subscript, lower, qtext, title, tpl, k > 0, k as i64));
    }
    rows
}

// stage-1 field extraction (the bootstrap kernel's statement reader),
// one implementation for both evaluator paths -- the D5 transducer beside
// lex (the 2026-07-07 ruling: a performant implementation proven to the
// interface; a canonical composition is not owed at the boundary).
// Mirrors python engine.stage1_fields: quoted spans blank to spaces
// length-preserving; vocabulary literals hit case-insensitively with no
// letter adjacent, longest first (stable); a Trailing Marker must trail;
// nouns hit case-sensitively; the FIRST quoted content is the Literal
// Role; the first structural mark outside literals is the prose tell.
fn s1_blank_quotes(cs: &[char]) -> Vec<char> {
    let mut out = cs.to_vec();
    let mut open: Option<usize> = None;
    for (i, c) in cs.iter().enumerate() {
        if *c == '\'' {
            match open {
                None => open = Some(i),
                Some(a) => {
                    for o in out.iter_mut().take(i + 1).skip(a) {
                        *o = ' ';
                    }
                    open = None;
                }
            }
        }
    }
    out
}

fn s1_word_hit(hay: &[char], needle: &str, ci: bool) -> bool {
    let n: Vec<char> = needle.chars().collect();
    if n.is_empty() || hay.len() < n.len() {
        return false;
    }
    let eqc = |a: char, b: char| {
        if ci { a.to_ascii_lowercase() == b.to_ascii_lowercase() } else { a == b }
    };
    let letter = |c: char| c.is_ascii_alphabetic();
    'outer: for s in 0..=(hay.len() - n.len()) {
        for k in 0..n.len() {
            if !eqc(hay[s + k], n[k]) {
                continue 'outer;
            }
        }
        let before_ok = s == 0 || !letter(hay[s - 1]);
        let e = s + n.len();
        let after_ok = e >= hay.len() || !letter(hay[e]);
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

fn stage1_rows_of(text: &str, vocab: &[(String, String)], nouns: &[String],
                  sid: &str) -> Vec<(String, String, String)> {
    let trimmed: &str = text.trim();
    let trimmed: &str = trimmed.trim_end_matches('.');
    let tcs: Vec<char> = trimmed.chars().collect();
    let bare = s1_blank_quotes(&tcs);
    let bare_s: String = bare.iter().collect();
    let mut out: Vec<(String, String, String)> = Vec::new();
    let mut order: Vec<usize> = (0..vocab.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(vocab[i].1.chars().count()));
    for &i in &order {
        let (ftb, lit) = &vocab[i];
        if !s1_word_hit(&bare, lit, true) {
            continue;
        }
        if ftb == "Statement_has_Trailing_Marker"
            && !bare_s.trim_end().to_ascii_lowercase()
                .ends_with(&lit.to_ascii_lowercase())
        {
            continue;
        }
        out.push((ftb.clone(), sid.to_string(), lit.clone()));
    }
    for nn in nouns {
        if s1_word_hit(&bare, nn, false) {
            out.push(("Statement_has_Role_Reference".into(),
                      sid.to_string(), nn.clone()));
        }
    }
    let mut open: Option<usize> = None;
    for (i, c) in tcs.iter().enumerate() {
        if *c == '\'' {
            match open {
                None => open = Some(i),
                Some(a) => {
                    let q: String = tcs[a + 1..i].iter().collect();
                    out.push(("Statement_has_Literal_Role".into(),
                              sid.to_string(), q));
                    break;
                }
            }
        }
    }
    for mark in [",", "(", ")", ": "] {
        if bare_s.contains(mark) {
            out.push(("Statement_has_Prose_Punctuation".into(),
                      sid.to_string(), mark.to_string()));
            break;
        }
    }
    out
}

// system:sqlname's composition, natively: slug then lowercase (the
// single-token lex row's lower field over slug output is exactly
// ASCII lowercase), empty falling back to "t". Serves the
// system:entity_view prim's column naming.
fn sql_name(s: &str) -> String {
    let t = slug_str(s).to_ascii_lowercase();
    if t.is_empty() { "t".into() } else { t }
}

// the JSON spelling of a value (render:json, the react/Worker view
// target): atoms to scalars, seqs to arrays — python json.dumps
// compact, ensure_ascii=False: only quote, backslash, and C0 controls
// escape. A pure format transducer (the implode class).
fn json_escape_into(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn v_json(v: &V, out: &mut String) -> bool {
    match aval(v) {
        Some(l) => {
            match &*l {
                Leaf::S(s) => json_escape_into(s, out),
                Leaf::I(i) => out.push_str(&i.to_string()),
                Leaf::F(f) => out.push_str(&f.to_string()),
                _ => return false,
            }
            true
        }
        None => {
            let it = items(&list_of(v));
            if isbot(v) {
                return false;
            }
            out.push('[');
            for (i, e) in it.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                if !v_json(e, out) {
                    return false;
                }
            }
            out.push(']');
            true
        }
    }
}

fn slug_str(t: &str) -> String {
    let mut out = String::new();
    let mut run = false;
    for c in t.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            run = false;
        } else if !run {
            out.push('_');
            run = true;
        }
    }
    out.trim_matches('_').to_string()
}

fn fastreg(name: &str, p: Prim) {
    FAST.with(|r| {
        r.borrow_mut().insert(name.to_string(), p);
    });
}

fn register_overrides() {
    // Rust's override set: native-container twins of the canonical combinator terms,
    // held observationally equal by the differential (overrides on ≡ off ≡ Python)
    fastreg("apndl", Rc::new(|_mu, o| {
        if !(pair_b(&o) && is_seq(&nth(&o, 1))) { return bot(); }
        seq(cons(nth(&o, 0), list_of(&nth(&o, 1))))
    }));
    fastreg("apndr", Rc::new(|_mu, o| {
        if !(pair_b(&o) && is_seq(&nth(&o, 0))) { return bot(); }
        let mut xs = items(&list_of(&nth(&o, 0)));
        xs.push(nth(&o, 1));
        seq(from_vec(xs))
    }));
    fastreg("distl", Rc::new(|_mu, o| {
        if !(pair_b(&o) && is_seq(&nth(&o, 1))) { return bot(); }
        let x = nth(&o, 0);
        seq(from_vec(items(&list_of(&nth(&o, 1))).into_iter()
            .map(|yv| seq(cons(x.clone(), cons(yv, nil())))).collect()))
    }));
    fastreg("distr", Rc::new(|_mu, o| {
        if !(pair_b(&o) && is_seq(&nth(&o, 0))) { return bot(); }
        let yv = nth(&o, 1);
        seq(from_vec(items(&list_of(&nth(&o, 0))).into_iter()
            .map(|x| seq(cons(x, cons(yv.clone(), nil())))).collect()))
    }));
    fastreg("reverse", Rc::new(|_mu, o| match shape(&o) {
        Shape::Seq(l) => { let mut xs = items(&l); xs.reverse(); seq(from_vec(xs)) }
        _ => bot(),
    }));
    fastreg("cat", Rc::new(|_mu, o| {
        if !(pair_b(&o) && is_seq(&nth(&o, 0)) && is_seq(&nth(&o, 1))) { return bot(); }
        let mut xs = items(&list_of(&nth(&o, 0)));
        xs.extend(items(&list_of(&nth(&o, 1))));
        seq(from_vec(xs))
    }));
    fastreg("1r", Rc::new(|_mu, o| match shape(&o) {
        Shape::Seq(l) => items(&l).last().cloned().unwrap_or_else(bot),
        _ => bot(),
    }));
    fastreg("tlr", Rc::new(|_mu, o| match shape(&o) {
        Shape::Seq(l) => {
            let mut xs = items(&l);
            if xs.is_empty() { return bot(); }
            xs.pop();
            seq(from_vec(xs))
        }
        _ => bot(),
    }));
    fastreg("rotl", Rc::new(|_mu, o| match shape(&o) {
        Shape::Seq(l) => {
            let mut xs = items(&l);
            if xs.is_empty() { return phi(); }
            let h = xs.remove(0);
            xs.push(h);
            seq(from_vec(xs))
        }
        _ => bot(),
    }));
    fastreg("rotr", Rc::new(|_mu, o| match shape(&o) {
        Shape::Seq(l) => {
            let mut xs = items(&l);
            if xs.is_empty() { return phi(); }
            let last = xs.pop().unwrap();
            xs.insert(0, last);
            seq(from_vec(xs))
        }
        _ => bot(),
    }));
    fastreg("trans", Rc::new(|_mu, o| match shape(&o) {
        Shape::Seq(l) => {
            let rows = items(&l);
            if rows.iter().any(|r| !is_seq(r)) { return bot(); }
            if rows.is_empty() { return phi(); }
            let mat: Vec<Vec<V>> = rows.iter().map(|r| items(&list_of(r))).collect();
            if mat.iter().all(|r| r.is_empty()) { return phi(); }
            if mat.iter().any(|r| r.is_empty()) { return bot(); }
            let w = mat[0].len();
            if mat.iter().any(|r| r.len() != w) { return bot(); }
            seq(from_vec((0..w).map(|j|
                seq(from_vec(mat.iter().map(|r| r[j].clone()).collect()))).collect()))
        }
        _ => bot(),
    }));
}


// ============================ the native carrier ==============================
// delta.py's analog, the deepest override behind the same universal interface: a
// scalar IS an atom, a Vec IS a sequence, Bot is bottom, and an application node is a
// 3-sequence headed by the shared AppTag sentinel (so App nodes survive conversion).
// The evaluator is the same mu with the same metacomposition; WHILE and INSERT are
// iterative exactly as delta.py runs them. Certified: engine=native equals
// engine=scott equals Python on the differential.
#[derive(Clone)]
enum N {
    A(Rc<Leaf>),
    S(Rc<Vec<N>>),
    Bot,
}

fn napp(f: N, x: N) -> N {
    N::S(Rc::new(vec![N::A(Rc::new(Leaf::AppTag)), f, x]))
}
fn nseq(xs: Vec<N>) -> N {
    if xs.iter().any(|x| matches!(x, N::Bot)) { N::Bot } else { N::S(Rc::new(xs)) }
}
fn n_at() -> N { N::A(Rc::new(Leaf::S("T".into()))) }
fn n_af() -> N { N::A(Rc::new(Leaf::S("F".into()))) }
fn nb(b: bool) -> N { if b { n_at() } else { n_af() } }
fn n_is_t(x: &N) -> bool { matches!(x, N::A(l) if matches!(&**l, Leaf::S(s) if s == "T")) }
fn n_is_f(x: &N) -> bool { matches!(x, N::A(l) if matches!(&**l, Leaf::S(s) if s == "F")) }
fn n_eq(a: &N, b: &N) -> bool {
    match (a, b) {
        (N::A(x), N::A(y)) => x.nateq(y),
        (N::S(x), N::S(y)) => x.len() == y.len() && x.iter().zip(y.iter()).all(|(p, q)| n_eq(p, q)),
        _ => false,
    }
}

struct NEval {
    cells: Vec<(Leaf, N)>,            // the step's DEFS-in-D, first match wins
    process: Vec<(String, N)>,        // compiled process defs (converted once)
    defs_n: N,                        // the retained store as N (the DEFS accessor)
    fuel: std::cell::Cell<i64>,       // <0 = unbounded
}

impl NEval {
    fn mu(&self, e: N) -> N {
        // an application node reduces; a value is its own meaning
        let (f0, x0) = match &e {
            N::S(v) if v.len() == 3 && matches!(&v[0], N::A(l) if matches!(&**l, Leaf::AppTag)) => {
                (v[1].clone(), v[2].clone())
            }
            _ => return e,
        };
        let fuel = self.fuel.get();
        if fuel >= 0 {
            self.fuel.set(fuel - 1);
            if fuel - 1 <= 0 { return N::Bot; }
        }
        let f = self.mu(f0);
        let x = self.mu(x0);
        if matches!(f, N::Bot) || matches!(x, N::Bot) { return N::Bot; }
        match &f {
            N::S(v) => {
                if v.is_empty() { return N::Bot; }
                let pair = nseq(vec![f.clone(), x]);
                self.mu(napp(v[0].clone(), pair))            // metacomposition on the head
            }
            N::A(l) => {
                let lc: &Leaf = l;
                if let Some((_, sd)) = self.cells.iter().find(|(k, _)| k.nateq(lc)) {
                    return self.mu(napp(sd.clone(), x));     // the step's DEFS cell first
                }
                if let Some(r) = self.prim(lc, &x) {
                    // the coverage tracer's second stage: a KNOWN prim
                    // answering ⊥ names the semantic gap (shape mismatch
                    // inside the op), which a name-miss trace cannot see
                    if matches!(r, N::Bot)
                        && std::env::var_os("AREST_NEVAL_TRACE").is_some()
                    {
                        let shape = match &x {
                            N::A(_) => "atom".to_string(),
                            N::S(v) => format!("seq({})", v.len()),
                            N::Bot => "bot".to_string(),
                        };
                        eprintln!("neval-bot: prim {:?} over {}", lc, shape);
                    }
                    return self.mu(r);                       // the native base
                }
                if let Some(k) = leaf_key(lc) {
                    if let Some((_, obj)) = self.process.iter().rev().find(|(n, _)| *n == k) {
                        return self.mu(napp(obj.clone(), x));
                    }
                    // the bisection tracer (third stage): under
                    // AREST_NEVAL_TRACE every CANON-def evaluation prints
                    // name -> answer shape, so one traced run walks the
                    // whole tree and the first unexpectedly-empty answer
                    // names the collapse point (the silent-semantic gap the
                    // miss/bot tracers cannot see).
                    if std::env::var_os("AREST_NEVAL_TRACE").is_some() {
                        if let Some(obj) = NCANON.with(|c| {
                            c.borrow().iter().rev().find(|(n, _)| *n == k)
                                .map(|(_, o)| o.clone())
                        }) {
                            let r = self.mu(napp(obj, x));
                            let shape = match &r {
                                N::A(_) => "atom".to_string(),
                                N::S(v) => format!("seq({})", v.len()),
                                N::Bot => "BOT".to_string(),
                            };
                            eprintln!("neval-def: {} -> {}", k, shape);
                            return r;
                        }
                    }
                    // the intersection-source fallback: the native mirror of the
                    // Scott mu's CANON step (make_mu resolves cells, then the
                    // base, then process, then canon). A rule body that reaches a
                    // theta/ast/system/constraints def a partial process list
                    // does not carry still resolves, exactly as Scott resolves
                    // it, so a hand-built store without a process field derives
                    // the same rows as a fully compiled one. When the process
                    // list already carries the canon def (the compiled apps and
                    // the differential), that match wins above and this branch is
                    // never reached, so the certified machine step is unchanged.
                    if let Some(obj) = NCANON.with(|c| {
                        c.borrow().iter().rev().find(|(n, _)| *n == k).map(|(_, o)| o.clone())
                    }) {
                        return self.mu(napp(obj, x));
                    }
                    // the coverage tracer (the synthesize-plumb diagnosis,
                    // ledger 2026-07-08): a name neither prim nor process
                    // nor canon is the carrier's gap — name it on stderr
                    // when tracing, so the missing-op list enumerates
                    // itself instead of collapsing silently to ⊥.
                    if std::env::var_os("AREST_NEVAL_TRACE").is_some() {
                        eprintln!("neval-miss: {}", k);
                    }
                }
                N::Bot
            }
            N::Bot => N::Bot,
        }
    }

    fn prim(&self, name: &Leaf, x: &N) -> Option<N> {
        if let Leaf::I(i) = name {
            let i = *i;
            if (1..=32).contains(&i) {
                return Some(match x {
                    N::S(v) if v.len() >= i as usize => v[(i - 1) as usize].clone(),
                    _ => N::Bot,
                });
            }
            return None;
        }
        let s = match name { Leaf::S(s) => s.as_str(), _ => return None };
        let seqv = |x: &N| -> Option<Rc<Vec<N>>> {
            if let N::S(v) = x { Some(v.clone()) } else { None }
        };
        let pairv = |x: &N| -> Option<(N, N)> {
            seqv(x).and_then(|v| if v.len() == 2 { Some((v[0].clone(), v[1].clone())) } else { None })
        };
        let numv = |n: &N| -> Option<f64> {
            if let N::A(l) = n { num(l) } else { None }
        };
        Some(match s {
            "id" => x.clone(),
            "tl" => match seqv(x) { Some(v) if !v.is_empty() => N::S(Rc::new(v[1..].to_vec())), _ => N::Bot },
            "atom" => match x { N::Bot => N::Bot, N::A(_) => n_at(), N::S(v) => nb(v.is_empty()) },
            "null" => match x { N::Bot => N::Bot, N::A(_) => n_af(), N::S(v) => nb(v.is_empty()) },
            "eq" => match pairv(x) { Some((a, b)) => nb(n_eq(&a, &b)), None => N::Bot },
            "apndl" => match pairv(x) {
                Some((h, N::S(t))) => { let mut v = vec![h]; v.extend(t.iter().cloned()); N::S(Rc::new(v)) }
                _ => N::Bot,
            },
            "apndr" => match pairv(x) {
                Some((N::S(t), e)) => { let mut v = t.to_vec(); v.push(e); N::S(Rc::new(v)) }
                _ => N::Bot,
            },
            "distl" => match pairv(x) {
                Some((a, N::S(ys))) => N::S(Rc::new(ys.iter().map(|y| N::S(Rc::new(vec![a.clone(), y.clone()]))).collect())),
                _ => N::Bot,
            },
            "distr" => match pairv(x) {
                Some((N::S(xs), b)) => N::S(Rc::new(xs.iter().map(|a| N::S(Rc::new(vec![a.clone(), b.clone()]))).collect())),
                _ => N::Bot,
            },
            "length" => match seqv(x) { Some(v) => N::A(Rc::new(Leaf::I(v.len() as i64))), None => N::Bot },
            "reverse" => match seqv(x) { Some(v) => { let mut w = v.to_vec(); w.reverse(); N::S(Rc::new(w)) } None => N::Bot },
            "cat" => match pairv(x) {
                Some((N::S(a), N::S(b))) => { let mut v = a.to_vec(); v.extend(b.iter().cloned()); N::S(Rc::new(v)) }
                _ => N::Bot,
            },
            "not" => if n_is_t(x) { n_af() } else if n_is_f(x) { n_at() } else { N::Bot },
            "and" | "or" => match pairv(x) {
                Some((a, b)) if (n_is_t(&a) || n_is_f(&a)) && (n_is_t(&b) || n_is_f(&b)) =>
                    nb(if s == "and" { n_is_t(&a) && n_is_t(&b) } else { n_is_t(&a) || n_is_t(&b) }),
                _ => N::Bot,
            },
            "1r" => match seqv(x) { Some(v) if !v.is_empty() => v[v.len() - 1].clone(), _ => N::Bot },
            "tlr" => match seqv(x) { Some(v) if !v.is_empty() => N::S(Rc::new(v[..v.len() - 1].to_vec())), _ => N::Bot },
            "rotl" => match seqv(x) {
                Some(v) => if v.is_empty() { N::S(Rc::new(vec![])) } else {
                    let mut w = v[1..].to_vec(); w.push(v[0].clone()); N::S(Rc::new(w)) },
                None => N::Bot,
            },
            "rotr" => match seqv(x) {
                Some(v) => if v.is_empty() { N::S(Rc::new(vec![])) } else {
                    let mut w = vec![v[v.len() - 1].clone()]; w.extend(v[..v.len() - 1].iter().cloned()); N::S(Rc::new(w)) },
                None => N::Bot,
            },
            "trans" => match seqv(x) {
                Some(rows) => {
                    if rows.iter().any(|r| !matches!(r, N::S(_))) { return Some(N::Bot); }
                    if rows.is_empty() { return Some(N::S(Rc::new(vec![]))); }
                    let mat: Vec<Rc<Vec<N>>> = rows.iter().map(|r| seqv(r).unwrap()).collect();
                    let w = mat[0].len();
                    if mat.iter().any(|r| r.len() != w) { return Some(N::Bot); }
                    if w == 0 { return Some(N::S(Rc::new(vec![]))); }
                    N::S(Rc::new((0..w).map(|j| N::S(Rc::new(mat.iter().map(|r| r[j].clone()).collect()))).collect()))
                }
                None => N::Bot,
            },
            "+" | "-" | "*" => match pairv(x) {
                // arithmetic-local coercion of lexical atoms (cint/cnum),
                // int first — mirrors the Python paths' _tonum exactly
                Some((a, b)) => {
                    let ci = |n: &N| if let N::A(l) = n { cint(l) } else { None };
                    let cn = |n: &N| if let N::A(l) = n { cnum(l) } else { None };
                    match (ci(&a), ci(&b)) {
                        (Some(p), Some(q)) => N::A(Rc::new(Leaf::I(match s { "+" => p + q, "-" => p - q, _ => p * q }))),
                        _ => match (cn(&a), cn(&b)) {
                            (Some(p), Some(q)) => N::A(Rc::new(Leaf::F(match s { "+" => p + q, "-" => p - q, _ => p * q }))),
                            _ => N::Bot,
                        },
                    }
                }
                None => N::Bot,
            },
            "div" => match pairv(x) {
                Some((a, b)) => match (numv(&a), numv(&b)) {
                    (Some(p), Some(q)) if q != 0.0 => N::A(Rc::new(Leaf::F(p / q))),
                    _ => N::Bot,
                },
                None => N::Bot,
            },
            "ge" | "gt" | "le" | "lt" => match pairv(x) {
                Some((N::A(a), N::A(b))) => match (cnum(&a), cnum(&b)) {
                    (Some(p), Some(q)) => nb(match s { "ge" => p >= q, "gt" => p > q, "le" => p <= q, _ => p < q }),
                    _ => match (&*a, &*b) {
                        (Leaf::S(p), Leaf::S(q)) => nb(match s { "ge" => p >= q, "gt" => p > q, "le" => p <= q, _ => p < q }),
                        _ => N::Bot,
                    },
                },
                _ => N::Bot,
            },
            "apply" => match pairv(x) { Some((f, y)) => self.mu(napp(f, y)), None => N::Bot },
            "COMP" => match pairv(x) {
                Some((N::S(whole), y)) => whole[1..].iter().rev().fold(y, |acc, f| napp(f.clone(), acc)),
                _ => N::Bot,
            },
            "CONS" => match pairv(x) {
                Some((N::S(whole), y)) => nseq(whole[1..].iter().map(|f| self.mu(napp(f.clone(), y.clone()))).collect()),
                _ => N::Bot,
            },
            "CONST" => match pairv(x) {
                Some((N::S(whole), y)) => {
                    if matches!(y, N::Bot) { N::Bot }
                    else if whole.len() >= 2 { whole[1].clone() } else { N::Bot }
                }
                _ => N::Bot,
            },
            "ALPHA" => match pairv(x) {
                Some((N::S(whole), N::S(ys))) if whole.len() >= 2 =>
                    nseq(ys.iter().map(|yi| self.mu(napp(whole[1].clone(), yi.clone()))).collect()),
                Some((N::S(_), _)) => N::Bot,
                _ => N::Bot,
            },
            "COND" => match pairv(x) {
                Some((N::S(whole), y)) if whole.len() >= 4 => {
                    let pv = self.mu(napp(whole[1].clone(), y.clone()));
                    if n_is_t(&pv) { self.mu(napp(whole[2].clone(), y)) }
                    else if n_is_f(&pv) { self.mu(napp(whole[3].clone(), y)) }
                    else { N::Bot }
                }
                _ => N::Bot,
            },
            "INSERT" => match pairv(x) {
                Some((N::S(whole), N::S(ys))) if whole.len() >= 2 && !ys.is_empty() => {
                    let mut acc = ys[ys.len() - 1].clone();
                    for xi in ys[..ys.len() - 1].iter().rev() {
                        acc = self.mu(napp(whole[1].clone(), nseq(vec![xi.clone(), acc])));
                        if matches!(acc, N::Bot) { break; }
                    }
                    acc
                }
                _ => N::Bot,
            },
            "WHILE" => match pairv(x) {
                Some((N::S(whole), y0)) if whole.len() >= 3 => {
                    let mut y = y0;
                    loop {
                        let pv = self.mu(napp(whole[1].clone(), y.clone()));
                        if n_is_f(&pv) { break y; }
                        if !n_is_t(&pv) { break N::Bot; }
                        y = self.mu(napp(whole[2].clone(), y));
                    }
                }
                _ => N::Bot,
            },
            "BU" => match pairv(x) {
                Some((N::S(whole), y)) if whole.len() >= 3 =>
                    self.mu(napp(whole[1].clone(), nseq(vec![whole[2].clone(), y]))),
                _ => N::Bot,
            },
            "DEFS" => self.defs_n.clone(),
            "cellkey" => match pairv(x) {
                Some((N::A(a), N::A(b))) => {
                    let sv = |l: &Leaf| match l {
                        Leaf::S(t) => Some(t.clone()),
                        Leaf::I(i) => Some(i.to_string()),
                        _ => None,
                    };
                    match (sv(&a), sv(&b)) {
                        (Some(p), Some(q)) => N::A(Rc::new(Leaf::S(format!("{}:{}", p, q)))),
                        _ => N::Bot,
                    }
                }
                _ => N::Bot,
            },
            // CERTIFIED-EQUAL OVERRIDE of DEF("system:vb_fetch")
            // (shared/system.canon) — the resident's first canon-NAMED
            // native arm; the meaning stays in canon, this arm exists for
            // SPEED only and is twinned by the parity pin (the canonical
            // absorbed reassembly evaluates one interpretive ast:DynFetch
            // per entity id: measured 301 s per fact type over the tasks
            // store, 2026-07-08; this arm is one spine pass). Prim wins
            // over the process/canon def by NEval's resolution order —
            // cells still shadow it first, exactly as they shadow defs.
            // CERTIFIED-EQUAL OVERRIDE of DEF("system:entity_view") — the
            // whole 3NF per-entity view in one spine pass (the vb_fetch
            // treatment: the canon def is the meaning, this arm exists
            // because the interpretive evaluation is minutes at tasks
            // scale). Answers the canon shape ⟨exists, fields, facts⟩
            // with the canon encodings (unary T/F, absent "#"); kinds
            // ride system:ev_cols for the render. Twinned by the
            // entity-view parity pin (tests/derive.rs).
            // CERTIFIED-EQUAL OVERRIDE of DEF("system:ev_cols") — the
            // classified column layout, one spine pass (the WHILE-fold is
            // interpretive-minutes at fleet scale). Twinned beside
            // entity_view's pin.
            "system:ev_cols" => match pairv(x) {
                Some((N::A(nl), d)) => {
                    let noun = match leaf_str(&nl) {
                        Some(s) => s,
                        None => return Some(N::Bot),
                    };
                    let spine: Vec<(String, N)> = match &d {
                        N::S(cells) => cells
                            .iter()
                            .filter_map(|c| {
                                if let N::S(it) = c {
                                    if it.len() == 3 {
                                        if let (N::A(l0), N::A(k)) = (&it[0], &it[1]) {
                                            if matches!(&**l0, Leaf::S(s) if s == "CELL") {
                                                return leaf_str(k).map(|key| (key, it[2].clone()));
                                            }
                                        }
                                    }
                                }
                                None
                            })
                            .collect(),
                        _ => return Some(N::Bot),
                    };
                    let hash = N::A(Rc::new(Leaf::S("#".into())));
                    N::S(Rc::new(
                        ev_cols_native(&spine, &noun)
                            .into_iter()
                            .map(|(ft, kind, other, col)| {
                                N::S(Rc::new(vec![
                                    N::A(Rc::new(Leaf::S(ft))),
                                    N::A(Rc::new(Leaf::S(kind))),
                                    match other {
                                        Some(o) => N::A(Rc::new(Leaf::S(o))),
                                        None => hash.clone(),
                                    },
                                    N::A(Rc::new(Leaf::S(col))),
                                ]))
                            })
                            .collect(),
                    ))
                }
                _ => N::Bot,
            },
            "system:entity_view" => match x {
                N::S(v3) if v3.len() == 3 => {
                    let noun = match &v3[0] {
                        N::A(l) => match leaf_str(l) { Some(s) => s, None => return Some(N::Bot) },
                        _ => return Some(N::Bot),
                    };
                    let id = match &v3[1] {
                        N::A(l) => match leaf_str(l) { Some(s) => s, None => return Some(N::Bot) },
                        _ => return Some(N::Bot),
                    };
                    let spine: Vec<(String, N)> = match &v3[2] {
                        N::S(cells) => cells
                            .iter()
                            .filter_map(|c| {
                                if let N::S(it) = c {
                                    if it.len() == 3 {
                                        if let (N::A(l0), N::A(k)) = (&it[0], &it[1]) {
                                            if matches!(&**l0, Leaf::S(s) if s == "CELL") {
                                                return leaf_str(k).map(|key| (key, it[2].clone()));
                                            }
                                        }
                                    }
                                }
                                None
                            })
                            .collect(),
                        _ => return Some(N::Bot),
                    };
                    let fetch = |name: &str| -> Option<&N> {
                        spine.iter().find(|(k, _)| k == name).map(|(_, v)| v)
                    };
                    let rows_of = |name: &str| -> Vec<N> {
                        match fetch(name) {
                            Some(N::S(v)) => v.to_vec(),
                            _ => Vec::new(),
                        }
                    };
                    let sv2 = |n: &N| -> Option<String> {
                        match n { N::A(l) => leaf_str(l), _ => None }
                    };
                    let hash = N::A(Rc::new(Leaf::S("#".into())));
                    let t_at = N::A(Rc::new(Leaf::S("T".into())));
                    let f_at = N::A(Rc::new(Leaf::S("F".into())));
                    let mut fields: Vec<N> = Vec::new();
                    let mut any_seen = false;
                    let classified = ev_cols_native(&spine, &noun);
                    for (ft, kind, other, col) in &classified {
                        let key = if kind == "unary" {
                            col.clone()
                        } else {
                            other.clone().unwrap_or_else(|| col.clone())
                        };
                        let pop = rows_of(ft);
                        let val: N = if kind.as_str() == "unary" {
                            let hit = pop.iter().any(|r| match r {
                                N::S(cc) if !cc.is_empty() =>
                                    sv2(&cc[0]).as_deref() == Some(id.as_str()),
                                _ => false,
                            });
                            if hit { any_seen = true; t_at.clone() } else { f_at.clone() }
                        } else {
                            let mut last: Option<N> = None;
                            for r in pop.iter() {
                                if let N::S(cc) = r {
                                    if cc.len() >= 2
                                        && sv2(&cc[0]).as_deref() == Some(id.as_str())
                                    {
                                        last = Some(cc[1].clone());
                                    }
                                }
                            }
                            match last {
                                Some(v) => { any_seen = true; v }
                                None => hash.clone(),
                            }
                        };
                        fields.push(N::S(Rc::new(vec![
                            N::A(Rc::new(Leaf::S(key))),
                            val,
                        ])));
                    }
                    // own facts: factType order, minus absorbed, the noun's
                    // role positions, any position matching the id
                    let all_absorbed: Vec<String> = rows_of("rmapColumns")
                        .iter()
                        .filter_map(|r| {
                            if let N::S(cc) = r {
                                if cc.len() >= 3 { return sv2(&cc[2]); }
                            }
                            None
                        })
                        .collect();
                    let mut facts: Vec<N> = Vec::new();
                    for fr in rows_of("factType") {
                        let ft = match &fr {
                            N::S(cc) if !cc.is_empty() => match sv2(&cc[0]) {
                                Some(f) => f,
                                None => continue,
                            },
                            _ => continue,
                        };
                        if all_absorbed.iter().any(|a| *a == ft) {
                            continue;
                        }
                        let positions: Vec<i64> = rows_of("role")
                            .iter()
                            .filter_map(|r| {
                                if let N::S(cc) = r {
                                    if cc.len() >= 4
                                        && sv2(&cc[1]).as_deref() == Some(ft.as_str())
                                        && sv2(&cc[3]).as_deref() == Some(noun.as_str())
                                    {
                                        if let N::A(pl) = &cc[2] {
                                            if let Leaf::I(pp) = &**pl {
                                                return Some(*pp);
                                            }
                                        }
                                    }
                                }
                                None
                            })
                            .collect();
                        if positions.is_empty() {
                            continue;
                        }
                        for r in rows_of(&ft) {
                            if let N::S(cc) = &r {
                                let hit = positions.iter().any(|p| {
                                    let idx = (*p as usize).saturating_sub(1);
                                    cc.len() > idx
                                        && sv2(&cc[idx]).as_deref() == Some(id.as_str())
                                });
                                if hit {
                                    any_seen = true;
                                    facts.push(N::S(Rc::new(vec![
                                        N::A(Rc::new(Leaf::S(ft.clone()))),
                                        r.clone(),
                                    ])));
                                }
                            }
                        }
                    }
                    let spine_hit = rows_of(&noun).iter().any(|r| match r {
                        N::S(cc) if !cc.is_empty() =>
                            sv2(&cc[0]).as_deref() == Some(id.as_str()),
                        _ => false,
                    });
                    let exists = any_seen || spine_hit;
                    Some(N::S(Rc::new(vec![
                        if exists { t_at } else { f_at },
                        N::S(Rc::new(fields)),
                        N::S(Rc::new(facts)),
                    ]))).map(|r| r)
                    .unwrap_or(N::Bot)
                    .into()
                }
                _ => N::Bot,
            },
            "system:vb_fetch" => match pairv(x) {
                Some((ft, d)) => {
                    let spine: Vec<(String, N)> = match &d {
                        N::S(cells) => cells
                            .iter()
                            .filter_map(|c| {
                                if let N::S(it) = c {
                                    if it.len() == 3 {
                                        if let (N::A(l0), N::A(k)) = (&it[0], &it[1]) {
                                            if matches!(&**l0, Leaf::S(s) if s == "CELL") {
                                                let key = match &**k {
                                                    Leaf::S(s) => s.clone(),
                                                    Leaf::I(i) => i.to_string(),
                                                    _ => return None,
                                                };
                                                return Some((key, it[2].clone()));
                                            }
                                        }
                                    }
                                }
                                None
                            })
                            .collect(),
                        _ => return Some(N::Bot),
                    };
                    let hash = N::A(Rc::new(Leaf::S("#".into())));
                    // ast:Fetch — the FIRST cell of that name (n_cells_of's
                    // own precedence); ast:FetchPop — missing, or a value
                    // eq to the "#" sentinel, answers the empty population
                    let fetch = |name: &str| -> Option<N> {
                        spine.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
                    };
                    let pop = |name: &str| -> N {
                        match fetch(name) {
                            Some(v) if !n_eq(&v, &hash) => v,
                            _ => N::S(Rc::new(vec![])),
                        }
                    };
                    // system:vb_colrow — rmapColumns rows ⟨noun, col, ft⟩
                    // filtered on this ft; a malformed row bottoms exactly
                    // like the canonical selector-through-Filter would
                    let rmap = pop("rmapColumns");
                    let rrows = match &rmap {
                        N::S(v) => v.clone(),
                        _ => return Some(N::Bot),
                    };
                    let mut colrow: Option<(N, N)> = None;
                    for r in rrows.iter() {
                        match r {
                            N::S(cols) if cols.len() >= 3 => {
                                if n_eq(&cols[2], &ft) && colrow.is_none() {
                                    colrow = Some((cols[0].clone(), cols[1].clone()));
                                }
                            }
                            _ => return Some(N::Bot),
                        }
                    }
                    let (noun, col) = match colrow {
                        None => {
                            // own-table: FetchPop(ft) : D — a non-atom ft
                            // names no cell, so its population is empty
                            let name = match &ft {
                                N::A(l) => match &**l {
                                    Leaf::S(s) => s.clone(),
                                    Leaf::I(i) => i.to_string(),
                                    _ => return Some(N::Bot),
                                },
                                _ => return Some(N::S(Rc::new(vec![]))),
                            };
                            return Some(pop(&name));
                        }
                        Some(nc) => nc,
                    };
                    let noun_s = match &noun {
                        N::A(l) => match &**l {
                            Leaf::S(s) => s.clone(),
                            Leaf::I(i) => i.to_string(),
                            _ => return Some(N::Bot),
                        },
                        _ => return Some(N::Bot),
                    };
                    // the composed column selector is a prim selector in
                    // the canonical pipeline: integer, 1..=32, else ⊥
                    let col_i = match &col {
                        N::A(l) => match &**l {
                            Leaf::I(i) if (1..=32).contains(i) => *i as usize,
                            _ => return Some(N::Bot),
                        },
                        _ => return Some(N::Bot),
                    };
                    let table = pop(&noun_s);
                    let trows = match &table {
                        N::S(v) => v.clone(),
                        _ => return Some(N::Bot),
                    };
                    // per spine id: ast:DynFetch of the per-entity cell
                    // noun:id — missing or atom-valued answers "#" and the
                    // outer Filter drops the pair; a wide row shorter than
                    // the selector bottoms the whole answer (α strictness)
                    let mut out: Vec<N> = Vec::new();
                    for r in trows.iter() {
                        let id = match r {
                            N::S(cols) if !cols.is_empty() => cols[0].clone(),
                            _ => return Some(N::Bot),
                        };
                        let id_s = match &id {
                            N::A(l) => match &**l {
                                Leaf::S(s) => s.clone(),
                                Leaf::I(i) => i.to_string(),
                                _ => return Some(N::Bot),
                            },
                            _ => return Some(N::Bot),
                        };
                        let val = match fetch(&format!("{}:{}", noun_s, id_s)) {
                            None => hash.clone(),
                            Some(N::A(_)) => hash.clone(),
                            Some(N::S(w)) => {
                                if w.len() < col_i {
                                    return Some(N::Bot);
                                }
                                w[col_i - 1].clone()
                            }
                            Some(N::Bot) => return Some(N::Bot),
                        };
                        if !n_eq(&val, &hash) {
                            out.push(N::S(Rc::new(vec![id, val])));
                        }
                    }
                    N::S(Rc::new(out))
                }
                None => N::Bot,
            },
            "stage1_fields" => match x {
                N::S(v) if v.len() == 4 => {
                    let strv = |n: &N| -> Option<String> {
                        match n {
                            N::A(l) => match &**l { Leaf::S(s) => Some(s.clone()), _ => None },
                            _ => None,
                        }
                    };
                    let anyv = |n: &N| -> Option<String> {
                        match n { N::A(l) => leaf_str(l), _ => None }
                    };
                    let text = match strv(&v[0]) { Some(s) => s, None => return Some(N::Bot) };
                    let sid = match strv(&v[3]) { Some(s) => s, None => return Some(N::Bot) };
                    let mut vocab: Vec<(String, String)> = Vec::new();
                    if let N::S(ps) = &v[1] {
                        for p in ps.iter() {
                            if let N::S(pi) = p {
                                if pi.len() >= 2 {
                                    if let (Some(a), Some(b)) = (anyv(&pi[0]), anyv(&pi[1])) {
                                        vocab.push((a, b));
                                    }
                                }
                            }
                        }
                    }
                    let mut nouns: Vec<String> = Vec::new();
                    if let N::S(ns) = &v[2] {
                        for nx in ns.iter() {
                            match anyv(nx) {
                                Some(s) => nouns.push(s),
                                None => return Some(N::Bot),
                            }
                        }
                    }
                    let rows = stage1_rows_of(&text, &vocab, &nouns, &sid);
                    N::S(Rc::new(rows
                        .into_iter()
                        .map(|(ft, s, vv)| {
                            N::S(Rc::new(vec![
                                N::A(Rc::new(Leaf::S(ft))),
                                N::S(Rc::new(vec![
                                    N::A(Rc::new(Leaf::S(s))),
                                    N::A(Rc::new(Leaf::S(vv))),
                                ])),
                            ]))
                        })
                        .collect()))
                }
                _ => N::Bot,
            },
            "render:json" => {
                let v = n_to_v(x);
                let mut out = String::new();
                if v_json(&v, &mut out) {
                    N::A(Rc::new(Leaf::S(out)))
                } else {
                    N::Bot
                }
            }
            "strip_prefix" => match pairv(x) {
                Some((N::A(a), N::A(b))) => match (leaf_str(&a), leaf_str(&b)) {
                    (Some(p), Some(s)) => {
                        let t = s.strip_prefix(&p).map(|t| t.to_string()).unwrap_or(s);
                        N::A(Rc::new(Leaf::S(t)))
                    }
                    _ => N::Bot,
                },
                _ => N::Bot,
            },
            "escape_html" => match x {
                N::A(l) => {
                    let s = match &**l {
                        Leaf::S(s) => s.clone(),
                        Leaf::I(i) => i.to_string(),
                        _ => return Some(N::Bot),
                    };
                    let e = s.replace('&', "&amp;").replace('<', "&lt;")
                        .replace('>', "&gt;").replace('"', "&quot;");
                    N::A(Rc::new(Leaf::S(e)))
                }
                _ => N::Bot,
            },
            "skolem" => match x {
                N::S(xs) if !xs.is_empty() => {
                    let mut vals: Vec<String> = Vec::new();
                    let mut ok = true;
                    for v in xs.iter() {
                        match v {
                            N::A(l) => match &**l {
                                Leaf::S(s) => vals.push(s.clone()),
                                Leaf::I(i) => vals.push(i.to_string()),
                                _ => {
                                    ok = false;
                                    break;
                                }
                            },
                            _ => {
                                ok = false;
                                break;
                            }
                        }
                    }
                    if !ok {
                        N::Bot
                    } else {
                        let mut h: u64 = 14695981039346656037;
                        for b in vals.join("|").as_bytes() {
                            h ^= *b as u64;
                            h = h.wrapping_mul(1099511628211);
                        }
                        N::A(Rc::new(Leaf::S(format!("ve_{:016x}", h))))
                    }
                }
                _ => N::Bot,
            },
            "lex" => match x {
                N::A(l) => match leaf_str(l) {
                    Some(t) => {
                        let na = |s: String| N::A(Rc::new(Leaf::S(s)));
                        nseq(
                            lex_rows(&t)
                                .into_iter()
                                .map(|r| {
                                    nseq(vec![
                                        na(r.0),
                                        na(r.1),
                                        na(r.2),
                                        na(r.3),
                                        na(r.4),
                                        na(r.5),
                                        na(if r.6 { "T".into() } else { "F".into() }),
                                        na(r.7),
                                        na(if r.8 { "T".into() } else { "F".into() }),
                                        N::A(Rc::new(Leaf::I(r.9))),
                                    ])
                                })
                                .collect(),
                        )
                    }
                    None => N::Bot,
                },
                _ => N::Bot,
            },
            "implode" => match pairv(x) {
                Some((N::A(sep), N::S(ws))) => {
                    let sv = |n: &N| match n {
                        N::A(l) => leaf_str(l),
                        _ => None,
                    };
                    match leaf_str(&sep) {
                        Some(sp) => {
                            let parts: Option<Vec<String>> = ws.iter().map(sv).collect();
                            match parts {
                                Some(p) => N::A(Rc::new(Leaf::S(p.join(&sp)))),
                                None => N::Bot,
                            }
                        }
                        None => N::Bot,
                    }
                }
                _ => N::Bot,
            },
            "slug" => match x {
                N::A(l) => match leaf_str(l) {
                    Some(t) => N::A(Rc::new(Leaf::S(slug_str(&t)))),
                    None => N::Bot,
                },
                _ => N::Bot,
            },
            _ => return None,
        })
    }
}

fn write_n(n: &N, out: &mut String) {
    match n {
        N::Bot => out.push_str("null"),
        N::A(l) => match &**l {
            Leaf::S(s) => esc(s, out),
            Leaf::I(i) => out.push_str(&i.to_string()),
            Leaf::F(f) => {
                if f.fract() == 0.0 && f.is_finite() { out.push_str(&format!("{:.1}", f)); }
                else { out.push_str(&format!("{}", f)); }
            }
            Leaf::AppTag => out.push_str("\"#APP#\""),
        },
        N::S(v) => {
            out.push('[');
            for (i, x) in v.iter().enumerate() {
                if i > 0 { out.push(','); }
                write_n(x, out);
            }
            out.push(']');
        }
    }
}

fn j_to_n(j: &J) -> N {
    match j {
        J::S(s) => N::A(Rc::new(Leaf::S(s.clone()))),
        J::I(i) => N::A(Rc::new(Leaf::I(*i))),
        J::F(f) => N::A(Rc::new(Leaf::F(*f))),
        J::A(xs) => N::S(Rc::new(xs.iter().map(j_to_n).collect())),
        J::O(_) | J::Null | J::B(_) => N::Bot,
    }
}

// system:ev_cols' classification, natively (shared by the ev_cols and
// entity_view prims): the noun's rmapColumns layout in column order,
// each ft classified unary/value/ref with its played type and its
// deduped sql column name. Reads only the store spine the caller
// already built.
fn ev_cols_native(
    spine: &[(String, N)],
    noun: &str,
) -> Vec<(String, String, Option<String>, String)> {
    let fetch = |name: &str| -> Option<&N> {
        spine.iter().find(|(k, _)| k == name).map(|(_, v)| v)
    };
    let rows_of = |name: &str| -> Vec<N> {
        match fetch(name) {
            Some(N::S(v)) => v.to_vec(),
            _ => Vec::new(),
        }
    };
    let sv2 = |n: &N| -> Option<String> {
        match n { N::A(l) => leaf_str(l), _ => None }
    };
    // THE SUBTYPE TABLE RESOLUTION (2026-07-08): a subtype's fact
    // types absorb into its TOP SUPERTYPE's table (RMAP rule 2 — the
    // Support Request row lives in "Agent Chat"), so matching the
    // requested noun's NAME against the table column classifies ZERO
    // columns for every subtype. Resolve the noun's table through its
    // role-1 fact types first; a noun with no absorbed fts keeps its
    // own name (the own-table case).
    let noun_fts: Vec<String> = rows_of("role")
        .iter()
        .filter_map(|r| {
            if let N::S(cc) = r {
                if cc.len() >= 4
                    && sv2(&cc[3]).as_deref() == Some(noun)
                {
                    if let N::A(pl) = &cc[2] {
                        if matches!(&**pl, Leaf::I(1)) {
                            return sv2(&cc[1]);
                        }
                    }
                }
            }
            None
        })
        .collect();
    let table: String = rows_of("rmapColumns")
        .iter()
        .find_map(|r| {
            if let N::S(cc) = r {
                if cc.len() >= 3 {
                    if let (Some(t), Some(ft)) = (sv2(&cc[0]), sv2(&cc[2])) {
                        if noun_fts.iter().any(|f| *f == ft) {
                            return Some(t);
                        }
                    }
                }
            }
            None
        })
        .unwrap_or_else(|| noun.to_string());
    let mut colrows: Vec<(i64, String)> = rows_of("rmapColumns")
        .iter()
        .filter_map(|r| {
            if let N::S(cc) = r {
                if cc.len() >= 3 && sv2(&cc[0]).as_deref() == Some(table.as_str()) {
                    if let (N::A(cl), Some(ft)) = (&cc[1], sv2(&cc[2])) {
                        if let Leaf::I(ci) = &**cl {
                            return Some((*ci, ft));
                        }
                    }
                }
            }
            None
        })
        .collect();
    colrows.sort_by_key(|(c, _)| *c);
    let mut roles: Vec<(String, i64, String)> = Vec::new();
    for r in rows_of("role") {
        if let N::S(cc) = &r {
            if cc.len() >= 4 {
                if let (Some(ft), N::A(pl), Some(player)) =
                    (sv2(&cc[1]), &cc[2], sv2(&cc[3]))
                {
                    if let Leaf::I(pp) = &**pl {
                        roles.push((ft, *pp, player));
                    }
                }
            }
        }
    }
    let mut entities: Vec<String> = Vec::new();
    for r in rows_of("instanceOf") {
        if let N::S(cc) = &r {
            if cc.len() >= 2 && sv2(&cc[1]).as_deref() == Some("ObjectType") {
                if let Some(e) = sv2(&cc[0]) {
                    entities.push(e);
                }
            }
        }
    }
    let mut refmode: Vec<(String, String)> = Vec::new();
    for cell in ["refScheme", "refMode"] {
        for r in rows_of(cell) {
            if let N::S(cc) = &r {
                if cc.len() >= 2 {
                    if let (Some(a), Some(b)) = (sv2(&cc[0]), sv2(&cc[1])) {
                        if !refmode.iter().any(|(k, _)| *k == a) {
                            refmode.push((a, b));
                        }
                    }
                }
            }
        }
    }
    let mut out: Vec<(String, String, Option<String>, String)> = Vec::new();
    let mut counts: Vec<(String, i64)> = Vec::new();
    for (_c, ft) in &colrows {
        let rs: Vec<(i64, String)> = {
            let mut v: Vec<(i64, String)> = roles
                .iter()
                .filter(|(f, _, _)| f == ft)
                .map(|(_, p, pl)| (*p, pl.clone()))
                .collect();
            v.sort();
            v
        };
        let (kind, other): (&str, Option<String>) = if rs.len() == 1 {
            ("unary", None)
        } else {
            let o = rs.iter().find(|(_, pl)| *pl != noun).map(|(_, pl)| pl.clone());
            match &o {
                Some(pl) if entities.iter().any(|e| e == pl) => ("ref", o.clone()),
                Some(_) => ("value", o.clone()),
                None => ("value", None),
            }
        };
        let base = match kind {
            "unary" => sql_name(ft.strip_prefix(noun).unwrap_or(ft)),
            "ref" => {
                let o = other.clone().unwrap_or_default();
                let m = refmode
                    .iter()
                    .find(|(k, _)| *k == o)
                    .map(|(_, v)| v.clone())
                    .unwrap_or_else(|| "id".into());
                format!("{}_{}", sql_name(&o), sql_name(&m))
            }
            _ => match &other {
                Some(o) => sql_name(o),
                None => sql_name(ft),
            },
        };
        let n = {
            let e = counts.iter_mut().find(|(b, _)| *b == base);
            match e {
                Some((_, c)) => { *c += 1; *c }
                None => { counts.push((base.clone(), 1)); 1 }
            }
        };
        let col = if n >= 2 { format!("{}_{}", base, n) } else { base };
        out.push((ft.clone(), kind.to_string(), other, col));
    }
    out
}

fn n_cells_of(d: &N) -> Vec<(Leaf, N)> {
    let mut out: Vec<(Leaf, N)> = Vec::new();
    if let N::S(cells) = d {
        for c in cells.iter() {
            if let N::S(it) = c {
                if it.len() == 3 {
                    if let (N::A(l0), N::A(k)) = (&it[0], &it[1]) {
                        if matches!(&**l0, Leaf::S(s) if s == "CELL")
                            && !out.iter().any(|(e, _)| e.nateq(k)) {
                            out.push(((**k).clone(), it[2].clone()));
                        }
                    }
                }
            }
        }
    }
    out
}

// v_to_n and n_to_v carry a value between the Scott carrier V and the native
// carrier N, shape for shape: an atom over the same Leaf, a sequence over the
// converted elements, and bottom for bottom. They mirror to_v and j_to_n (an
// array becomes a SEQ, a scalar an ATOM, nothing else appears) so a value
// reduced on one carrier reads identically on the other. run_rules evaluates
// every rule body on N and reads the rows back on V through this pair.
fn v_to_n(v: &V) -> N {
    match shape(v) {
        Shape::Atom(l) => N::A(l),
        Shape::Seq(l) => N::S(Rc::new(items(&l).iter().map(v_to_n).collect())),
        Shape::Bot => N::Bot,
    }
}
fn n_to_v(n: &N) -> V {
    match n {
        N::A(l) => atom((**l).clone()),
        N::S(v) => seq(from_vec(v.iter().map(n_to_v).collect())),
        N::Bot => bot(),
    }
}

// atom_n is the native atom addressing a cell by its leaf, the N twin of atom.
fn atom_n(l: &Leaf) -> N {
    N::A(Rc::new(l.clone()))
}

// neval_rule reduces ONE rule body on the native carrier: it builds an NEval
// over the current native store view (the maintained cells and defs, the
// resident process defs, unbounded fuel as run_rules always passes None) and
// answers the reduced rows back on V, so every pass around it (dedup by key,
// sort, store) is byte for byte the Scott path it replaces. The rule id (or a
// "~d" delta variant) resolves through the native cells exactly as it resolved
// through D's DEFS cells under Scott; the operand is the native store, or the
// native pair the delta variants take.
fn neval_rule(ncells: &[(Leaf, N)], nprocess: &[(String, N)], nd: &N, rid: &Leaf, operand: N) -> V {
    let ev = NEval {
        cells: ncells.to_vec(),
        process: nprocess.to_vec(),
        defs_n: nd.clone(),
        fuel: std::cell::Cell::new(-1), // None means unbounded; -1 is the native carrier's unbounded
    };
    n_to_v(&ev.mu(napp(atom_n(rid), operand)))
}

// ============================ JSON (hand-rolled, zero deps) ==================
#[derive(Debug, Clone)]
enum J {
    // Null and B never appear in scenario payloads (V has no such values);
    // the MCP transport carries them in every real client's requests.
    Null,
    B(bool),
    S(String),
    I(i64),
    F(f64),
    A(Vec<J>),
    O(Vec<(String, J)>),
}

struct P<'a> {
    b: &'a [u8],
    i: usize,
}
impl<'a> P<'a> {
    fn ws(&mut self) {
        while self.i < self.b.len() && (self.b[self.i] as char).is_whitespace() {
            self.i += 1;
        }
    }
    fn parse(&mut self) -> J {
        self.ws();
        match self.b[self.i] {
            b'"' => J::S(self.string()),
            b'[' => {
                self.i += 1;
                let mut v = Vec::new();
                loop {
                    self.ws();
                    if self.b[self.i] == b']' {
                        self.i += 1;
                        break;
                    }
                    v.push(self.parse());
                    self.ws();
                    if self.b[self.i] == b',' {
                        self.i += 1;
                    }
                }
                J::A(v)
            }
            b'{' => {
                self.i += 1;
                let mut v = Vec::new();
                loop {
                    self.ws();
                    if self.b[self.i] == b'}' {
                        self.i += 1;
                        break;
                    }
                    let k = self.string();
                    self.ws();
                    self.i += 1; // ':'
                    v.push((k, self.parse()));
                    self.ws();
                    if self.b[self.i] == b',' {
                        self.i += 1;
                    }
                }
                J::O(v)
            }
            // true, false, and null ride only the MCP transport; the parser
            // trusts the spelling and consumes the bytes by length, extending
            // number()'s trust in its input.
            b't' => {
                self.i += 4;
                J::B(true)
            }
            b'f' => {
                self.i += 5;
                J::B(false)
            }
            b'n' => {
                self.i += 4;
                J::Null
            }
            _ => self.number(),
        }
    }
    fn string(&mut self) -> String {
        self.i += 1; // opening quote
        let mut s = String::new();
        loop {
            match self.b[self.i] {
                b'"' => {
                    self.i += 1;
                    return s;
                }
                b'\\' => {
                    self.i += 1;
                    match self.b[self.i] {
                        b'n' => s.push('\n'),
                        b't' => s.push('\t'),
                        b'r' => s.push('\r'),
                        b'b' => s.push('\u{8}'),
                        b'f' => s.push('\u{c}'),
                        b'u' => {
                            let h = std::str::from_utf8(&self.b[self.i + 1..self.i + 5]).unwrap();
                            let mut cp = u32::from_str_radix(h, 16).unwrap();
                            self.i += 4;
                            if (0xD800..0xDC00).contains(&cp) {
                                // surrogate pair
                                self.i += 3; // skip \u
                                let h2 =
                                    std::str::from_utf8(&self.b[self.i + 1..self.i + 5]).unwrap();
                                let lo = u32::from_str_radix(h2, 16).unwrap();
                                self.i += 4;
                                cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                            }
                            s.push(char::from_u32(cp).unwrap_or('?'));
                        }
                        c => s.push(c as char),
                    }
                    self.i += 1;
                }
                _ => {
                    // consume one UTF-8 scalar
                    let start = self.i;
                    let len = match self.b[self.i] {
                        x if x < 0x80 => 1,
                        x if x < 0xE0 => 2,
                        x if x < 0xF0 => 3,
                        _ => 4,
                    };
                    self.i += len;
                    s.push_str(std::str::from_utf8(&self.b[start..self.i]).unwrap_or("?"));
                }
            }
        }
    }
    fn number(&mut self) -> J {
        let start = self.i;
        let mut float = false;
        while self.i < self.b.len() {
            match self.b[self.i] {
                b'0'..=b'9' | b'-' | b'+' => self.i += 1,
                b'.' | b'e' | b'E' => {
                    float = true;
                    self.i += 1;
                }
                _ => break,
            }
        }
        let s = std::str::from_utf8(&self.b[start..self.i]).unwrap();
        if float {
            J::F(s.parse().unwrap())
        } else {
            J::I(s.parse().unwrap())
        }
    }
}

fn jget<'a>(o: &'a J, key: &str) -> Option<&'a J> {
    if let J::O(kv) = o {
        kv.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    } else {
        None
    }
}

fn to_v(j: &J) -> V {
    match j {
        J::S(s) => atom(Leaf::S(s.clone())),
        J::I(i) => atom(Leaf::I(*i)),
        J::F(f) => atom(Leaf::F(*f)),
        J::A(xs) => seq(from_vec(xs.iter().map(to_v).collect())),
        // Scenario values never carry objects, booleans, or null; they land
        // as bottom, total here as everywhere.
        J::O(_) | J::Null | J::B(_) => bot(),
    }
}

fn esc(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write_v(v: &V, out: &mut String) {
    match shape(v) {
        Shape::Bot => out.push_str("null"),
        Shape::Atom(l) => match &*l {
            Leaf::S(s) => esc(s, out),
            Leaf::I(i) => out.push_str(&i.to_string()),
            Leaf::F(f) => {
                // Python-json-compatible float text (round-trips through json.loads)
                if f.fract() == 0.0 && f.is_finite() {
                    out.push_str(&format!("{:.1}", f));
                } else {
                    out.push_str(&format!("{}", f));
                }
            }
            Leaf::AppTag => out.push_str("\"#APP#\""),
        },
        Shape::Seq(l) => {
            out.push('[');
            let xs = items(&l);
            for (i, x) in xs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_v(x, out);
            }
            out.push(']');
        }
    }
}

// write_j prints any parsed J back verbatim; the MCP binding echoes request
// ids and parameters with it (write_v serves reduced values, not requests).
fn write_j(j: &J, out: &mut String) {
    match j {
        J::Null => out.push_str("null"),
        J::B(b) => out.push_str(if *b { "true" } else { "false" }),
        J::S(s) => esc(s, out),
        J::I(i) => out.push_str(&i.to_string()),
        J::F(f) => {
            if f.fract() == 0.0 && f.is_finite() {
                out.push_str(&format!("{:.1}", f));
            } else {
                out.push_str(&format!("{}", f));
            }
        }
        J::A(xs) => {
            out.push('[');
            for (i, x) in xs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_j(x, out);
            }
            out.push(']');
        }
        J::O(kv) => {
            out.push('{');
            for (i, (k, v)) in kv.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                esc(k, out);
                out.push(':');
                write_j(v, out);
            }
            out.push('}');
        }
    }
}

// ============================ the scenario runner ============================
struct Srv {
    d: V,
    cells: Vec<(Leaf, V)>,
    mu: V,
    nd: N,
    ncells: Vec<(Leaf, N)>,
    nprocess: Vec<(String, N)>,
}

fn handle(j: &J, srv: &mut Srv, serve: bool) -> String {
    match jget(j, "overrides") {
        Some(J::I(0)) => FAST.with(|r| r.borrow_mut().clear()),
        Some(J::I(_)) => {
            FAST.with(|r| r.borrow_mut().clear());
            register_overrides();
        }
        _ => {}
    }
    if let Some(J::A(procs)) = jget(j, "process") {
        PROCESS.with(|p| {
            let mut b = p.borrow_mut();
            b.clear();
            srv.nprocess.clear();
            for entry in procs {
                if let J::A(pair) = entry {
                    if let (J::S(name), val) = (&pair[0], &pair[1]) {
                        b.push((name.clone(), to_v(val)));
                        srv.nprocess.push((name.clone(), j_to_n(val)));
                    }
                }
            }
        });
    }
    if let Some(dj) = jget(j, "d") {
        srv.d = to_v(dj);
        srv.cells = cells_of(&srv.d); // cached once per retained store (resident mode)
        srv.nd = j_to_n(dj);
        srv.ncells = n_cells_of(&srv.nd);
    }
    if let Some(J::S(op)) = jget(j, "op") {
        // a verb request: the store preamble above has already applied, so
        // {"d": .., "op": ..} sets the resident store and asks in one line
        // (an op line answers the op alone; cases ride their own lines)
        return handle_op(op, j, srv);
    }
    let native = matches!(jget(j, "engine"), Some(J::S(s)) if s == "native");
    let mut outs: Vec<String> = Vec::new();
    if jget(j, "dump").is_some() {
        let mut s = String::new();
        write_v(&srv.d, &mut s);
        outs.push(s);
    }
    if let Some(J::A(cases)) = jget(j, "cases") {
        for case in cases {
            if native {
                // the native-carrier machine: the deepest override, same protocol
                let f = j_to_n(jget(case, "f").unwrap());
                let x = match (jget(case, "x"), jget(case, "xd")) {
                    (Some(xj), _) => j_to_n(xj),
                    (None, Some(fj)) => nseq(vec![j_to_n(fj), srv.nd.clone()]),
                    _ => N::Bot,
                };
                let fuel = match jget(case, "fuel") {
                    Some(J::I(n)) if *n > 0 => *n,
                    _ => -1,
                };
                let ev = NEval {
                    cells: srv.ncells.clone(),
                    process: srv.nprocess.clone(),
                    defs_n: srv.nd.clone(),
                    fuel: std::cell::Cell::new(fuel),
                };
                let res = ev.mu(napp(f, x));
                let mut s = String::new();
                write_n(&res, &mut s);
                outs.push(s);
                continue;
            }
            let f = to_v(jget(case, "f").unwrap());
            // "x" carries the operand verbatim; "xd" pairs a fact with the RETAINED
            // store (⟨fact, D⟩ without re-serializing D — the resident protocol)
            let x = match (jget(case, "x"), jget(case, "xd")) {
                (Some(xj), _) => to_v(xj),
                (None, Some(fj)) => seq(cons(to_v(fj), cons(srv.d.clone(), nil()))),
                _ => bot(),
            };
            let fuel = match jget(case, "fuel") {
                Some(J::I(n)) if *n > 0 => Some(*n),
                _ => None,
            };
            FRAME.with(|fr| {
                fr.borrow_mut().push(Frame { cells: srv.cells.clone(), d: srv.d.clone(), fuel })
            });
            let res = srv.mu.app(mkapp(f, x));
            FRAME.with(|fr| {
                fr.borrow_mut().pop();
            });
            if matches!(jget(case, "retain"), Some(J::I(1))) {
                // commit the step's D' into the retained store (the cluster's owner
                // instance evolves; a refused step retains nothing)
                let it = items(&list_of(&res));
                if it.len() == 2 && !isbot(&it[0]) {
                    srv.d = it[1].clone();
                    srv.cells = cells_of(&srv.d);
                    // MIRROR COHERENCE (2026-07-08): every site that
                    // replaces the retained store refreshes the native
                    // mirror, and every native read TRUSTS it — the
                    // stale-mirror caveat retires at the source
                    srv.nd = v_to_n(&srv.d);
                    srv.ncells = n_cells_of(&srv.nd);
                }
            }
            let mut s = String::new();
            write_v(&res, &mut s);
            outs.push(s);
        }
    }
    if serve {
        format!("[{}]", outs.join(","))                       // one line per request
    } else {
        outs.join("
")
    }
}

// ============================ the verb surface ================================
// The system's verb table (python/protocol.py: SESSION_VERBS + APP_VERBS, with
// the Registry-backed synthesize/explain) is surface-agnostic — every binding
// lists the SAME verbs. The resident kernel mirrors it as JSON-RPC-style
// requests on the serve loop, one object per line:
//   {"op": "verbs"}                        -> the table + the resident subset
//   {"op": "query", "fact_type": NAME}     -> (ast:FetchPop : NAME) : D
//   {"op": "cells", "pattern": SUBSTR?}    -> cell names + row counts over D
//   {"op": "synthesize_pairs", "id": ID}   -> (system:verbalize : ID) : D
//   {"op": "run_rules", "changed": [..]?}  -> the derivation fixpoint over D
// answered as {"op": .., "result": ..} or {"op": .., "error": ..}. Every
// reduction runs over the RESIDENT store the loop already holds (set and
// evolved via the d / retain ops) through the canonical definitions the kernel
// loads (canon_defs) — no engine semantics re-live here. Verbs needing the
// apps registry (readings compile, sqlite) stay host-side: the table names
// them; the resident subset serves what the store alone can answer.
const SESSION_VERBS: [&str; 11] =
    ["apps_check", "apps_compile", "apps_create", "apps_current", "apps_list",
     "apps_register", "apps_status", "apps_use", "context", "engine_version", "orient"];
const APP_VERBS: [&str; 13] =
    ["apply", "ask", "cells", "compile", "explain", "get", "induce", "propose",
     "query", "retract", "schema", "sql", "synthesize"];
const RESIDENT_OPS: [&str; 8] =
    ["base_seed", "cells", "compile_model", "query", "run_rules", "sql_project",
     "synthesize_pairs", "verbs"];

fn reduce_in(mu: &V, cells: &[(Leaf, V)], d: &V, f: V, x: V, fuel: Option<i64>) -> V {
    // one reduction under a given store binding, the case path's frame
    // discipline: the frame carries the store's cells (so compiled defs in D
    // resolve first, Backus 14.6) and the whole D (for the DEFS accessor)
    FRAME.with(|fr| {
        fr.borrow_mut().push(Frame { cells: cells.to_vec(), d: d.clone(), fuel })
    });
    let res = mu.app(mkapp(f, x));
    FRAME.with(|fr| {
        fr.borrow_mut().pop();
    });
    res
}

fn reduce_over(srv: &Srv, f: V, x: V, fuel: Option<i64>) -> V {
    // one reduction over the resident store
    reduce_in(&srv.mu, &srv.cells, &srv.d, f, x, fuel)
}

// (system:verbalize : id) : D over the NATIVE carrier — the ~40x plumb
// (2026-07-08). READS TRUST THE MIRROR: every site that replaces the
// retained store refreshes srv.nd/srv.ncells (the coherence audit,
// 2026-07-08), so the per-call v_to_n rebuild — 247 s at tasks scale —
// retires with the staleness it guarded against. Serves both the
// synthesize_pairs op (raw pairs) and the MCP synthesize tool (rendered
// to the Registry's shape).
fn native_verbalize(srv: &Srv, id: &V) -> V {
    let ev = NEval {
        cells: srv.ncells.clone(),
        process: srv.nprocess.clone(),
        defs_n: srv.nd.clone(),
        fuel: std::cell::Cell::new(-1),
    };
    n_to_v(&ev.mu(napp(
        napp(N::A(Rc::new(Leaf::S("system:verbalize".into()))), v_to_n(id)),
        srv.nd.clone(),
    )))
}

fn scalar_atom(j: &J) -> Option<V> {
    // a scalar request parameter as the atom the canon addresses cells by
    match j {
        J::S(s) => Some(atom(Leaf::S(s.clone()))),
        J::I(i) => Some(atom(Leaf::I(*i))),
        J::F(f) => Some(atom(Leaf::F(*f))),
        _ => None,
    }
}

// ============================ the derivation fixpoint =========================
// Phase two of the run_rules port (python/engine.py run_rules, lines 1046
// through 1110): the SEMI-NAIVE positive-rule fixpoint with frontier
// bounding (Bancilhon-Ramakrishnan 1986). Round one evaluates full rule
// bodies against the current D, bounded by the optional "changed" frontier
// (only rules whose ruleReads intersect it fire); every later round joins
// only each head's per-round delta through the stored ~d variants, one per
// atom position from the ruleAtom facts, each applied to the pair of the
// sorted delta rows and D. Rules without atom facts fall back to full
// evaluation in rounds where their reads changed. A rule id (and each of its
// "<rule id>~d<position>" variants) resolves to its compiled object through
// D's OWN DEFS cells (ast:DefineIn stored it there at compile time), exactly
// as Python evaluates inside defs.step(D), and every reduction runs through
// the same mu and frame the ops use. New rows union into the head cell and
// the loop stops when a round adds nothing. Rules are positive and monotone,
// so the loop reaches the least fixed point (Knaster-Tarski), and finiteness
// bounds the rounds. Rules named by ruleAgg are SKIPPED here, never mis-run:
// an aggregate head supersedes per group instead of unioning, which is the
// next stratum's contract. Deferred to later phases: the FAST rule twins,
// the keyed upserts, the DRed sweeps, and the joint upper-strata iteration.

fn float_text(f: f64) -> String {
    if f.fract() == 0.0 && f.is_finite() {
        format!("{:.1}", f)
    } else {
        format!("{}", f)
    }
}

fn leaf_text(l: &Leaf) -> String {
    match l {
        Leaf::S(s) => s.clone(),
        Leaf::I(i) => i.to_string(),
        Leaf::F(f) => float_text(*f),
        Leaf::AppTag => "#APP#".to_string(),
    }
}

// set_key renders a value for SET membership the way engine.py's row sets
// compare (Python ==): an int and a float of equal value coalesce, while a
// numeric-looking string stays distinct from the number. Strings are length
// prefixed so no content can imitate the structure.
fn set_key(v: &V, out: &mut String) {
    match shape(v) {
        Shape::Atom(l) => match &*l {
            Leaf::S(s) => {
                out.push('s');
                out.push_str(&s.len().to_string());
                out.push(':');
                out.push_str(s);
            }
            Leaf::I(i) => {
                out.push('n');
                out.push_str(&i.to_string());
            }
            Leaf::F(f) => {
                out.push('n');
                // an integral float keys like its int (Python 1 == 1.0);
                // the range guard keeps the cast exact
                if f.fract() == 0.0 && f.is_finite() && f.abs() < 9.0e18 {
                    out.push_str(&(*f as i64).to_string());
                } else {
                    out.push_str(&format!("{}", f));
                }
            }
            Leaf::AppTag => out.push('t'),
        },
        Shape::Seq(l) => {
            out.push('(');
            for x in items(&l) {
                set_key(&x, out);
                out.push(',');
            }
            out.push(')');
        }
        Shape::Bot => out.push('!'),
    }
}

fn key_of(v: &V) -> String {
    let mut s = String::new();
    set_key(v, &mut s);
    s
}

// row_sort_key mirrors engine.py's _rowsort: type name then lexical text per
// element, so mixed-type cells (a lexical '150' beside a derived 150) order
// deterministically with no cross-type comparison.
fn row_sort_key(row: &V) -> Vec<(&'static str, String)> {
    fn elem(v: &V) -> (&'static str, String) {
        match shape(v) {
            Shape::Atom(l) => match &*l {
                Leaf::S(s) => ("str", s.clone()),
                Leaf::I(i) => ("int", i.to_string()),
                Leaf::F(f) => ("float", float_text(*f)),
                Leaf::AppTag => ("AppTag", String::new()),
            },
            Shape::Seq(_) => ("tuple", key_of(v)),
            Shape::Bot => ("bot", String::new()),
        }
    }
    match shape(row) {
        Shape::Seq(l) => items(&l).iter().map(elem).collect(),
        _ => vec![elem(row)],
    }
}

fn sort_rows(rows: &mut Vec<V>) {
    let mut keyed: Vec<(Vec<(&'static str, String)>, V)> =
        rows.drain(..).map(|r| (row_sort_key(&r), r)).collect();
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    rows.extend(keyed.into_iter().map(|(_, r)| r));
}

// group_key keys a row over every column but the last (engine.py's r[:-1]),
// the aggregate group: on supersession a produced group replaces its stored
// row while a group no rule produced survives. It shares key_of's encoding
// over the truncated row, so a group key never collides across arities and
// compares exactly as the Python tuple slice does. A single-column row has
// the empty group (a global aggregate), matching r[:-1] on a 1-tuple.
fn group_key(row: &V) -> String {
    match shape(row) {
        Shape::Seq(l) => {
            let mut cols = items(&l);
            cols.pop();
            key_of(&seq(from_vec(cols)))
        }
        _ => key_of(row),
    }
}

// pop_rows is FetchPop's view over the cached cell index: the named cell's
// rows, with an absent cell or an atom-valued cell the empty population.
fn pop_rows(cells: &[(Leaf, V)], name: &Leaf) -> Vec<V> {
    match cells.iter().find(|(k, _)| k.nateq(name)) {
        Some((_, contents)) => match shape(contents) {
            Shape::Seq(l) => items(&l),
            _ => Vec::new(),
        },
        None => Vec::new(),
    }
}

// store_into is ast:Store run natively (Backus 13.3.4, pop then push): drop
// the first top-level entry carrying the name in its second position (the
// ast:named key), prepend the fresh CELL triple, and keep the cached index
// consistent with the new D. Real stores hold only CELL triples, so the
// index and the pop agree by construction.
fn store_into(
    d: &mut V,
    cells: &mut Vec<(Leaf, V)>,
    nd: &mut N,
    ncells: &mut Vec<(Leaf, N)>,
    name: &Leaf,
    contents: V,
) {
    // the native mirror of the fresh contents, taken before contents moves into
    // the V store below, so the native view updates for this head in lockstep
    let ncontents = v_to_n(&contents);
    let mut entries = items(&list_of(d));
    if let Some(pos) = entries.iter().position(|c| {
        let it = items(&list_of(c));
        it.len() >= 2 && matches!(aval(&it[1]), Some(k) if k.nateq(name))
    }) {
        entries.remove(pos);
    }
    entries.insert(
        0,
        seq(from_vec(vec![
            atom(Leaf::S("CELL".into())),
            atom(name.clone()),
            contents.clone(),
        ])),
    );
    *d = seq(from_vec(entries));
    match cells.iter_mut().find(|(k, _)| k.nateq(name)) {
        Some(slot) => slot.1 = contents,
        None => cells.push((name.clone(), contents)),
    }
    // The SAME pop-then-push on the parallel N store: drop the head's old cell,
    // prepend its fresh CELL triple at position zero (so D and the native view
    // agree on cell order, and FetchPop's first match agrees on both), and keep
    // the native cell index consistent. A rule stored mid-round is thus visible
    // to the next rule's NEval the instant it lands, exactly as D threads the
    // fresh cell to the next Scott reduction.
    let mut nentries: Vec<N> = match nd {
        N::S(v) => v.to_vec(),
        _ => Vec::new(),
    };
    if let Some(pos) = nentries.iter().position(|c| {
        matches!(c, N::S(it) if it.len() >= 2 && matches!(&it[1], N::A(k) if k.nateq(name)))
    }) {
        nentries.remove(pos);
    }
    nentries.insert(
        0,
        N::S(Rc::new(vec![
            N::A(Rc::new(Leaf::S("CELL".into()))),
            N::A(Rc::new(name.clone())),
            ncontents.clone(),
        ])),
    );
    *nd = N::S(Rc::new(nentries));
    match ncells.iter_mut().find(|(k, _)| k.nateq(name)) {
        Some(slot) => slot.1 = ncontents,
        None => ncells.push((name.clone(), ncontents)),
    }
}

// setcell_into is engine.py _reconcile_absorbed_heads' setcell (:1165): the
// reassembly's write REPLACES IN PLACE when the cell exists and APPENDS AT
// THE END when it doesn't — Store's remove+prepend would re-top the written
// cells and reorder the dump against python (order forensics, 2026-07-11).
fn setcell_into(
    d: &mut V,
    cells: &mut Vec<(Leaf, V)>,
    nd: &mut N,
    ncells: &mut Vec<(Leaf, N)>,
    name: &Leaf,
    contents: V,
) {
    let ncontents = v_to_n(&contents);
    let cellv = seq(from_vec(vec![
        atom(Leaf::S("CELL".into())),
        atom(name.clone()),
        contents.clone(),
    ]));
    let mut entries = items(&list_of(d));
    match entries.iter().position(|c| {
        let it = items(&list_of(c));
        it.len() >= 2 && matches!(aval(&it[1]), Some(k) if k.nateq(name))
    }) {
        Some(p) => entries[p] = cellv,
        None => entries.push(cellv),
    }
    *d = seq(from_vec(entries));
    match cells.iter_mut().find(|(k, _)| k.nateq(name)) {
        Some(slot) => slot.1 = contents,
        None => cells.push((name.clone(), contents)),
    }
    let ncellv = N::S(Rc::new(vec![
        N::A(Rc::new(Leaf::S("CELL".into()))),
        N::A(Rc::new(name.clone())),
        ncontents.clone(),
    ]));
    let mut nentries: Vec<N> = match nd {
        N::S(v) => v.to_vec(),
        _ => Vec::new(),
    };
    match nentries.iter().position(|c| {
        matches!(c, N::S(it) if it.len() >= 2 && matches!(&it[1], N::A(k) if k.nateq(name)))
    }) {
        Some(p) => nentries[p] = ncellv,
        None => nentries.push(ncellv),
    }
    *nd = N::S(Rc::new(nentries));
    match ncells.iter_mut().find(|(k, _)| k.nateq(name)) {
        Some(slot) => slot.1 = ncontents,
        None => ncells.push((name.clone(), ncontents)),
    }
}

// ============================ FastStore: the store twin (#35) ==================
// docs/2026-07-11-store-twin-spec.md. ONE store, no redundant views: op_run_rules'
// round loop paid a four-views tax on every write (Scott `d`, the Vec<(Leaf,V)>
// `cells` cache, the native `nd`, the Vec<(Leaf,N)> `ncells` cache) -- store_into/
// setcell_into rebuilt BOTH the Scott cons-list (a walk of closure calls) and the
// native Vec on every single write, and pop_rows scanned `cells` linearly by name.
// FastStore replaces all four with one HashMap-indexed, doubly-linked-list-in-an-
// arena cell sequence: O(1) amortized store/setcell/pop_rows, and the Scott `d`
// materializes ONLY at the op's exit (nothing inside the loop -- pop_rows,
// neval_rule, eval_rules -- ever reads it mid-round; only the final commit does).
// The native carrier's contract (NEval wants a monolithic Vec<(Leaf,N)> / N
// snapshot per rule evaluation) is UNCHANGED -- ncells_native/nd_native satisfy it
// from a lazy per-node cache (invalidated only on THAT node's own write) plus a
// whole-store cache (invalidated on ANY write, rebuilt from the per-node caches --
// so a cell read many times between writes, or a rule considered but not fired,
// costs nothing beyond the first read since the last write).
//
// Built from raw_cells_of, NOT cells_of: a pre-existing shadowed same-named cell
// (raw_cells_of's own comment two screens below -- "duplicates preserved...
// cells_of... would silently drop a shadowed same-named cell") rides along in the
// list inertly, exactly as `d`'s own pop-only-the-first-match Store semantics
// leave it today -- the twin changes REPRESENTATION, never boundary semantics
// (the frontier-failure lesson: mirror the canon exactly, never innovate it).
struct FSNode {
    name: Leaf,
    content: V,
    ncache: RefCell<Option<N>>,
    prev: Option<usize>,
    next: Option<usize>,
}

struct FastStore {
    nodes: Vec<Option<FSNode>>,
    free: Vec<usize>,
    head: Option<usize>,
    tail: Option<usize>,
    // name key (Self::key) -> node index of the FIRST (active, first-match-wins)
    // occurrence; a name never in this map has no active cell (pop_rows: empty).
    active: HashMap<String, usize>,
    // the CACHE order: name keys in first-appearance order, APPEND-ONLY, never
    // reordered by a later Store. This mirrors store_into/setcell_into's OWN
    // `cells`/`ncells` Vec maintenance precisely -- both update an existing
    // name's slot IN PLACE (`cells.iter_mut().find(..).slot.1 = contents`) and
    // only PUSH a brand new name at the end; unlike `d`/`nd`, that Vec is never
    // re-topped on a Store. The distinction is observable: srv.cells feeds the
    // {"op":"cells"} verb directly (main.rs's "cells" op iterates &srv.cells in
    // order), so this is a second dump-shaped surface, not an internal detail.
    cache_order: Vec<String>,
    // the whole-store native snapshot, invalidated to None by ANY store/setcell;
    // rebuilt lazily (and only then) by ncells_native/nd_native.
    whole: RefCell<Option<(Rc<Vec<(Leaf, N)>>, N)>>,
}

impl FastStore {
    // a type-strict cell-name key mirroring Leaf::nateq exactly (S/I/F/AppTag
    // never cross-equal; unlike key_of/set_key this does NOT coalesce an int and
    // an equal-valued float, since store_into/setcell_into/pop_rows all match
    // cell names via nateq, never via python-set-style value equality). The 's'
    // branch is length-prefixed so no string's content can imitate another
    // variant's tag.
    fn key(l: &Leaf) -> String {
        match l {
            Leaf::S(s) => format!("s{}:{}", s.len(), s),
            Leaf::I(i) => format!("I{}", i),
            Leaf::F(f) => format!("F{}", float_text(*f)),
            Leaf::AppTag => "T".to_string(),
        }
    }

    // ENTRY conversion (paid ONCE): raw_cells_of walks d's Scott cons-list a
    // single time, duplicates preserved; the native cache seeds for free from
    // srv.ncells wherever the mirror-coherence invariant already computed it
    // (2026-07-08's write-site audit), so entry pays no redundant v_to_n for any
    // cell already resident.
    fn from_srv(srv: &Srv) -> FastStore {
        let raw = raw_cells_of(&srv.d);
        let seed: HashMap<String, N> = srv
            .ncells
            .iter()
            .map(|(k, v)| (FastStore::key(k), v.clone()))
            .collect();
        let mut fs = FastStore {
            nodes: Vec::with_capacity(raw.len()),
            free: Vec::new(),
            head: None,
            tail: None,
            active: HashMap::with_capacity(raw.len()),
            cache_order: Vec::with_capacity(raw.len()),
            whole: RefCell::new(None),
        };
        let mut prev_idx: Option<usize> = None;
        for (name, content) in raw {
            let k = FastStore::key(&name);
            let ncache = seed.get(&k).cloned();
            let idx = fs.nodes.len();
            fs.nodes.push(Some(FSNode {
                name: name.clone(),
                content,
                ncache: RefCell::new(ncache),
                prev: prev_idx,
                next: None,
            }));
            if let Some(p) = prev_idx {
                fs.nodes[p].as_mut().unwrap().next = Some(idx);
            } else {
                fs.head = Some(idx);
            }
            prev_idx = Some(idx);
            if !fs.active.contains_key(&k) {
                fs.cache_order.push(k.clone()); // cells_of's own first-match order
            }
            fs.active.entry(k).or_insert(idx); // first occurrence wins; a
                                                // deeper same-named raw entry
                                                // stays un-indexed (inert)
        }
        fs.tail = prev_idx;
        fs
    }

    fn alloc(&mut self, node: FSNode) -> usize {
        if let Some(idx) = self.free.pop() {
            self.nodes[idx] = Some(node);
            idx
        } else {
            self.nodes.push(Some(node));
            self.nodes.len() - 1
        }
    }

    // splice a node out of the dump-order list WITHOUT touching `active` --
    // callers that pop an active occurrence remove its `active` entry
    // themselves first (Store: about to prepend a brand new node in its place).
    fn unlink(&mut self, idx: usize) {
        let (prev, next) = {
            let node = self.nodes[idx].as_ref().unwrap();
            (node.prev, node.next)
        };
        match prev {
            Some(p) => self.nodes[p].as_mut().unwrap().next = next,
            None => self.head = next,
        }
        match next {
            Some(n) => self.nodes[n].as_mut().unwrap().prev = prev,
            None => self.tail = prev,
        }
        self.nodes[idx] = None;
        self.free.push(idx);
    }

    fn push_front(&mut self, idx: usize) {
        let old_head = self.head;
        {
            let node = self.nodes[idx].as_mut().unwrap();
            node.prev = None;
            node.next = old_head;
        }
        if let Some(h) = old_head {
            self.nodes[h].as_mut().unwrap().prev = Some(idx);
        }
        self.head = Some(idx);
        if self.tail.is_none() {
            self.tail = Some(idx);
        }
    }

    fn push_back(&mut self, idx: usize) {
        let old_tail = self.tail;
        {
            let node = self.nodes[idx].as_mut().unwrap();
            node.next = None;
            node.prev = old_tail;
        }
        if let Some(t) = old_tail {
            self.nodes[t].as_mut().unwrap().next = Some(idx);
        }
        self.tail = Some(idx);
        if self.head.is_none() {
            self.head = Some(idx);
        }
    }

    fn iter_indices(&self) -> Vec<usize> {
        let mut out = Vec::new();
        let mut cur = self.head;
        while let Some(idx) = cur {
            out.push(idx);
            cur = self.nodes[idx].as_ref().unwrap().next;
        }
        out
    }

    // FetchPop's view over the active index: O(1) to the top pop; rows stay
    // decoded (Vec<V>) -- the read side of "no repeated from_lam".
    fn pop_rows(&self, name: &Leaf) -> Vec<V> {
        match self.active.get(&Self::key(name)) {
            Some(&idx) => {
                let node = self.nodes[idx].as_ref().unwrap();
                match shape(&node.content) {
                    Shape::Seq(l) => items(&l),
                    _ => Vec::new(),
                }
            }
            None => Vec::new(),
        }
    }

    // presence, independent of row content (an explicitly-materialized empty
    // cell reads differently from an absent one -- op_run_rules' passHeads
    // fallback gate needs exactly this distinction).
    fn has_cell(&self, name: &Leaf) -> bool {
        self.active.contains_key(&Self::key(name))
    }

    // ast:Store (Backus 13.3.4): pop the topmost same-named cell, prepend the
    // new one at the front -- O(1) via the active index + the linked list, no
    // Scott/native cons-list rebuild.
    fn store(&mut self, name: &Leaf, contents: V) {
        let k = Self::key(name);
        match self.active.remove(&k) {
            Some(idx) => self.unlink(idx),
            // a brand new name: joins the CACHE order at the end (cache_order
            // is append-only, independent of dump-order re-topping below)
            None => self.cache_order.push(k.clone()),
        }
        let idx = self.alloc(FSNode {
            name: name.clone(),
            content: contents,
            ncache: RefCell::new(None),
            prev: None,
            next: None,
        });
        self.push_front(idx);
        self.active.insert(k, idx);
        *self.whole.borrow_mut() = None;
    }

    // the reassembly's setcell: replace in place when present, append at the
    // end when absent -- never moves an existing cell (setcell_into's own
    // comment: Store's re-top would reorder the dump against python here).
    fn setcell(&mut self, name: &Leaf, contents: V) {
        let k = Self::key(name);
        if let Some(&idx) = self.active.get(&k) {
            let node = self.nodes[idx].as_mut().unwrap();
            node.content = contents;
            node.ncache = RefCell::new(None);
        } else {
            let idx = self.alloc(FSNode {
                name: name.clone(),
                content: contents,
                ncache: RefCell::new(None),
                prev: None,
                next: None,
            });
            self.push_back(idx);
            self.active.insert(k.clone(), idx);
            self.cache_order.push(k);
        }
        *self.whole.borrow_mut() = None;
    }

    fn node_native(&self, idx: usize) -> N {
        let node = self.nodes[idx].as_ref().unwrap();
        {
            let cache = node.ncache.borrow();
            if let Some(n) = cache.as_ref() {
                return n.clone();
            }
        }
        let n = v_to_n(&node.content);
        *node.ncache.borrow_mut() = Some(n.clone());
        n
    }

    fn rebuild_whole(&self) {
        // dump-order triples: nd's own mirror of `d` -- reorders on Store,
        // shadow-inclusive (every physical node, active or inert).
        let mut triples: Vec<N> = Vec::new();
        for idx in self.iter_indices() {
            let (name, n) = {
                let node = self.nodes[idx].as_ref().unwrap();
                (node.name.clone(), self.node_native(idx))
            };
            triples.push(N::S(Rc::new(vec![
                N::A(Rc::new(Leaf::S("CELL".to_string()))),
                N::A(Rc::new(name)),
                n,
            ])));
        }
        // cache-order pairs: ncells' own mirror of `cells` -- stable,
        // append-only, first-match, exactly cache_order's own contract.
        let mut pairs: Vec<(Leaf, N)> = Vec::with_capacity(self.cache_order.len());
        for k in &self.cache_order {
            let idx = self.active[k];
            let node = self.nodes[idx].as_ref().unwrap();
            pairs.push((node.name.clone(), self.node_native(idx)));
        }
        *self.whole.borrow_mut() = Some((Rc::new(pairs), N::S(Rc::new(triples))));
    }

    // the native carrier's view: a monolithic Vec<(Leaf,N)> (DEFS, first match
    // wins) and the whole-store N (the rule-evaluation operand) -- NEval's
    // existing contract, untouched; only rebuilt on the first ask since the
    // last write (any write invalidates both at once, since they are one
    // gather over the same list).
    fn ncells_native(&self) -> Rc<Vec<(Leaf, N)>> {
        if self.whole.borrow().is_none() {
            self.rebuild_whole();
        }
        self.whole.borrow().as_ref().unwrap().0.clone()
    }

    fn nd_native(&self) -> N {
        if self.whole.borrow().is_none() {
            self.rebuild_whole();
        }
        self.whole.borrow().as_ref().unwrap().1.clone()
    }

    // rule-body evaluation, routed through FastStore's own native view (the
    // spec's "eval_rules routes through FastStore" -- neval_rule/eval_rules
    // themselves are untouched; FastStore only supplies their operands).
    fn eval_full(&self, nprocess: &[(String, N)], rid: &Leaf) -> V {
        let ncells = self.ncells_native();
        let nd = self.nd_native();
        neval_rule(&ncells, nprocess, &nd, rid, nd.clone())
    }

    fn eval_delta(&self, nprocess: &[(String, N)], variant: &Leaf, drows_n: Vec<N>) -> V {
        let ncells = self.ncells_native();
        let nd = self.nd_native();
        let operand = N::S(Rc::new(vec![N::S(Rc::new(drows_n)), nd.clone()]));
        neval_rule(&ncells, nprocess, &nd, variant, operand)
    }

    fn eval_rules_many(&self, nprocess: &[(String, N)], rids: &[Leaf]) -> Vec<V> {
        let ncells = self.ncells_native();
        let nd = self.nd_native();
        eval_rules(&ncells, nprocess, &nd, rids)
    }

    // a fresh NEval over the current view, for call sites that build one
    // directly (the reassembly's canon-first partition fallback).
    fn build_neval(&self, process: Vec<(String, N)>) -> NEval {
        let ncells = self.ncells_native();
        let nd = self.nd_native();
        NEval {
            cells: (*ncells).clone(),
            process,
            defs_n: nd,
            fuel: std::cell::Cell::new(-1),
        }
    }

    // EXIT conversion (paid once): every physical cell, active or shadow, head
    // to tail -- cells_to_d's own input shape, so d comes back exactly as
    // raw_cells_of would re-read it.
    fn to_all_cells(&self) -> Vec<(Leaf, V)> {
        self.iter_indices()
            .into_iter()
            .map(|idx| {
                let node = self.nodes[idx].as_ref().unwrap();
                (node.name.clone(), node.content.clone())
            })
            .collect()
    }

    // the active (first-match) view, srv.cells' own contract: stable
    // cache_order (NOT dump order -- see cache_order's own comment; a Store's
    // re-top moves a cell to the front of `d`/`nd` but never moves its slot in
    // `cells`/`ncells`, so this must walk cache_order, not the linked list).
    fn to_active_cells(&self) -> Vec<(Leaf, V)> {
        self.cache_order
            .iter()
            .map(|k| {
                let idx = self.active[k];
                let node = self.nodes[idx].as_ref().unwrap();
                (node.name.clone(), node.content.clone())
            })
            .collect()
    }
}

// eval_rules is engine.py's _eval_rules (line 1188): the UNION of the given
// rules' outputs over the store, deduplicated by row key in first-seen order
// and keeping only sequence rows (Python keeps only tuples). It is the shared
// evaluator for the keyed and sweep passes; each rule id reduces through D's
// own compiled definition exactly as the agg pass reduces one aggregate rule.
// Python's rule twins are a host FAST-path held observationally equal to this
// reduction, so the port takes the reduce path for every rule.
fn eval_rules(ncells: &[(Leaf, N)], nprocess: &[(String, N)], nd: &N, rids: &[Leaf]) -> Vec<V> {
    use std::collections::HashSet;
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<V> = Vec::new();
    for rid in rids {
        // the rule body evaluates on the native carrier over the current store;
        // its operand is the native store, exactly the D the Scott path passed
        let res = neval_rule(ncells, nprocess, nd, rid, nd.clone());
        if let Shape::Seq(l) = shape(&res) {
            for r in items(&l) {
                if matches!(shape(&r), Shape::Seq(_)) && seen.insert(key_of(&r)) {
                    out.push(r);
                }
            }
        }
    }
    out
}

// keyed_key is engine.py's per-head key(r) (line 1250): the row's values at the
// sorted keyspan positions (1-indexed), taking a position only when it falls
// within the row (Python's `if p <= len(r)`). The selected columns key as a
// sequence through key_of, so a produced key and a stored key compare exactly
// as the Python tuples do and never collide across arities. A row that is not
// a sequence is treated as a single column, which real (tuple) cells never hit.
fn keyed_key(row: &V, key_pos: &[usize]) -> String {
    let cols = match shape(row) {
        Shape::Seq(l) => items(&l),
        _ => vec![row.clone()],
    };
    let mut sel: Vec<V> = Vec::new();
    for &p in key_pos {
        if p >= 1 && p <= cols.len() {
            sel.push(cols[p - 1].clone());
        }
    }
    key_of(&seq(from_vec(sel)))
}

// The self-support walk RETIRED here 2026-07-08 (scheduler-in-canon slice
// 2): the sweep/dred split is read from the passHeads cell, and the walk's
// meaning lives in canon (system:cls_selfsup, the WHILE closure over
// system:cls_edges in shared/system.canon), twinned to python's override.

// touched_by is engine.py's _touched (line 1215): with no dirty set (a FULL
// derive's first round) every read set is live; otherwise a read set is live
// when it meets the dirty set or this round's own stores. The keyed and sweep
// passes gate on it exactly as the agg pass does.
fn touched_by<'a>(
    reads: impl IntoIterator<Item = &'a String>,
    dirty: &Option<std::collections::HashSet<String>>,
    round_changed: &std::collections::HashSet<String>,
) -> bool {
    match dirty {
        None => true,
        Some(dd) => reads
            .into_iter()
            .any(|k| dd.contains(k) || round_changed.contains(k)),
    }
}

// classify_heads_native (engine.py:1724 _classify_heads): the joint
// fixpoint's head classification, extracted as ONE shared function so
// op_run_rules' absent-passHeads fallback (below) and scheduler_cells_native
// (#20, the final pipeline slice) materialize the SAME answer instead of two
// independently-maintained copies -- the spec's own instruction: "the
// op_run_rules fallback IS the live classifier; the cell just materializes
// its answer, share the code, do not duplicate." A faithful, self-contained
// twin of python's _classify_heads(D): re-derives aggids/plain rules/agg
// rules/keyspans/reach/kindmap fresh from `cells` on every call, exactly as
// python re-derives them on every _classify_heads(D) call (neither host
// memoizes) -- correct to call from op_run_rules (once per run_rules
// invocation, only when passHeads is absent) and from scheduler_cells_native
// (once per compile, unconditionally, mirroring scheduler_cells' own
// unconditional _classify_heads(D) call).
struct HeadClasses {
    agg: Vec<(String, Leaf)>,
    keyed: Vec<(String, Leaf)>,
    sweep: Vec<(String, Leaf)>,
    dred: Vec<(String, Leaf)>,
    aggwhole: Vec<(String, Leaf)>,
}

fn classify_heads_native(cells: &[(Leaf, V)]) -> HeadClasses {
    use std::collections::{BTreeSet, HashMap, HashSet};
    let leaf = |s: &str| Leaf::S(s.to_string());

    // aggids: rule id key -> membership in ruleAgg (the aggregate rule set)
    let mut aggids: HashSet<String> = HashSet::new();
    for r in pop_rows(cells, &leaf("ruleAgg")) {
        let it = items(&list_of(&r));
        if !it.is_empty() {
            aggids.insert(key_of(&it[0]));
        }
    }
    // ruleDerives rows <rule id, head>, split on aggids -- plain_rows feed
    // plain_of/reach (python's own split); agg_rows feed agg_heads/aggwhole
    struct HRow {
        head: Leaf,
        head_key: String,
        rid_key: String,
    }
    let mut plain_rows: Vec<HRow> = Vec::new();
    let mut agg_rows: Vec<HRow> = Vec::new();
    for r in pop_rows(cells, &leaf("ruleDerives")) {
        let it = items(&list_of(&r));
        if it.len() >= 2 {
            if let (Some(rid), Some(head)) = (aval(&it[0]), aval(&it[1])) {
                let row = HRow {
                    head: (*head).clone(),
                    head_key: key_of(&it[1]),
                    rid_key: key_of(&it[0]),
                };
                if aggids.contains(&row.rid_key) {
                    agg_rows.push(row);
                } else {
                    plain_rows.push(row);
                }
            }
        }
    }
    // plain_of's keys (python: `for h in plain_of`) as head_key -> head Leaf
    // (first occurrence wins; only membership + the Leaf are ever needed
    // here, never the rid list plain_of itself carries)
    let mut head_leaf_of: HashMap<String, Leaf> = HashMap::new();
    for r in &plain_rows {
        head_leaf_of.entry(r.head_key.clone()).or_insert_with(|| r.head.clone());
    }
    let mut agg_head_leaf_of: HashMap<String, Leaf> = HashMap::new();
    for r in &agg_rows {
        agg_head_leaf_of.entry(r.head_key.clone()).or_insert_with(|| r.head.clone());
    }
    // keyspanned: fact type keys carrying a uniqueness/spanning_uniqueness
    // constraint with a non-empty spans row set (constraint rows are <id,
    // kind, ft, ..>; spans rows are <constraint id, position>)
    let mut spans_of: HashMap<String, BTreeSet<i64>> = HashMap::new();
    for r in pop_rows(cells, &leaf("spans")) {
        let it = items(&list_of(&r));
        if it.len() >= 2 {
            if let Some(Leaf::I(p)) = aval(&it[1]).as_deref() {
                spans_of.entry(key_of(&it[0])).or_default().insert(*p);
            }
        }
    }
    let mut keyspanned: HashSet<String> = HashSet::new();
    for c in pop_rows(cells, &leaf("constraint")) {
        let it = items(&list_of(&c));
        if it.len() >= 3 {
            let is_uc = matches!(aval(&it[1]).as_deref(),
                Some(Leaf::S(s)) if s == "uniqueness" || s == "spanning_uniqueness");
            if is_uc {
                if let Some(ps) = spans_of.get(&key_of(&it[0])) {
                    if !ps.is_empty() {
                        keyspanned.insert(key_of(&it[2]));
                    }
                }
            }
        }
    }
    // reads: rule id key -> its ruleReads cell-key set
    let mut reads: HashMap<String, HashSet<String>> = HashMap::new();
    for r in pop_rows(cells, &leaf("ruleReads")) {
        let it = items(&list_of(&r));
        if it.len() >= 2 {
            reads.entry(key_of(&it[0])).or_default().insert(key_of(&it[1]));
        }
    }
    // reach: plain head_key -> union of its (plain) rules' reads, exactly
    // python's reach = {h: {...} for h, rids in plain_of.items()} (agg rules
    // never contribute -- plain_of never held them)
    let mut reach: HashMap<String, HashSet<String>> = HashMap::new();
    for r in &plain_rows {
        let e = reach.entry(r.head_key.clone()).or_default();
        if let Some(rs) = reads.get(&r.rid_key) {
            e.extend(rs.iter().cloned());
        }
    }
    // kindmap: fact type key -> derivation kind, LAST ROW WINS (python's
    // dict comprehension over _pop_rows(D,"derivation") in pop order)
    let mut kindmap: HashMap<String, String> = HashMap::new();
    for r in pop_rows(cells, &leaf("derivation")) {
        let it = items(&list_of(&r));
        if it.len() >= 2 {
            if let Some(k) = aval(&it[1]) {
                kindmap.insert(key_of(&it[0]), leaf_text(&k));
            }
        }
    }
    let owned = |hk: &String| {
        matches!(kindmap.get(hk).map(|s| s.as_str()),
            Some("fully-derived") | Some("derived-and-stored"))
    };
    let agg_head_keys: HashSet<String> = agg_rows.iter().map(|r| r.head_key.clone()).collect();
    let derived_heads: HashSet<String> = agg_head_keys
        .iter()
        .cloned()
        .chain(head_leaf_of.keys().cloned())
        .collect();
    let self_supporting = |h: &String| -> bool {
        let mut seen: HashSet<String> = HashSet::new();
        let mut stack: Vec<String> = reach
            .get(h)
            .map(|rs| rs.iter().filter(|x| derived_heads.contains(*x)).cloned().collect())
            .unwrap_or_default();
        while let Some(x) = stack.pop() {
            if &x == h {
                return true;
            }
            if !seen.insert(x.clone()) {
                continue;
            }
            if let Some(rx) = reach.get(&x) {
                stack.extend(rx.iter().filter(|y| derived_heads.contains(*y)).cloned());
            }
        }
        false
    };
    let mut agg: Vec<(String, Leaf)> =
        agg_head_leaf_of.iter().map(|(k, l)| (k.clone(), l.clone())).collect();
    let mut keyed: Vec<(String, Leaf)> = Vec::new();
    let mut sweep: Vec<(String, Leaf)> = Vec::new();
    let mut dred: Vec<(String, Leaf)> = Vec::new();
    for (hk, hl) in &head_leaf_of {
        if keyspanned.contains(hk) {
            // python: 'keyed' = plain-ruled heads with a key span; owned
            // EXCLUDES keyspanned (h not in keyspanned), so the else-if
            // chain matches _classify_heads exactly
            keyed.push((hk.clone(), hl.clone()));
        } else if owned(hk) && !agg_head_keys.contains(hk) {
            if self_supporting(hk) {
                dred.push((hk.clone(), hl.clone()));
            } else {
                sweep.push((hk.clone(), hl.clone()));
            }
        }
    }
    let mut aggwhole: Vec<(String, Leaf)> = agg_head_leaf_of
        .iter()
        .filter(|(hk, _)| owned(hk))
        .map(|(k, l)| (k.clone(), l.clone()))
        .collect();
    let by_text = |a: &(String, Leaf), b: &(String, Leaf)| leaf_text(&a.1).cmp(&leaf_text(&b.1));
    agg.sort_by(by_text);
    keyed.sort_by(by_text);
    sweep.sort_by(by_text);
    dred.sort_by(by_text);
    aggwhole.sort_by(by_text);
    HeadClasses { agg, keyed, sweep, dred, aggwhole }
}

fn op_run_rules(j: &J, srv: &mut Srv) -> Result<String, String> {
    use std::collections::{BTreeSet, HashMap, HashSet};
    // the optional frontier: an array of cell names bounding ROUND ONE to
    // the rules whose ruleReads intersect it (Python's changed argument);
    // absent means a full round one. It parses before anything runs so a
    // malformed request mutates nothing.
    let frontier: Option<HashSet<String>> = match jget(j, "changed") {
        None => None,
        Some(J::A(xs)) => {
            let mut set: HashSet<String> = HashSet::new();
            for x in xs {
                match scalar_atom(x) {
                    Some(a) => {
                        set.insert(key_of(&a));
                    }
                    None => {
                        return Err(
                            "run_rules changed must be an array of scalar cell names"
                                .to_string(),
                        )
                    }
                }
            }
            Some(set)
        }
        Some(_) => {
            return Err(
                "run_rules changed must be an array of scalar cell names".to_string(),
            )
        }
    };
    // FastStore (#35, the store twin, docs/2026-07-11-store-twin-spec.md): ONE
    // store for the round loop, built once at entry (raw_cells_of, duplicates
    // preserved) and converted back once at exit. Every rule body still
    // evaluates through NEval/the native carrier -- FastStore only supplies its
    // ncells/nd operands (eval_full/eval_delta/eval_rules_many/build_neval), so
    // the reduction semantics are byte-for-byte what they were; only the
    // bookkeeping around them changed representation.
    let mut store = FastStore::from_srv(srv);
    let nprocess = srv.nprocess.clone();
    let mut changed: BTreeSet<String> = BTreeSet::new();
    let leaf = |s: &str| Leaf::S(s.to_string());
    // reads: rule id key to the set of cell keys its body reads (ruleReads
    // rows are ⟨rule id, read cell⟩). The mirror blocks quantify over all
    // rules through it, round one intersects it with the frontier, and the
    // later rounds' full fallback intersects it with the delta.
    let mut reads: HashMap<String, HashSet<String>> = HashMap::new();
    for r in store.pop_rows(&leaf("ruleReads")) {
        let it = items(&list_of(&r));
        if it.len() >= 2 {
            reads.entry(key_of(&it[0])).or_default().insert(key_of(&it[1]));
        }
    }
    // The mirror blocks run BEFORE the loop and ignore the frontier, exactly
    // as Python's run before its frontier is even consulted.
    let any_reads = |target: &str| {
        let k = key_of(&atom(Leaf::S(target.to_string())));
        reads.values().any(|rs| rs.contains(&k))
    };
    // THE INSTANCE MIRROR (engine.py proposal B): when any rule reads
    // Resource_is_instance_of_Noun and that cell is EMPTY, derive it fresh
    // from the role facts and the noun kinds: every id playing one of a
    // noun's roles is an instance of that noun. Asserted rows win; the
    // mirror serves only the empty cell.
    const MIRROR: &str = "Resource_is_instance_of_Noun";
    if any_reads(MIRROR) {
        let mut nouns: HashSet<String> = HashSet::new();
        for r in store.pop_rows(&leaf("instanceOf")) {
            let it = items(&list_of(&r));
            if it.len() >= 2
                && matches!(aval(&it[1]).as_deref(), Some(Leaf::S(s)) if s == "ObjectType")
            {
                nouns.insert(key_of(&it[0]));
            }
        }
        // group the role rows ⟨role id, fact type, position, player⟩ by
        // fact type, in first-appearance order like the Python dict; a
        // position outside the int leaves its row out (Python would have
        // faulted on it, and the resident must not)
        let mut order: Vec<V> = Vec::new();
        let mut groups: HashMap<String, Vec<(usize, V)>> = HashMap::new();
        for r in store.pop_rows(&leaf("role")) {
            let it = items(&list_of(&r));
            if it.len() >= 4 {
                if let Some(Leaf::I(p)) = aval(&it[2]).as_deref() {
                    if *p >= 1 {
                        let k = key_of(&it[1]);
                        if !groups.contains_key(&k) {
                            order.push(it[1].clone());
                        }
                        groups.entry(k).or_default().push((*p as usize, it[3].clone()));
                    }
                }
            }
        }
        let mut out: Vec<V> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for ft in &order {
            let grp = &groups[&key_of(ft)];
            let mut ft_rows: Option<Vec<V>> = None;
            for (p, player) in grp {
                if nouns.contains(&key_of(player)) {
                    if ft_rows.is_none() {
                        // the fact type name addresses its own cell
                        let name = match aval(ft) {
                            Some(l) => (*l).clone(),
                            None => continue,
                        };
                        ft_rows = Some(store.pop_rows(&name));
                    }
                    for row in ft_rows.as_ref().unwrap() {
                        let rit = items(&list_of(row));
                        if rit.len() >= *p {
                            let pair =
                                seq(from_vec(vec![rit[*p - 1].clone(), player.clone()]));
                            if seen.insert(key_of(&pair)) {
                                out.push(pair);
                            }
                        }
                    }
                }
            }
        }
        if !out.is_empty() && store.pop_rows(&leaf(MIRROR)).is_empty() {
            sort_rows(&mut out);
            store.store(&leaf(MIRROR), seq(from_vec(out)));
            changed.insert(MIRROR.to_string());
        }
    }
    // THE ROLE MIRROR: Fact_Type_has_Role derives from the role M-facts the
    // same way (the role facts ARE the knowledge); only the empty cell fills.
    const FTR: &str = "Fact_Type_has_Role";
    if any_reads(FTR) {
        let mut out: Vec<V> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for r in store.pop_rows(&leaf("role")) {
            let it = items(&list_of(&r));
            if it.len() >= 2 {
                let pair = seq(from_vec(vec![it[1].clone(), it[0].clone()]));
                if seen.insert(key_of(&pair)) {
                    out.push(pair);
                }
            }
        }
        if !out.is_empty() && store.pop_rows(&leaf(FTR)).is_empty() {
            sort_rows(&mut out);
            store.store(&leaf(FTR), seq(from_vec(out)));
            changed.insert(FTR.to_string());
        }
    }
    // atomsof: rule id key to its body atoms as ⟨position text, atom cell
    // key⟩ in ruleAtom row order; the stored delta variant for an atom rides
    // the DEFS cell named "<rule id>~d<position>", exactly the name Python
    // formats
    let mut atomsof: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for r in store.pop_rows(&leaf("ruleAtom")) {
        let it = items(&list_of(&r));
        if it.len() >= 3 {
            if let Some(p) = aval(&it[1]) {
                atomsof
                    .entry(key_of(&it[0]))
                    .or_default()
                    .push((leaf_text(&p), key_of(&it[2])));
            }
        }
    }
    // the rule table: ruleDerives rows ⟨rule id, head cell⟩ in cell order,
    // split on ruleAgg into the closure's plain rules and the AGGREGATE
    // rules the upper stratum runs after the closure settles (an aggregate
    // head supersedes instead of unioning, so the closure must never run
    // one)
    let mut aggids: HashSet<String> = HashSet::new();
    for r in store.pop_rows(&leaf("ruleAgg")) {
        let it = items(&list_of(&r));
        if !it.is_empty() {
            aggids.insert(key_of(&it[0]));
        }
    }
    struct RuleRow {
        rid: Leaf,
        head: Leaf,
        key: String,
        head_key: String,
    }
    let mut rules: Vec<RuleRow> = Vec::new();
    let mut agg_rules: Vec<RuleRow> = Vec::new();
    for r in store.pop_rows(&leaf("ruleDerives")) {
        let it = items(&list_of(&r));
        if it.len() >= 2 {
            if let (Some(rid), Some(head)) = (aval(&it[0]), aval(&it[1])) {
                let row = RuleRow {
                    rid: (*rid).clone(),
                    head: (*head).clone(),
                    key: key_of(&it[0]),
                    head_key: key_of(&it[1]),
                };
                if aggids.contains(&row.key) {
                    agg_rules.push(row);
                } else {
                    rules.push(row);
                }
            }
        }
    }
    // the semi-naive loop, Python's delta threading exactly: round one runs
    // full bodies (bounded by the frontier when given); each later round
    // sees the PREVIOUS round's per-head delta, and a rule whose atoms
    // intersect it joins each stored ~d variant over the pair of that atom's
    // sorted delta rows and D, while a rule without atom facts re-runs whole
    // when its reads changed. A head stored mid-round is visible to the next
    // rule, exactly as Python threads D through its round. Only genuinely
    // new rows fire, and the round's additions become the next delta.
    let mut rounds: i64 = 0;
    let mut delta: Option<HashMap<String, Vec<V>>> = None;
    // closure_keys collects the closure's changed head keys, feeding the
    // upper stratum's dirty set exactly as Python's closure_changed does
    let mut closure_keys: HashSet<String> = HashSet::new();
    loop {
        rounds += 1;
        let mut fired = false;
        let mut next_delta: HashMap<String, Vec<V>> = HashMap::new();
        for rr in &rules {
            let full = |store: &FastStore, rid: &Leaf| -> Option<Vec<V>> {
                let res = store.eval_full(&nprocess, rid);
                match shape(&res) {
                    Shape::Seq(l) => Some(items(&l)),
                    // the rule is not compiled (M-facts only) or bottomed
                    _ => None,
                }
            };
            let cand: Vec<V> = match &delta {
                None => {
                    // round one: the full body, bounded by the frontier
                    if let Some(fr) = &frontier {
                        let hit = reads
                            .get(&rr.key)
                            .map_or(false, |rs| rs.iter().any(|k| fr.contains(k)));
                        if !hit {
                            continue;
                        }
                    }
                    match full(&store, &rr.rid) {
                        Some(c) => c,
                        None => continue,
                    }
                }
                Some(dl) => {
                    let hits: Vec<(String, String)> = match atomsof.get(&rr.key) {
                        Some(av) => av
                            .iter()
                            .filter(|(_p, ftk)| dl.contains_key(ftk))
                            .cloned()
                            .collect(),
                        None => Vec::new(),
                    };
                    if !hits.is_empty() {
                        // the semi-naive inner join: each hit's ~d variant
                        // applied to ⟨sorted delta rows, D⟩; a variant that
                        // is missing or bottoms contributes nothing
                        let mut c: Vec<V> = Vec::new();
                        for (p, ftk) in &hits {
                            let mut drows = dl[ftk].clone();
                            sort_rows(&mut drows);
                            let vid =
                                Leaf::S(format!("{}~d{}", leaf_text(&rr.rid), p));
                            // the native operand mirrors the V pair ⟨sorted delta
                            // rows, D⟩ exactly: a two-element SEQ of the delta rows
                            // as N and the maintained native store, built directly
                            // so D is not re-converted each hit. seq never collapses
                            // on bottom, so the rows use N::S, not nseq. Routed
                            // through FastStore's own eval_delta -- same
                            // neval_rule call, same operand shape, the store just
                            // supplies ncells/nd from its own cache now.
                            let drows_n: Vec<N> = drows.iter().map(v_to_n).collect();
                            let res = store.eval_delta(&nprocess, &vid, drows_n);
                            if let Shape::Seq(l) = shape(&res) {
                                c.extend(items(&l));
                            }
                        }
                        c
                    } else if !atomsof.contains_key(&rr.key)
                        && reads
                            .get(&rr.key)
                            .map_or(false, |rs| rs.iter().any(|k| dl.contains_key(k)))
                    {
                        // a rule without atom facts falls back to its full
                        // body in rounds where its reads changed
                        match full(&store, &rr.rid) {
                            Some(c) => c,
                            None => continue,
                        }
                    } else if !atomsof.contains_key(&rr.key)
                        && !reads.contains_key(&rr.key)
                    {
                        // Conservative widening beyond Python for HAND-BUILT
                        // stores: a rule carrying neither atom facts nor read
                        // facts has an unknown read set, so it re-evaluates
                        // fully rather than going dormant after round one.
                        // The compiler records ruleReads for every rule it
                        // compiles, so on compiled stores this branch never
                        // fires and the loop is exactly Python's.
                        match full(&store, &rr.rid) {
                            Some(c) => c,
                            None => continue,
                        }
                    } else {
                        continue;
                    }
                }
            };
            let old = store.pop_rows(&rr.head);
            let mut merged: Vec<V> = Vec::new();
            let mut keys: HashSet<String> = HashSet::new();
            for r in &old {
                if keys.insert(key_of(r)) {
                    merged.push(r.clone());
                }
            }
            let mut added: Vec<V> = Vec::new();
            for r in cand {
                // only sequence rows count, as Python keeps only tuples
                if !matches!(shape(&r), Shape::Seq(_)) {
                    continue;
                }
                if keys.insert(key_of(&r)) {
                    merged.push(r.clone());
                    added.push(r);
                }
            }
            if !added.is_empty() {
                sort_rows(&mut merged);
                store.store(&rr.head, seq(from_vec(merged)));
                fired = true;
                changed.insert(leaf_text(&rr.head));
                closure_keys.insert(rr.head_key.clone());
                next_delta
                    .entry(rr.head_key.clone())
                    .or_default()
                    .extend(added);
            }
        }
        if !fired {
            break;
        }
        delta = Some(next_delta);
    }
    // ---- THE UPPER STRATA, iterated to a JOINT fixpoint (engine.py lines
    // 1210 through 1288): three passes share one outer loop above the
    // positive closure, because each can invalidate the others' work through
    // the dependency graph (loads settle, ranks rederive over them, the peak
    // refolds over ranks), so they repeat until a full sweep changes nothing
    // (at most twelve rounds, as Python bounds it):
    //
    //   agg   — each aggregate rule evaluates its FULL body over the current
    //           D; its head then SUPERSEDES rather than unions. A
    //           derivation-OWNED head (kind_owned: NORMA's * and ** — kinds
    //           no user asserts into) on a FULL derive is REPLACED whole by the
    //           agg rows unioned with its plain rules' rows, so a group whose
    //           supply vanished dies (per-group supersession could never
    //           retire it) and the paired plain rows of the zero-supply idiom
    //           rejoin fresh; otherwise the head supersedes PER GROUP (the
    //           group is every column but the last; a stored row whose group
    //           no rule produced survives);
    //   keyed — a head whose fact type carries a key span re-evaluates over
    //           the settled store and supersedes PER KEY: a produced key
    //           replaces its stored row, an asserted row whose key no rule
    //           produced survives (task-955 upsert);
    //   sweep — a derivation-OWNED plain head is materialization, never ground
    //           truth (Gupta-Mumick-Subrahmanian 1993, delete-and-rederive).
    //           A non-self-supporting head re-evaluates whole and REPLACES,
    //           retiring staleness the monotone closure's union can never
    //           remove; a self-supporting head EMPTIES first, then rederives
    //           to a local least fixpoint, so rows with only cyclic support
    //           die while base-supported rows rebuild.
    //
    // Dirty-set filtering keeps incremental calls proportional: round zero of
    // a full derive evaluates every eligible head once (the idempotence
    // guarantee), while a frontier call touches only heads whose reads meet
    // the frontier, the closure's changes, or this round's own stores.
    // THE SCHEDULE IS STORE KNOWLEDGE (scheduler-in-canon slice 2): pass
    // membership comes from the passHeads cell — system:classify_heads'
    // materialization, written by every compile beside rmapColumns — read
    // here exactly as rmapColumns is read below, never reclassified. Rows
    // are ⟨pass, head⟩ with pass ∈ {agg, keyed, sweep, dred, aggwhole};
    // a store without the cell runs its positive closure but maintains no
    // destructive pass (the same posture as a store without rmapColumns
    // reading as all-own-table). The retired reclassification (kindmap +
    // kind_owned + the self-support walk) lives on as canon: the cls_*
    // family in shared/system.canon, twinned to python's override.
    let mut pass_sweep: Vec<String> = Vec::new();
    let mut pass_dred: Vec<String> = Vec::new();
    let mut pass_keyed: HashSet<String> = HashSet::new();
    let mut pass_aggwhole: HashSet<String> = HashSet::new();
    // the CELL-PRESENCE test (#20, the replay slice), distinct from
    // row-emptiness: an ABSENT passHeads cell means this store never met
    // scheduler_cells (protocol.py:1849 — a later phase than any this op's
    // callers run), and python's contract for that store is CLASSIFY LIVE
    // (engine.py:1396 calls _classify_heads unconditionally;
    // scheduler_cells' own docstring: "a store without it classifies at
    // run time, which is what run_rules does anyway") — see the fallback
    // block after `reach` below. A PRESENT-but-empty cell is an
    // explicitly-materialized empty schedule and is honored as before.
    let has_passheads_cell = store.has_cell(&leaf("passHeads"));
    for r in store.pop_rows(&leaf("passHeads")) {
        let it = items(&list_of(&r));
        if it.len() >= 2 {
            let p = match aval(&it[0]).as_deref() {
                Some(Leaf::S(s)) => s.clone(),
                _ => continue,
            };
            let h = key_of(&it[1]);
            match p.as_str() {
                "sweep" => pass_sweep.push(h),
                "dred" => pass_dred.push(h),
                "keyed" => {
                    pass_keyed.insert(h);
                }
                "aggwhole" => {
                    pass_aggwhole.insert(h);
                }
                _ => {}
            }
        }
    }
    let mut plain_of: HashMap<String, Vec<Leaf>> = HashMap::new();
    // head_leaf_of recovers a plain head's cell name (a Leaf) from its key,
    // for the keyed and sweep passes that address cells by name.
    let mut head_leaf_of: HashMap<String, Leaf> = HashMap::new();
    for rr in &rules {
        plain_of
            .entry(rr.head_key.clone())
            .or_default()
            .push(rr.rid.clone());
        head_leaf_of
            .entry(rr.head_key.clone())
            .or_insert_with(|| rr.head.clone());
    }
    // spans_of: constraint id key to its role-position set (spans rows are
    // ⟨constraint id, position⟩). A BTreeSet keeps positions sorted, so the
    // keyed key reads columns in Python's sorted(keyspans[head]) order.
    let mut spans_of: HashMap<String, BTreeSet<i64>> = HashMap::new();
    for r in store.pop_rows(&leaf("spans")) {
        let it = items(&list_of(&r));
        if it.len() >= 2 {
            if let Some(Leaf::I(p)) = aval(&it[1]).as_deref() {
                spans_of.entry(key_of(&it[0])).or_default().insert(*p);
            }
        }
    }
    // keyspans: fact type key to the union of the spans of its uniqueness /
    // spanning_uniqueness constraints (constraint rows are ⟨constraint id,
    // kind, fact type, ..⟩). A fact type with a key span is a keyed head.
    let mut keyspans: HashMap<String, BTreeSet<i64>> = HashMap::new();
    for c in store.pop_rows(&leaf("constraint")) {
        let it = items(&list_of(&c));
        if it.len() >= 3 {
            let is_uc = matches!(aval(&it[1]).as_deref(),
                Some(Leaf::S(s)) if s == "uniqueness" || s == "spanning_uniqueness");
            if is_uc {
                if let Some(ps) = spans_of.get(&key_of(&it[0])) {
                    if !ps.is_empty() {
                        keyspans
                            .entry(key_of(&it[2]))
                            .or_default()
                            .extend(ps.iter().copied());
                    }
                }
            }
        }
    }
    // keyed_of: the plain rules whose head fact type carries a key span,
    // grouped by head, each with the rule's reads key for gating.
    let mut keyed_of: HashMap<String, Vec<(Leaf, String)>> = HashMap::new();
    for rr in &rules {
        if keyspans.contains_key(&rr.head_key) {
            keyed_of
                .entry(rr.head_key.clone())
                .or_default()
                .push((rr.rid.clone(), rr.key.clone()));
        }
    }
    // reach: a plain head to the union of its rules' read cells, the
    // dependency graph the self-support test and the sweep gate walk.
    let mut reach: HashMap<String, HashSet<String>> = HashMap::new();
    for rr in &rules {
        let e = reach.entry(rr.head_key.clone()).or_default();
        if let Some(rs) = reads.get(&rr.key) {
            e.extend(rs.iter().cloned());
        }
    }
    // ==================== THE ABSENT-CELL FALLBACK (#20, replay slice) ====
    // python's run_rules NEVER reads passHeads — it calls _classify_heads(D)
    // LIVE on every invocation (engine.py:1396) and scheduler_cells merely
    // materializes that same classification for readers (engine.py:1797,
    // whose own docstring states the absent-cell contract: "a store without
    // it classifies at run time, which is what run_rules does anyway").
    // This op's prior degrade — NO destructive pass at all — was
    // unobservable before replay: no earlier boundary ever produced a store
    // where a sweep RETIRED anything or an aggregate-reading head needed
    // the sweep's second evaluation (every certified corpus byte-matched
    // python WITH python sweeping, i.e. those sweeps were no-ops — which is
    // also this fallback's safety proof for the certified corpora). Replay
    // created the first counterexamples: the rp-cascade staged probe's
    // s2/s3 stages diverged on python-identical input bytes, and the tasks
    // corpus's recommendation-cascade family reproduced it at scale
    // (rp-REPORT.md). This is the SECOND instance of the schedule-as-data
    // absent-cell class — the first was rmapColumns in the reassembly,
    // fixed with the canon-first partition fallback; the contract both
    // times: the cell is an optimization, absent means compute live.
    //
    // The twin mirrors _classify_heads (engine.py:1724) FAITHFULLY over
    // locals already built above (agg heads from agg_rules, plain_of,
    // keyspans, reach); fresh reads: the derivation cell (kindmap,
    // last-row-wins like python's dict comprehension), the _OWNED
    // storage-kind filter (ONLY "fully-derived"/"derived-and-stored" join
    // the destructive passes — +/++/unmarked ruled heads keep asserted
    // rows and MUST stay out, per the compiler's own over-marking warning),
    // and the self-support DFS (reach edges restricted to derived heads;
    // a head reachable from itself is 'dred', GMS93's recursive form).
    // Stores WITH the cell never enter this block — byte-unchanged path.
    if !has_passheads_cell {
        // classify_heads_native (just above) is the extracted twin of this
        // block's former inline computation -- SHARED now with
        // scheduler_cells_native (#20) rather than duplicated. Behavior
        // unchanged: same locals this function already builds are re-derived
        // identically inside the shared function; only the ORDER of
        // pass_sweep/pass_dred differs transiently (classify_heads_native
        // sorts by head text), which is moot since the sweep/sweep_cyclic
        // construction just below re-sorts by leaf text anyway.
        let hc = classify_heads_native(&store.to_active_cells());
        for (hk, _) in &hc.keyed {
            pass_keyed.insert(hk.clone());
        }
        for (hk, _) in &hc.sweep {
            pass_sweep.push(hk.clone());
        }
        for (hk, _) in &hc.dred {
            pass_dred.push(hk.clone());
        }
        for (hk, _) in &hc.aggwhole {
            pass_aggwhole.insert(hk.clone());
        }
    }
    // sweep / sweep_cyclic straight from the cell's lists: a listed head
    // with no rules in THIS store resolves no leaf and is skipped. The
    // sorts are defensive against hand stores — the compiled cell already
    // rides in head-name order per pass.
    let mut sweep: Vec<(String, Leaf)> = Vec::new();
    let mut sweep_cyclic: Vec<(String, Leaf)> = Vec::new();
    for hk in &pass_sweep {
        if let Some(hl) = head_leaf_of.get(hk) {
            sweep.push((hk.clone(), hl.clone()));
        }
    }
    for hk in &pass_dred {
        if let Some(hl) = head_leaf_of.get(hk) {
            sweep_cyclic.push((hk.clone(), hl.clone()));
        }
    }
    sweep.sort_by(|a, b| leaf_text(&a.1).cmp(&leaf_text(&b.1)));
    sweep_cyclic.sort_by(|a, b| leaf_text(&a.1).cmp(&leaf_text(&b.1)));
    // keyed_sorted: the keyed heads in head-name order, each with its head
    // leaf, its rules (rid and reads key), and its sorted key positions.
    let mut keyed_sorted: Vec<(String, Leaf, Vec<(Leaf, String)>, Vec<usize>)> =
        Vec::new();
    for (hk, rls) in &keyed_of {
        if !pass_keyed.contains(hk) {
            continue;
        }
        let hl = match head_leaf_of.get(hk) {
            Some(l) => l.clone(),
            None => continue,
        };
        let key_pos: Vec<usize> = keyspans
            .get(hk)
            .map(|s| s.iter().filter(|&&p| p >= 1).map(|&p| p as usize).collect())
            .unwrap_or_default();
        keyed_sorted.push((hk.clone(), hl, rls.clone(), key_pos));
    }
    keyed_sorted.sort_by(|a, b| leaf_text(&a.1).cmp(&leaf_text(&b.1)));
    let mut dirty: Option<HashSet<String>> = frontier
        .as_ref()
        .map(|fr| fr.union(&closure_keys).cloned().collect());
    // THE ORDER AND THE ROUND BOUND ARE STORE KNOWLEDGE (the passOrder /
    // passBound cells, system:pass_order / system:pass_bound materialized
    // — the same posture as passHeads): the joint loop DISPATCHES its
    // native pass bodies by the cell's sequence and falls back to the
    // doctrine literals when a store lacks the cells. Unknown pass names
    // skip (forward compatibility).
    let mut pass_order: Vec<(i64, String)> = Vec::new();
    for r in store.pop_rows(&leaf("passOrder")) {
        let it = items(&list_of(&r));
        if it.len() >= 2 {
            // the pass NAME must extract as the bare string (key_of would
            // quote it and no dispatch arm would ever match — the store's
            // whole schedule silently skipping was the 2026-07-08 bug the
            // minted-cell differential caught)
            if let (Some(li), Some(ln)) = (aval(&it[0]), aval(&it[1])) {
                if let (Leaf::I(i), Leaf::S(s)) = (&*li, &*ln) {
                    pass_order.push((*i, s.clone()));
                }
            }
        }
    }
    pass_order.sort();
    let order: Vec<String> = if pass_order.is_empty() {
        ["agg", "keyed", "sweep", "dred"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    } else {
        pass_order.into_iter().map(|(_i, p)| p).collect()
    };
    let mut bound: i64 = 12;
    for r in store.pop_rows(&leaf("passBound")) {
        let it = items(&list_of(&r));
        if !it.is_empty() {
            if let Some(Leaf::I(n)) = aval(&it[0]).as_deref() {
                bound = *n;
            }
        }
    }
    for outer in 0..bound {
        let mut settled = true;
        let mut round_changed: HashSet<String> = HashSet::new();
        for p in &order {
            match p.as_str() {
                "agg" => {
        for rr in &agg_rules {
            // the gate: round zero of a full derive evaluates everything;
            // otherwise only rules whose reads touch the dirty set or this
            // round's own changes fire
            if outer > 0 || dirty.is_some() {
                let touched = reads.get(&rr.key).map_or(false, |rs| {
                    rs.iter().any(|k| {
                        dirty.as_ref().map_or(false, |dd| dd.contains(k))
                            || round_changed.contains(k)
                    })
                });
                if !touched {
                    continue;
                }
            }
            let res = store.eval_full(&nprocess, &rr.rid);
            let outs = match shape(&res) {
                Shape::Seq(l) => items(&l),
                // an uncompiled aggregate (M-facts only) stores nothing
                _ => continue,
            };
            let mut merged: Vec<V> = Vec::new();
            let mut mkeys: HashSet<String> = HashSet::new();
            for r in &outs {
                if matches!(shape(r), Shape::Seq(_)) && mkeys.insert(key_of(r)) {
                    merged.push(r.clone());
                }
            }
            let before = store.pop_rows(&rr.head);
            let mut before_keys: HashSet<String> = HashSet::new();
            let mut before_rows: Vec<V> = Vec::new();
            for r in &before {
                if before_keys.insert(key_of(r)) {
                    before_rows.push(r.clone());
                }
            }
            if pass_aggwhole.contains(&rr.head_key) && dirty.is_none() {
                // whole-replace: the agg rows plus the head's plain rules'
                // rows ARE the cell; nothing older survives
                for rid in plain_of.get(&rr.head_key).map(|v| v.as_slice()).unwrap_or(&[])
                {
                    let res = store.eval_full(&nprocess, rid);
                    if let Shape::Seq(l) = shape(&res) {
                        for r in items(&l) {
                            if matches!(shape(&r), Shape::Seq(_))
                                && mkeys.insert(key_of(&r))
                            {
                                merged.push(r);
                            }
                        }
                    }
                }
            } else {
                // per-group supersession: the group is every column but the
                // last; stored rows whose group no rule produced survive
                let groups: HashSet<String> = merged.iter().map(group_key).collect();
                for r in &before_rows {
                    if !groups.contains(&group_key(r)) && mkeys.insert(key_of(r)) {
                        merged.push(r.clone());
                    }
                }
            }
            let same = mkeys.len() == before_keys.len()
                && mkeys.iter().all(|k| before_keys.contains(k));
            if !same {
                settled = false;
                round_changed.insert(rr.head_key.clone());
                changed.insert(leaf_text(&rr.head));
                sort_rows(&mut merged);
                store.store(&rr.head, seq(from_vec(merged)));
            }
        }
                }
                "keyed" => {
        // ---- THE KEYED-UPSERT PASS (engine.py lines 1243 through 1260):
        // each keyed head, in head-name order, re-evaluates over the settled
        // store and supersedes PER KEY. Gated like the agg pass on the union
        // of its rules' reads. The produced union keeps every produced row;
        // stored rows are kept only when their key is not among the produced
        // keys, so a produced key replaces its stored row while an asserted
        // row whose key no rule produced survives.
        for (hk, hl, rls, key_pos) in &keyed_sorted {
            if outer > 0 || dirty.is_some() {
                let live = touched_by(
                    rls.iter()
                        .flat_map(|(_, rk)| reads.get(rk).into_iter().flatten()),
                    &dirty,
                    &round_changed,
                );
                if !live {
                    continue;
                }
            }
            let rids: Vec<Leaf> = rls.iter().map(|(rid, _)| rid.clone()).collect();
            let outs = store.eval_rules_many(&nprocess, &rids);
            let mut prod_keys: HashSet<String> = HashSet::new();
            for r in &outs {
                prod_keys.insert(keyed_key(r, key_pos));
            }
            let stored = store.pop_rows(hl);
            let mut merged: Vec<V> = Vec::new();
            let mut mkeys: HashSet<String> = HashSet::new();
            for r in &outs {
                if mkeys.insert(key_of(r)) {
                    merged.push(r.clone());
                }
            }
            for r in &stored {
                if !prod_keys.contains(&keyed_key(r, key_pos)) && mkeys.insert(key_of(r))
                {
                    merged.push(r.clone());
                }
            }
            let mut cur_keys: HashSet<String> = HashSet::new();
            for r in &stored {
                cur_keys.insert(key_of(r));
            }
            let same = mkeys.len() == cur_keys.len()
                && mkeys.iter().all(|k| cur_keys.contains(k));
            if !same {
                settled = false;
                round_changed.insert(hk.clone());
                changed.insert(leaf_text(hl));
                sort_rows(&mut merged);
                store.store(hl, seq(from_vec(merged)));
            }
        }
                }
                "sweep" => {
        // ---- THE SWEEP PASS (engine.py lines 1261 through 1269): a
        // derivation-owned, non-self-supporting plain head re-evaluates whole
        // and REPLACES, so this call's supersessions propagate and staleness
        // the closure's union could never remove converges. Always gated on
        // _touched(reach), which a full derive makes true for every head.
        for (hk, hl) in &sweep {
            let empty = HashSet::new();
            let rs = reach.get(hk).unwrap_or(&empty);
            if !touched_by(rs.iter(), &dirty, &round_changed) {
                continue;
            }
            let rids = plain_of.get(hk).map(|v| v.as_slice()).unwrap_or(&[]);
            let outs = store.eval_rules_many(&nprocess, rids);
            let stored = store.pop_rows(hl);
            let mut oks: HashSet<String> = HashSet::new();
            for r in &outs {
                oks.insert(key_of(r));
            }
            let mut sks: HashSet<String> = HashSet::new();
            for r in &stored {
                sks.insert(key_of(r));
            }
            let same = oks.len() == sks.len() && oks.iter().all(|k| sks.contains(k));
            if !same {
                settled = false;
                round_changed.insert(hk.clone());
                changed.insert(leaf_text(hl));
                let mut m = outs;
                sort_rows(&mut m);
                store.store(hl, seq(from_vec(m)));
            }
        }
                }
                "dred" => {
        // ---- THE DRED SWEEP FOR CYCLES (engine.py lines 1270 through 1284):
        // a self-supporting head EMPTIES first, then rederives to a LOCAL
        // least fixpoint over the store with the emptied head, repeatedly
        // evaluating its plain rules until the output stops growing. Rows with
        // only cyclic support die; base-supported rows rebuild. The
        // emptied-then-refilled head commits unconditionally (Python's
        // D = Dx); only a net change to the row set marks the head changed.
        for (hk, hl) in &sweep_cyclic {
            let empty = HashSet::new();
            let rs = reach.get(hk).unwrap_or(&empty);
            if !touched_by(rs.iter(), &dirty, &round_changed) {
                continue;
            }
            let stored = store.pop_rows(hl);
            let mut cur_keys: HashSet<String> = HashSet::new();
            for r in &stored {
                cur_keys.insert(key_of(r));
            }
            store.store(hl, seq(from_vec(Vec::new())));
            let rids: Vec<Leaf> = plain_of.get(hk).cloned().unwrap_or_default();
            let mut prev: Option<HashSet<String>> = None;
            let mut outs_keys: HashSet<String> = HashSet::new();
            loop {
                if prev.as_ref() == Some(&outs_keys) {
                    break;
                }
                prev = Some(outs_keys.clone());
                let outs = store.eval_rules_many(&nprocess, &rids);
                outs_keys = outs.iter().map(|r| key_of(r)).collect();
                let mut m = outs;
                sort_rows(&mut m);
                store.store(hl, seq(from_vec(m)));
            }
            let same = outs_keys.len() == cur_keys.len()
                && outs_keys.iter().all(|k| cur_keys.contains(k));
            if !same {
                settled = false;
                round_changed.insert(hk.clone());
                changed.insert(leaf_text(hl));
            }
        }
                }
                _ => {}
            }
        }
        if settled {
            break;
        }
        dirty = Some(round_changed);
    }
    // ---- VIEW == REASSEMBLY FOR DERIVED HEADS (engine.py's
    // _reconcile_absorbed_heads): an ABSORBED head's ** cell is the derive
    // cache and its RMAP column is the storage, so after the fixpoint the
    // columns become exactly the cell — a present row writes its value onto
    // the key's table row (a fresh key joins the index, hole-padded) and a
    // key whose derived row VANISHED holes the column, so the sweep's
    // supersession reaches the storage. The layout rides in the store as the
    // rmapColumns cell (rows ⟨table, col, ft⟩); a changed head without a
    // layout row is own-table and needs nothing. Row cells address by
    // cellkey's text (S and I leaves only), exactly as native_apply writes
    // them.
    {
        let mut layout: HashMap<String, (String, usize)> = HashMap::new();
        let mut widths: HashMap<String, usize> = HashMap::new();
        for r in store.pop_rows(&leaf("rmapColumns")) {
            let it = items(&list_of(&r));
            if it.len() >= 3 {
                if let (Some(t), Some(Leaf::I(c)), Some(f)) =
                    (aval(&it[0]), aval(&it[1]).as_deref(), aval(&it[2]))
                {
                    if *c < 2 {
                        // a malformed layout row must not fault the resident
                        continue;
                    }
                    let (tn, fname) = (leaf_text(&t), leaf_text(&f));
                    let w = widths.entry(tn.clone()).or_insert(1);
                    *w = (*w).max(*c as usize);
                    layout.insert(fname, (tn, *c as usize));
                }
            }
        }
        if layout.is_empty() {
            // python's _reconcile_absorbed_heads reads NO cell: it computes
            // the partition FRESH per call (engine.py:1520 → rmap_partition,
            // which IS the canonical system:partition). rmapColumns is
            // layout_cells' LATER materialization, so a pre-layout store (the
            // native compile pipeline's post-model fixpoint) legitimately
            // lacks it — derive the same layout canon-first: system:partition
            // over D, then system:table_columns per absorbed table, the same
            // ⟨table, 2+j, ft⟩ rows layout_cells would write (engine.py:1701).
            let ev = store.build_neval(srv.nprocess.clone());
            let na = |s: &str| N::A(std::rc::Rc::new(Leaf::S(s.into())));
            let nd_v = store.nd_native();
            let pairs_v = n_to_v(&ev.mu(napp(na("system:partition"), nd_v)));
            // ⟨table, ft⟩ pairs in canon order; part: ft → table (engine.py:1685)
            let mut part: Vec<(String, String)> = Vec::new();
            if let Shape::Seq(l) = shape(&pairs_v) {
                for p in items(&l) {
                    let it = items(&list_of(&p));
                    if it.len() >= 2 {
                        if let (Some(t), Some(f)) = (aval(&it[0]), aval(&it[1])) {
                            part.push((leaf_text(&f), leaf_text(&t)));
                        }
                    }
                }
            }
            let items_v = seq(from_vec(
                part.iter()
                    .map(|(f, t)| {
                        seqc(vec![atom(Leaf::S(f.clone())), atom(Leaf::S(t.clone()))])
                    })
                    .collect(),
            ));
            let mut tables: Vec<String> = Vec::new();
            for (f, t) in &part {
                if f != t && !tables.iter().any(|x| x == t) {
                    tables.push(t.clone());
                }
            }
            tables.sort();
            for t in &tables {
                let cols_v = n_to_v(&ev.mu(napp(
                    napp(na("system:table_columns"), na(t)),
                    v_to_n(&items_v),
                )));
                if let Shape::Seq(l) = shape(&cols_v) {
                    for (j, c) in items(&l).iter().enumerate() {
                        if let Some(f) = aval(c) {
                            let col = 2 + j;
                            let w = widths.entry(t.clone()).or_insert(1);
                            *w = (*w).max(col);
                            layout.insert(leaf_text(&f), (t.clone(), col));
                        }
                    }
                }
            }
        }
        if !layout.is_empty() {
            // unary heads write "T"; the role rows carry each head's arity
            let mut maxpos: HashMap<String, i64> = HashMap::new();
            for r in store.pop_rows(&leaf("role")) {
                let it = items(&list_of(&r));
                if it.len() >= 4 {
                    if let (Some(f), Some(Leaf::I(p))) =
                        (aval(&it[1]), aval(&it[2]).as_deref())
                    {
                        let e = maxpos.entry(leaf_text(&f)).or_insert(0);
                        *e = (*e).max(*p);
                    }
                }
            }
            let keytext = |l: &Leaf| match l {
                Leaf::S(s) => Some(s.clone()),
                Leaf::I(i) => Some(i.to_string()),
                _ => None,
            };
            let hole = || atom(Leaf::S("#".to_string()));
            // python's reconcile filter (engine.py:1523): touched ∩
            // DERIVED_HEADS ∩ absorbed — a base-populated absorbed ft that
            // changed in the round must NOT have its column reassembled
            // from its pop here (its storage is the routed write's, not the
            // derive cache's; visible on pre-layout stores where the extra
            // write fills columns python leaves holed)
            let derived: HashSet<String> = rules
                .iter()
                .chain(agg_rules.iter())
                .map(|rr| leaf_text(&rr.head))
                .collect();
            for ftname in changed.iter() {
                if !derived.contains(ftname) {
                    continue;
                }
                let (table, col) = match layout.get(ftname) {
                    Some((t, c)) => (t.clone(), *c),
                    None => continue,
                };
                let width = *widths.get(&table).unwrap_or(&1);
                let unary = maxpos.get(ftname).copied() == Some(1);
                // want: key text → ⟨key atom, the value the column must carry⟩
                let mut want: HashMap<String, (V, V)> = HashMap::new();
                for r in store.pop_rows(&leaf(ftname)) {
                    let it = items(&list_of(&r));
                    if it.is_empty() {
                        continue;
                    }
                    let kt = match aval(&it[0]).and_then(|l| keytext(&l)) {
                        Some(t) => t,
                        None => continue,
                    };
                    let v = if unary {
                        atom(Leaf::S("T".to_string()))
                    } else if it.len() >= 2 {
                        it[1].clone()
                    } else {
                        hole()
                    };
                    want.insert(kt, (it[0].clone(), v));
                }
                let tleaf = leaf(&table);
                let mut tbl = store.pop_rows(&tleaf);
                // every indexed key gets its column written or holed; a
                // duplicate index entry visits once, as Python's key set does
                let mut seen: HashSet<String> = HashSet::new();
                let mut visits: Vec<String> = Vec::new();
                for r in &tbl {
                    let it = items(&list_of(r));
                    if it.is_empty() {
                        continue;
                    }
                    if let Some(kt) = aval(&it[0]).and_then(|l| keytext(&l)) {
                        if seen.insert(kt.clone()) {
                            visits.push(kt);
                        }
                    }
                }
                for kt in visits {
                    let v = want.remove(&kt).map(|(_, v)| v).unwrap_or_else(hole);
                    let rc = Leaf::S(format!("{}:{}", table, kt));
                    let mut row = store.pop_rows(&rc);
                    if row.is_empty() {
                        row = vec![atom(Leaf::S(kt.clone()))];
                    }
                    while row.len() < width {
                        row.push(hole());
                    }
                    if !eqobj(&row[col - 1], &v) {
                        row[col - 1] = v;
                        store.setcell(&rc,
                                   seq(from_vec(row)));
                    }
                }
                // the leftover keys are FRESH: hole-padded rows join the
                // index in Python's sorted(...) order (numeric ints, lexical
                // strings; Python faults on a mix, so the split is free)
                let mut fresh: Vec<(u8, i64, String)> = Vec::new();
                for (kt, (ka, _)) in &want {
                    match aval(ka).as_deref() {
                        Some(Leaf::I(i)) => fresh.push((0, *i, kt.clone())),
                        _ => fresh.push((1, 0, kt.clone())),
                    }
                }
                fresh.sort();
                let grew = !fresh.is_empty();
                for (_, _, kt) in fresh {
                    let (ka, v) = match want.remove(&kt) {
                        Some(kv) => kv,
                        None => continue,
                    };
                    let rc = Leaf::S(format!("{}:{}", table, kt));
                    let mut row = store.pop_rows(&rc);
                    if row.is_empty() {
                        row = vec![ka.clone()];
                    }
                    while row.len() < width {
                        row.push(hole());
                    }
                    row[col - 1] = v;
                    store.setcell(&rc,
                               seq(from_vec(row)));
                    tbl.push(seq(from_vec(vec![ka])));
                }
                if grew {
                    store.setcell(&tleaf,
                               seq(from_vec(tbl)));
                }
            }
        }
    }
    // REPLACE the retained store with the fixpoint, the retain protocol's
    // commit: d and its cell index move together, and the native-carrier mirror
    // moves with them (nd was maintained in lockstep through the passes, so it
    // already IS the fixpoint; ncells rebuilds from it once so its canonical
    // order matches n_cells_of). Keeping the mirror current here means a native
    // machine step after a derive reads the derived store, not a stale one.
    srv.d = cells_to_d(&store.to_all_cells());
    srv.cells = store.to_active_cells();
    srv.nd = store.nd_native();
    srv.ncells = (*store.ncells_native()).clone();
    let mut r = String::from("{\"rounds\":");
    r.push_str(&rounds.to_string());
    r.push_str(",\"changed\":[");
    for (i, name) in changed.iter().enumerate() {
        if i > 0 {
            r.push(',');
        }
        esc(name, &mut r);
    }
    r.push_str("]}");
    Ok(r)
}

// ============================ the compile driver ==============================
// Phase one of the Rust-native compile (#20, docs/2026-07-10-rust-native-compile.md):
// op_compile_model is the DRIVER SKELETON of python compiler.compile_model_selfhost
// (compiler.py:2214) — the thin host g-loop over canon. Shape (the doc's anatomy):
//   (a) statements(text)      — split the readings text into statements (host string op)
//   (b) _split_modality       — alethic/deontic + sign + inner (host string op)
//   (c) BATCH classification  — stage-1 field facts under per-statement ids into a
//       SCRATCH view of the resident store, ONE op_run_rules derive over the grammar's
//       recognizer rules (stratum 4: one lfp for every statement, never one each),
//       Statement_has_Classification read back, the resident store RESTORED whole
//       (python classify_all_via_M's immutability, done here by save/restore)
//   (d) the dispatch loop     — Classification_has_Translator rows name the
//       translators; each dispatches through the reducer (rho): reduce_over over
//       ⟨inner, mfield, ctx, D⟩, the direct analog of python _apply(_A(t), operand)
// NATIVE SINCE THE SKELETON (driver slice 2):
//   - the grammar store (gap 1): grammar_D()'s resident equivalent — when the
//     resident store lacks the grammar cells (the classLit probe), the compiled
//     grammar THAWS from a serve-protocol sidecar (grammar_sidecar op arg, else
//     <root>/shared/forml2-grammar.store.json walking up from the exe) into the
//     classification SCRATCH; the resident store never becomes the grammar.
//   - the prepass context (gap 2): _known + _prepass_context + _context_of
//     ported whole (the seed classifier's pattern table hand-rolled below), so
//     nouns carry the known names into Stage-1 (Statement_has_Role_Reference)
//     and ctx carries ⟨names, subtype closure, fact-type slugs, plain⟩.
// NOT YET NATIVE (the honest gaps, reported in the answer's "missing"):
//   - the model D: python seeds meta.initial_D() (the process seed); the
//     skeleton threads an EMPTY store.
//   - the translator BODIES: host closures in python (_stmt_translator_impl,
//     compiler.py:2100) until #18 canonizes them; reduce_over answers ⊥ for a
//     name DEFS does not carry, and the _COOK boundary below explains which
//     kinds are gated on a host cook. Python's "graceful absence" (unregistered
//     translator counts accepted) is NOT mirrored: the skeleton reports the
//     truth — nothing translated is nothing accepted.

// _TRAIL_MARK (compiler.py:165): "<body>. <mark>" with mark in {**, ++, *, +}
// normalizes to the marker-before-period form "<body> <mark>." — NORMA writes
// storage markers AFTER the period. Hand-rolled: the host build is zero-dep.
fn trail_mark(s: &str) -> Option<(String, String)> {
    for mark in ["**", "++", "*", "+"] {
        if let Some(pre) = s.strip_suffix(mark) {
            let pre = pre.trim_end();
            if let Some(body) = pre.strip_suffix('.') {
                if body.chars().last().map_or(false, |c| !c.is_whitespace()) {
                    return Some((body.to_string(), mark.to_string()));
                }
            }
        }
    }
    None
}

// _split_sentences (compiler.py:197): a line carrying SEVERAL sentences splits
// at quote-aware boundaries ('. ' followed by a capital or a marker); periods
// inside quoted values never split.
fn split_sentences(s: &str) -> Vec<String> {
    let cs: Vec<char> = s.chars().collect();
    let mut parts: Vec<String> = Vec::new();
    let mut cur: Vec<char> = Vec::new();
    let mut q = false;
    let mut i = 0usize;
    while i < cs.len() {
        let c = cs[i];
        if c == '\'' {
            q = !q;
        }
        cur.push(c);
        if !q
            && c == '.'
            && i + 2 < cs.len()
            && cs[i + 1] == ' '
            && (cs[i + 2].is_uppercase() || "'*+".contains(cs[i + 2]))
        {
            let part: String = cur.iter().collect();
            parts.push(part.trim().to_string());
            cur.clear();
            i += 1;
        }
        i += 1;
    }
    let tail: String = cur.iter().collect();
    let tail = tail.trim();
    if !tail.is_empty() {
        parts.push(tail.to_string());
    }
    parts
}

// statements (compiler.py:168): accumulate lines until one ends with '.'
// (multi-line aware); comment blocks vanish, a heading BREAKS accumulation,
// trailing NORMA markers normalize before the period.
fn split_statements(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut buf: Vec<String> = Vec::new();
    let mut in_comment = false;
    for line in text.lines() {
        let mut s = line.trim().to_string();
        if in_comment {
            if s.contains("-->") {
                in_comment = false;
            }
            continue;
        }
        if s.starts_with("<!--") {
            in_comment = !s.contains("-->");
            continue;
        }
        if s.starts_with('#') {
            buf.clear();
            continue;
        }
        if s.is_empty() || s == "Fact Types:" {
            continue;
        }
        if let Some((body, mark)) = trail_mark(&s) {
            s = format!("{} {}.", body, mark);
        }
        let done = s.ends_with('.');
        buf.push(s);
        if done {
            out.extend(split_sentences(&buf.join(" ")));
            buf.clear();
        }
    }
    if !buf.is_empty() {
        out.extend(split_sentences(&buf.join(" ")));
    }
    out
}

// _MODAL / _split_modality (compiler.py:224): strip a leading modal operator,
// yielding (modality, sign, inner). possibility = the ABSENCE of a constraint.
const MODAL: [(&str, &str, &str); 6] = [
    ("It is obligatory that ", "deontic", "positive"),
    ("It is forbidden that ", "deontic", "negative"),
    ("It is permitted that ", "deontic", "possibility"),
    ("It is necessary that ", "alethic", "positive"),
    ("It is impossible that ", "alethic", "negative"),
    ("It is possible that ", "alethic", "possibility"),
];

fn split_modality(stmt: &str) -> (&'static str, &'static str, String) {
    for (op, m, sg) in MODAL {
        if let Some(rest) = stmt.strip_prefix(op) {
            return (m, sg, rest.trim().to_string());
        }
    }
    ("alethic", "positive", stmt.to_string())
}

// _SM_SUSPECT (compiler.py:2244): a statement carrying quoted literals AND
// machine phrasing that parses as NOTHING is malformed, reported loudly (the
// arrow-glue-loud class). The regex '[^']+'.*(phrase) hand-rolls as: any
// adjacent quote pair with content, one of the phrases after its close.
fn sm_suspect(stmt: &str) -> bool {
    const PHRASES: [&str; 5] = [
        "is initial",
        "is from Status",
        "is to Status",
        "is triggered by Fact Type",
        "is defined in State Machine",
    ];
    let qs: Vec<usize> = stmt
        .char_indices()
        .filter(|(_, c)| *c == '\'')
        .map(|(i, _)| i)
        .collect();
    for w in qs.windows(2) {
        if w[1] - w[0] > 1 {
            let rest = &stmt[w[1] + 1..];
            if PHRASES.iter().any(|p| rest.contains(p)) {
                return true;
            }
        }
    }
    false
}

// The _COOK boundary (compiler.py, the #18 doctrine at system.canon:5504):
// Stage-1 text→X resolution the HOST performs before a translator body sees
// its groups. PORTED (#20): src/cooks.rs carries the productions (the
// _CLASSIFY table with group extraction), every _COOK entry (rule_if/rule_iff
// included), the cs_rows/sm_rows canon reductions, and the _plan/_h_* handler
// layer, so the dispatch loop below translates natively when a translator name
// carries no canon DEF. native_cook is that boundary: production match →
// cooks::cook → the crows groups through the translator body, answering the
// per-statement ⟨asserts, objs⟩ (python _stmt_translator_impl's contract).
fn native_cook(
    t: &str,
    inner: &str,
    mfield: &str,
    known: &cooks::Known,
    srv: &Srv,
) -> Result<Option<cooks::Fire>, String> {
    cooks::translate(translator_kinds(t), inner, mfield, known, srv)
}

// register_translators' table (compiler.py:2151): the Stage-1 kinds each
// translator name serves. The dispatch loop consults it to EXPLAIN a native
// miss through the _COOK boundary — which host cooks gate that translator.
fn translator_kinds(t: &str) -> &'static [&'static str] {
    match t {
        "translate_nouns" => &["entity_type", "value_type", "subtype_of", "brace_subtypes"],
        "translate_subtypes" => &["subtype_of", "brace_subtypes"],
        "translate_enum_values" => &["value_constraint"],
        "translate_data_types" => &["data_type"],
        "translate_instance_facts" => &["fact_type_reading"],
        "translate_fact_types" => &["fact_type_reading"],
        "translate_derivation_mode_facts" => &["fact_type_reading"],
        "translate_derivation_rules" => {
            &["class_rule", "rule_if", "rule_iff", "derivation_rule"]
        }
        "translate_cardinality_constraints" => &[
            "uniqueness",
            "inverse_uc",
            "spanning_uc",
            "spanning_uc2",
            "frequency",
            "neg_uniqueness",
            "disjunctive_mandatory",
            "mandatory",
            "for_each_mandatory",
            "neg_mandatory",
        ],
        "translate_ring_constraints" => &["ring"],
        "translate_set_constraints" => &[
            "set_comparison",
            "subset",
            "subset_trailing",
            "equality",
            "disjunctive_mandatory",
        ],
        "translate_value_constraints" => &["value_constraint"],
        "translate_state_machines" => &[
            "sm_def",
            "sm_initial",
            "sm_from",
            "sm_to",
            "sm_trigger",
            "sm_guard",
            "sm_emit",
            "sm_moore",
        ],
        "translate_finality" => &["finality"],
        "translate_objectifications" => &["objectification"],
        "translate_negation" => &["neg_pair", "negation"],
        _ => &[],
    }
}

// ======================= the prepass context (gap 2) ==========================
// _known + _prepass_context + _context_of (compiler.py:529/416/404) ported
// whole, with their dependency chain: the SEED classifier's ordered pattern
// table (_CLASSIFY, compiler.py:253 — every regex hand-rolled, zero-dep),
// _implicit_nouns, _prose_suspect, _strip_derivation, and the reading
// machinery _reading/_ftid_from/_atomic_run_guard (the certified-equal host
// override of system:reading_parse/ftid, test_reading_canon). The prepass
// feeds classification (nouns → Statement_has_Role_Reference field facts)
// and the translator operand's ctx (names, subtype closure, fact-type slugs,
// plain readings).

// split at the FIRST separator occurrence leaving both sides nonempty — the
// lazy-group split (.+?)SEP(.+): regex backtracking skips an occurrence whose
// left side is empty, so the scan continues instead of failing
fn split_first<'a>(s: &'a str, sep: &str) -> Option<(&'a str, &'a str)> {
    for (p, _) in s.match_indices(sep) {
        let (a, b) = (&s[..p], &s[p + sep.len()..]);
        if !a.is_empty() && !b.is_empty() {
            return Some((a, b));
        }
    }
    None
}

// split at the LAST such occurrence — the greedy-group split (.+)SEP(.+)
fn split_last<'a>(s: &'a str, sep: &str) -> Option<(&'a str, &'a str)> {
    let hits: Vec<usize> = s.match_indices(sep).map(|(p, _)| p).collect();
    for p in hits.into_iter().rev() {
        let (a, b) = (&s[..p], &s[p + sep.len()..]);
        if !a.is_empty() && !b.is_empty() {
            return Some((a, b));
        }
    }
    None
}

// analyze()'s annotation strip (compiler.py:353): a TRAILING parenthetical is
// an annotation, not sentence content — re.sub(r"\s*\([^()]*\)\.$", ".", s)
fn strip_annotation(stmt: &str) -> String {
    let body = match stmt.strip_suffix('.') {
        Some(b) => b,
        None => return stmt.to_string(),
    };
    if !body.ends_with(')') {
        return stmt.to_string();
    }
    // the nearest paren before the closer must be the opener (no nesting)
    match body[..body.len() - 1].rfind(|c| c == '(' || c == ')') {
        Some(i) if body.as_bytes()[i] == b'(' => {
            format!("{}.", body[..i].trim_end())
        }
        _ => stmt.to_string(),
    }
}

// quote parity: '[^']*' pairs quotes sequentially, so a position is OUTSIDE
// literals exactly when an even number of quotes precede it
fn quote_positions(s: &str) -> Vec<usize> {
    s.match_indices('\'').map(|(p, _)| p).collect()
}

fn even_before(qs: &[usize], p: usize) -> bool {
    qs.iter().filter(|&&q| q < p).count() % 2 == 0
}

// _QUOTED_SPAN.sub(repl, s): every 'span' replaces with repl, quotes pairing
// sequentially, an unpaired trailing quote left verbatim
fn blank_spans(s: &str, repl: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    loop {
        match rest.find('\'') {
            None => {
                out.push_str(rest);
                break;
            }
            Some(a) => match rest[a + 1..].find('\'') {
                None => {
                    out.push_str(rest);
                    break;
                }
                Some(b) => {
                    out.push_str(&rest[..a]);
                    out.push_str(repl);
                    rest = &rest[a + 1 + b + 1..];
                }
            },
        }
    }
    out
}

// entity_type / value_type: ^(.+?)(?:\(\.(.+)\))? SUFFIX$ — lazy head, an
// optional (.RefMode) parenthetical peeled when it directly abuts the suffix
fn cls_entity_like(s: &str, suffix: &str) -> Option<Vec<String>> {
    let head = s.strip_suffix(suffix)?;
    if head.is_empty() {
        return None;
    }
    if head.ends_with(')') {
        for (p, _) in head.match_indices("(.") {
            if p >= 1 && p + 2 < head.len() - 1 {
                return Some(vec![
                    head[..p].to_string(),
                    head[p + 2..head.len() - 1].to_string(),
                ]);
            }
        }
    }
    Some(vec![head.to_string()])
}

// the sm_* family: ^PFX'(.+)'MID'(.+)'\.$ — existence only (no prepass groups)
fn sm_two(s: &str, pfx: &str, mid: &str) -> bool {
    match s.strip_prefix(pfx).and_then(|b| b.strip_suffix("'.")) {
        Some(body) => split_first(body, mid).is_some(),
        None => false,
    }
}

// ^[Ff]or each (.+?), it is impossible that that .+? (.+)MID(.+)\.$ —
// the negative-form recognizers (existence only)
fn impossible_that_that(s: &str, mid: &str) -> bool {
    for pfx in ["For each ", "for each "] {
        let body = match s.strip_prefix(pfx).and_then(|b| b.strip_suffix('.')) {
            Some(b) => b,
            None => continue,
        };
        for (p, _) in body.match_indices(", it is impossible that that ") {
            if p == 0 {
                continue;
            }
            let r = &body[p + 29..];
            for (m, _) in r.match_indices(mid) {
                if r[m + mid.len()..].is_empty() {
                    continue;
                }
                // ^.+? (.+)MID: a space with ≥1 char before it and ≥1 after
                if r[..m].match_indices(' ').any(|(a, _)| a >= 1 && a + 1 < m) {
                    return true;
                }
            }
        }
    }
    false
}

// class_rule: ^(?![*+])(\S[^']*?) has (\S[^']*?) '(.+?)' iff (.+)\.$ —
// existence only, with the lazy groups' full backtracking order
fn cls_class_rule(s: &str) -> bool {
    if s.starts_with('*') || s.starts_with('+') {
        return false;
    }
    let body = match s.strip_suffix('.') {
        Some(b) => b,
        None => return false,
    };
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
                    return true;
                }
            }
        }
    }
    false
}

// rule_if: ^(?![*+])(quote-aware*?\d\S*quote-aware*?) iff? (.+)\.$ — the head
// carries a digit OUTSIDE literals; the keyword boundary sits at/after the
// digit token's end, outside literals, earliest first. (The corner where the
// digit's own token carries quotes — \S* eating a lone quote — is accepted as
// a delta; no corpus rule head does.) Answers the head group.
fn cls_rule_if(s: &str) -> Option<Vec<String>> {
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
    cands.sort();
    for (p, kw) in cands {
        let tail = &s[p + kw..];
        if tail.len() >= 2 && tail.ends_with('.') {
            return Some(vec![s[..p].to_string()]);
        }
    }
    None
}

// rule_iff: ^(?:([*+]{1,2}) )?(quote-aware*?) iff (.+)\.$ — optional NORMA
// storage marker (greedy: two chars, then one, then none), quote-aware head
// (possibly empty), earliest outside-literals " iff ". Answers ⟨marker, head⟩.
fn cls_rule_iff(s: &str) -> Option<Vec<String>> {
    if !s.ends_with('.') {
        return None;
    }
    let b = s.as_bytes();
    let marker_at = |n: usize| -> bool {
        s.len() > n
            && b[..n].iter().all(|c| *c == b'*' || *c == b'+')
            && b[n] == b' '
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
                let marker = if off > 0 { &s[..off - 1] } else { "" };
                return Some(vec![marker.to_string(), body[..p].to_string()]);
            }
        }
    }
    None
}

// subset_trailing: ^(?![*+])(quote-aware+?) if (quote-aware+)\.$ — existence
fn cls_subset_trailing(s: &str) -> bool {
    if s.starts_with('*') || s.starts_with('+') || !s.ends_with('.') {
        return false;
    }
    let qs = quote_positions(s);
    for (p, _) in s.match_indices(" if ") {
        if p >= 1 && even_before(&qs, p) {
            let tail = &s[p + 4..s.len() - 1];
            if !tail.is_empty() && quote_positions(tail).len() % 2 == 0 {
                return true;
            }
        }
    }
    false
}

// frequency: ^[Ii]n each population of (.+), each (.+) combination occurs
// (at most|at least|exactly) (\d+) times?\.$ — existence, parsed off the tail
fn cls_frequency(s: &str) -> bool {
    for pfx in ["In each population of ", "in each population of "] {
        let body = match s.strip_prefix(pfx).and_then(|t| t.strip_suffix('.')) {
            Some(t) => t,
            None => continue,
        };
        let body = match body.strip_suffix("times").or_else(|| body.strip_suffix("time")) {
            Some(t) => t,
            None => continue,
        };
        let body = match body.strip_suffix(' ') {
            Some(t) => t,
            None => continue,
        };
        let trimmed = body.trim_end_matches(|c: char| c.is_ascii_digit());
        if trimmed.len() == body.len() {
            continue; // (\d+) needs at least one digit
        }
        let body = match trimmed.strip_suffix(' ') {
            Some(t) => t,
            None => continue,
        };
        let body = match ["at most", "at least", "exactly"]
            .iter()
            .find_map(|kw| body.strip_suffix(kw))
        {
            Some(t) => t,
            None => continue,
        };
        let body = match body.strip_suffix(" combination occurs ") {
            Some(t) => t,
            None => continue,
        };
        if split_first(body, ", each ").is_some() {
            return true;
        }
    }
    false
}

// brace_subtypes: ^\{(.+)\} are (mutually exclusive )?subtypes of (.+)\.$
fn cls_brace_subtypes(s: &str) -> Option<Vec<String>> {
    let t = s.strip_prefix('{')?.strip_suffix('.')?;
    let hits: Vec<usize> = t.match_indices("} are ").map(|(p, _)| p).collect();
    for p in hits.into_iter().rev() {
        let g0 = &t[..p];
        if g0.is_empty() {
            continue;
        }
        let mut rest = &t[p + 6..];
        let mut g1 = "";
        if let Some(r2) = rest.strip_prefix("mutually exclusive ") {
            g1 = "mutually exclusive ";
            rest = r2;
        }
        if let Some(g2) = rest.strip_prefix("subtypes of ") {
            if !g2.is_empty() {
                return Some(vec![g0.to_string(), g1.to_string(), g2.to_string()]);
            }
        }
    }
    None
}

// The seed classifier over the modality-stripped inner statement: the FULL
// _CLASSIFY table (compiler.py:253) in its exact arbitration order — an
// earlier pattern's claim never reaches a later one. Groups are extracted
// only for the kinds the prepass consumes; recognition-only kinds answer
// empty groups (the prepass has no branch for them).
fn classify_inner(s: &str) -> (&'static str, Vec<String>) {
    if let Some(g) = cls_entity_like(s, " is an entity type.") {
        return ("entity_type", g);
    }
    if let Some(g) = cls_entity_like(s, " is a value type.") {
        return ("value_type", g);
    }
    if let Some(body) = s.strip_prefix("Reference Scheme: ").and_then(|b| b.strip_suffix('.')) {
        if let Some((a, b)) = split_last(body, " has ") {
            return ("ref_scheme", vec![a.to_string(), b.to_string()]);
        }
    }
    if let Some(body) = s.strip_prefix("Reference Mode: ").and_then(|b| b.strip_suffix('.')) {
        if !body.is_empty() {
            return ("ref_mode", Vec::new());
        }
    }
    if let Some(body) = s.strip_prefix("Data Type: ").and_then(|b| b.strip_suffix('.')) {
        if !body.is_empty() {
            return ("data_type", Vec::new());
        }
    }
    for (kind, pfx, mid) in [
        ("sm_def", "State Machine Definition '", "' is for Noun '"),
        ("sm_initial", "Status '", "' is initial in State Machine Definition '"),
        ("sm_from", "Transition '", "' is from Status '"),
        ("sm_to", "Transition '", "' is to Status '"),
        ("sm_trigger", "Transition '", "' is triggered by Fact Type '"),
        ("sm_guard", "Transition '", "' is guarded by Fact Type '"),
        ("sm_emit", "Transition '", "' emits '"),
        ("sm_moore", "Status '", "' emits '"),
    ] {
        if sm_two(s, pfx, mid) {
            return (kind, Vec::new());
        }
    }
    for pfx in [
        "The possible values of ",
        "the possible values of ",
        "The possible value of ",
        "the possible value of ",
    ] {
        if let Some(body) = s.strip_prefix(pfx).and_then(|b| b.strip_suffix('.')) {
            if split_first(body, " are ").is_some() || split_first(body, " is ").is_some() {
                return ("value_constraint", Vec::new());
            }
        }
    }
    for pfx in ["In each population of ", "in each population of "] {
        if let Some(body) = s
            .strip_prefix(pfx)
            .and_then(|b| b.strip_suffix(" combination occurs at most once."))
        {
            if split_first(body, ", each ").is_some() {
                return ("spanning_uc", Vec::new());
            }
        }
    }
    for pfx in ["Each ", "each "] {
        if let Some(body) = s.strip_prefix(pfx).and_then(|b| b.strip_suffix('.')) {
            if split_first(body, " combination occurs at most once in the population of ")
                .is_some()
            {
                return ("spanning_uc2", Vec::new());
            }
        }
    }
    if let Some(body) = s.strip_prefix("For each ").and_then(|b| b.strip_suffix('.')) {
        if split_first(body, ", some ").is_some() {
            return ("for_each_mandatory", Vec::new());
        }
    }
    if cls_frequency(s) {
        return ("frequency", Vec::new());
    }
    for w in [
        "acyclic", "asymmetric", "antisymmetric", "intransitive", "irreflexive", "symmetric",
    ] {
        if let Some(head) = s.strip_suffix(&format!(" is {}.", w)) {
            if !head.is_empty() {
                return ("ring", Vec::new());
            }
        }
    }
    if let Some(body) = s.strip_suffix('.') {
        if let Some((a, b)) = split_last(body, " is a subtype of ") {
            return ("subtype_of", vec![a.to_string(), b.to_string()]);
        }
    }
    if let Some(g) = cls_brace_subtypes(s) {
        return ("brace_subtypes", g);
    }
    for pfx in ["This association with ", "this association with "] {
        if let Some(body) = s.strip_prefix(pfx).and_then(|b| b.strip_suffix('.')) {
            if let Some((a, b)) =
                split_last(body, " provides the preferred identification scheme for ")
            {
                return ("objectification", vec![a.to_string(), b.to_string()]);
            }
        }
    }
    for pfx in ["For each ", "for each "] {
        if let Some(body) = s.strip_prefix(pfx).and_then(|b| b.strip_suffix('.')) {
            if split_first(body, ", exactly one of the following holds: ").is_some()
                || split_first(body, ", at most one of the following holds: ").is_some()
            {
                return ("set_comparison", Vec::new());
            }
        }
    }
    if impossible_that_that(s, " more than one ") {
        return ("neg_uniqueness", Vec::new());
    }
    if impossible_that_that(s, " no ") {
        return ("neg_mandatory", Vec::new());
    }
    for pfx in ["For each ", "for each "] {
        if let Some(body) = s.strip_prefix(pfx).and_then(|b| b.strip_suffix('.')) {
            for (p, _) in body.match_indices(", ") {
                if p >= 1 && split_first(&body[p + 2..], " or ").is_some() {
                    return ("disjunctive_mandatory", Vec::new());
                }
            }
        }
    }
    for pfx in ["For each ", "for each "] {
        if let Some(body) = s.strip_prefix(pfx).and_then(|b| b.strip_suffix('.')) {
            for sep in [", at most one ", ", exactly one "] {
                for (p, _) in body.match_indices(sep) {
                    if p >= 1 {
                        let t = &body[p + sep.len()..];
                        if split_first(t, " that ").is_some()
                            || split_first(t, " those ").is_some()
                        {
                            return ("inverse_uc", Vec::new());
                        }
                    }
                }
            }
        }
    }
    for pfx in ["If ", "if "] {
        if let Some(body) = s.strip_prefix(pfx).and_then(|b| b.strip_suffix('.')) {
            if split_first(body, " then ").is_some() {
                return ("subset", Vec::new());
            }
        }
    }
    if cls_class_rule(s) {
        return ("class_rule", Vec::new());
    }
    if let Some(body) = s.strip_suffix('.') {
        if split_first(body, " if and only if ").is_some() {
            return ("equality", Vec::new());
        }
    }
    if let Some(g) = cls_rule_if(s) {
        return ("rule_if", g);
    }
    if let Some(g) = cls_rule_iff(s) {
        return ("rule_iff", g);
    }
    if let Some(body) = s.strip_prefix("*Each ").and_then(|b| b.strip_suffix('.')) {
        for (p, _) in body.match_indices(" is some ") {
            if p >= 1 && split_first(&body[p + 9..], " who ").is_some() {
                return ("derivation_rule", Vec::new());
            }
        }
    }
    if let Some(body) = s.strip_prefix("any ").and_then(|b| b.strip_suffix('.')) {
        if split_first(body, " more than one ").is_some() {
            return ("neg_uniqueness", Vec::new());
        }
    }
    if let Some(body) = s.strip_prefix("any ").and_then(|b| b.strip_suffix('.')) {
        if split_first(body, " no ").is_some() {
            return ("neg_mandatory", Vec::new());
        }
    }
    for pfx in ["Each ", "each "] {
        if let Some(body) = s.strip_prefix(pfx).and_then(|b| b.strip_suffix('.')) {
            if split_first(body, " or ").is_some() {
                return ("disjunctive_mandatory", Vec::new());
            }
        }
    }
    for pfx in ["Each ", "each "] {
        if let Some(body) = s.strip_prefix(pfx).and_then(|b| b.strip_suffix('.')) {
            let mut cands: Vec<(usize, &str)> = Vec::new();
            for (p, _) in body.match_indices(" at most one ") {
                cands.push((p, "at most one"));
            }
            for (p, _) in body.match_indices(" exactly one ") {
                cands.push((p, "exactly one"));
            }
            cands.sort();
            for (p, q) in cands {
                let right = &body[p + 13..];
                if p >= 1 && !right.is_empty() {
                    return (
                        "uniqueness",
                        vec![body[..p].to_string(), q.to_string(), right.to_string()],
                    );
                }
            }
        }
    }
    for pfx in ["Each ", "each "] {
        if let Some(body) = s.strip_prefix(pfx).and_then(|b| b.strip_suffix('.')) {
            if let Some((a, b)) = split_first(body, " some ") {
                return ("mandatory", vec![a.to_string(), b.to_string()]);
            }
        }
    }
    if let Some(body) = s.strip_suffix('.') {
        if let Some(sp) = body.find(' ') {
            let (tok0, rest) = (&body[..sp], &body[sp + 1..]);
            if !tok0.is_empty() && !tok0.contains(char::is_whitespace) {
                if let Some(dg) = rest.strip_prefix("becomes final at depth ") {
                    if !dg.is_empty() && dg.bytes().all(|b| b.is_ascii_digit()) {
                        return ("finality", Vec::new());
                    }
                }
                for neg in ["does not ", "is not "] {
                    if let Some(t) = rest.strip_prefix(neg) {
                        if t.chars().next().map_or(false, |c| !c.is_whitespace()) {
                            return ("neg_pair", Vec::new());
                        }
                    }
                }
            }
        }
    }
    if let Some(body) = s.strip_suffix('.') {
        if split_last(body, " ~").is_some() {
            return ("negation", Vec::new());
        }
    }
    if cls_subset_trailing(s) {
        return ("subset_trailing", Vec::new());
    }
    if let Some(body) = s.strip_suffix('.') {
        if !body.is_empty() {
            return ("fact_type_reading", vec![body.to_string()]);
        }
    }
    ("UNPARSED", vec![s.to_string()])
}

// classify (compiler.py:364) = analyze minus the modality tag: annotation
// strip, modality split, possibility short-circuit, then the pattern table
fn classify_kind(stmt: &str) -> (&'static str, Vec<String>) {
    let stripped = strip_annotation(stmt);
    let (_m, sg, inner) = split_modality(&stripped);
    if sg == "possibility" {
        return ("possibility", vec![inner.trim_end_matches('.').to_string()]);
    }
    classify_inner(&inner)
}

// the sentence vocabulary that never OPENS a type name (compiler.py:468)
const IMPLICIT_STOP: [&str; 27] = [
    "If", "When", "Then", "That", "This", "An", "A", "The", "Each", "Some", "No", "Every",
    "Not", "It", "There", "Once", "For", "In", "Of", "To", "On", "At", "By", "With", "And",
    "Or", "Only",
];

fn strip_pnc(t: &str) -> &str {
    t.trim_matches(|c| c == '.' || c == ';' || c == ':')
}

// _implicit_nouns (compiler.py:474): a maximal Title-case run is a noun
// CANDIDATE; it becomes a noun only when CORROBORATED — followed somewhere by
// a quoted literal (instance evidence) or opened by a quantifier
fn implicit_nouns(stmts: &[String]) -> std::collections::HashSet<String> {
    use std::collections::HashSet;
    let quantifiers = ["each", "some", "every", "no", "any"];
    let mut candidates: HashSet<String> = HashSet::new();
    let mut corroborated: HashSet<String> = HashSet::new();
    for s in stmts {
        let s = strip_annotation(s);
        let bare = blank_spans(&s, " '' ");
        if bare.contains(',') || bare.contains('(') || bare.contains(')') {
            continue;
        }
        let mut run: Vec<String> = Vec::new();
        let mut after_quant = false;
        let mut prev = String::new();
        for tok in bare.split_whitespace() {
            if tok == "''" || tok == "''." {
                if !run.is_empty() {
                    let name = run.join(" ");
                    candidates.insert(name.clone());
                    corroborated.insert(name); // instance evidence
                }
                run.clear();
                after_quant = false;
                prev = tok.to_string();
                continue;
            }
            let base = strip_pnc(tok).trim_end_matches(|c: char| c.is_ascii_digit());
            let title = base.chars().next().map_or(false, |c| c.is_uppercase());
            if title && !IMPLICIT_STOP.contains(&base) {
                if run.is_empty() {
                    after_quant =
                        quantifiers.contains(&strip_pnc(&prev).to_lowercase().as_str());
                }
                run.push(base.to_string());
                prev = tok.to_string();
                continue;
            }
            if !run.is_empty() {
                let name = run.join(" ");
                candidates.insert(name.clone());
                if after_quant {
                    corroborated.insert(name); // a quantifier names a TYPE
                }
            }
            run.clear();
            after_quant = false;
            prev = tok.to_string();
        }
        if !run.is_empty() {
            let name = run.join(" ");
            candidates.insert(name.clone());
            if after_quant {
                corroborated.insert(name);
            }
        }
    }
    candidates.intersection(&corroborated).cloned().collect()
}

// _name_refmode (compiler.py:747): strip a (.RefMode) parenthetical
fn name_refmode(text: &str) -> String {
    let t = text.trim();
    if t.ends_with(')') {
        for (p, _) in t.match_indices("(.") {
            if p >= 1 && p + 2 < t.len() - 1 {
                return t[..p].to_string();
            }
        }
    }
    t.to_string()
}

// _known (compiler.py:529): the declared type names, plus the implicit nouns
fn known_names(stmts: &[String]) -> std::collections::HashSet<String> {
    let mut names: std::collections::HashSet<String> = std::collections::HashSet::new();
    for s in stmts {
        let (k, g) = classify_kind(s);
        match k {
            "entity_type" | "value_type" => {
                names.insert(name_refmode(&g[0]));
            }
            "ref_scheme" => {
                names.insert(g[0].clone());
                names.insert(g[1].clone());
            }
            "objectification" => {
                names.insert(g[1].clone());
            }
            "subtype_of" => {
                // a subtype clause DECLARES both names
                names.insert(g[0].clone());
                names.insert(g[1].clone());
            }
            "brace_subtypes" => {
                for x in g[0].split(',') {
                    names.insert(x.trim().to_string());
                }
                names.insert(g[2].clone());
            }
            _ => {}
        }
    }
    for n in implicit_nouns(stmts) {
        names.insert(n);
    }
    names
}

// _prose_suspect (compiler.py:372): structural punctuation outside quoted
// spans is the paragraph tell
fn prose_suspect(text: &str) -> bool {
    let bare = blank_spans(text, " ");
    bare.contains(',') || bare.contains('(') || bare.contains(')') || bare.contains(": ")
}

// _strip_derivation (compiler.py:674), the name half: peel a trailing NORMA
// derivation-storage marker
fn strip_derivation_name(text: &str) -> String {
    for mark in [" **", " ++", " *", " +"] {
        if let Some(pre) = text.strip_suffix(mark) {
            return pre.trim().to_string();
        }
    }
    text.to_string()
}

// _atomic_run_guard (compiler.py:577): a noun match whose Title-case
// continuation is not covered by a longer known name is predicate text
fn atomic_run_guard(
    toks: &[&str],
    i: usize,
    kw: &[String],
    known: &std::collections::HashSet<String>,
) -> bool {
    let j = i + kw.len();
    if j >= toks.len() {
        return true;
    }
    let nxt = strip_pnc(toks[j]).trim_end_matches(|c: char| c.is_ascii_digit());
    let title = nxt.chars().next().map_or(false, |c| c.is_uppercase());
    if !(title && !IMPLICIT_STOP.contains(&nxt)) {
        return true; // no Title-case continuation
    }
    let ext = format!("{} {}", kw.join(" "), nxt);
    let pfx = format!("{} ", ext);
    known.iter().any(|k| *k == ext || k.starts_with(&pfx))
}

// the known names pre-split for _reading's longest-first scan (word count
// descending; the lexicographic tiebreak is deterministic where Python's set
// order was arbitrary — equal-length matches at one position are impossible)
fn sort_known(names: &std::collections::HashSet<String>) -> Vec<Vec<String>> {
    let mut ks: Vec<Vec<String>> = names
        .iter()
        .map(|k| k.split_whitespace().map(|w| w.to_string()).collect())
        .collect();
    ks.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    ks
}

// _reading (compiler.py): a fact-type reading → (template, roles) — the
// certified-equal host override of system:reading_parse. Hyphen binding is
// NORMA's (#24): the bound word stays in the template with its one-sided
// touching hyphen consumed (hyphen_tpl), the '--' escape keeps a literal
// hyphen, and the old touching bind ('from-Status' claiming role Status) is
// retired — a touching hyphen is just a word.
fn reading_split(
    text: &str,
    known_sorted: &[Vec<String>],
    known: &std::collections::HashSet<String>,
) -> (String, Vec<String>) {
    let toks: Vec<&str> = text.split_whitespace().collect();
    let mut roles: Vec<String> = Vec::new();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0usize;
    while i < toks.len() {
        let tok = toks[i];
        let mut matched: Option<&Vec<String>> = None;
        for kw in known_sorted {
            let n = kw.len();
            if n >= 1
                && i + n <= toks.len()
                && toks[i..i + n].iter().zip(kw.iter()).all(|(a, b)| *a == b.as_str())
                && atomic_run_guard(&toks, i, kw, known)
            {
                matched = Some(kw);
                break;
            }
        }
        match matched {
            Some(kw) => {
                roles.push(kw.join(" "));
                out.push(format!("{{{}}}", roles.len() - 1));
                i += kw.len();
            }
            None => {
                out.push(hyphen_tpl(tok));
                i += 1;
            }
        }
    }
    (out.join(" "), roles)
}

// _ftid_from (compiler.py:621): substitute the roles back in and slugify
fn ftid_from(template: &str, roles: &[String]) -> String {
    let mut s = template.to_string();
    for (i, r) in roles.iter().enumerate() {
        s = s.replace(&format!("{{{}}}", i), r);
    }
    slug_str(&s)
}

// _fact_type (compiler.py:634) reduced to the prepass's use: the ftid alone.
// The parallel-ft unification branch is OFF here by construction — the
// prepass passes a plain name set (no subs/fts attrs), exactly as Python does.
fn fact_type_slug(
    reading: &str,
    known_sorted: &[Vec<String>],
    known: &std::collections::HashSet<String>,
) -> String {
    let (template, roles) = reading_split(reading, known_sorted, known);
    ftid_from(&template, &roles)
}

// _prepass_context (compiler.py:416): subtype edges (closed transitively),
// declared fact-type slugs, and the PLAIN reading declarations
#[allow(clippy::type_complexity)]
fn prepass_context(
    stmts: &[String],
    names: &std::collections::HashSet<String>,
    extra_edges: &[(String, String)],
    extra_fts: &[String],
) -> (
    std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
    std::collections::BTreeSet<String>,
    std::collections::BTreeSet<String>,
) {
    use std::collections::{BTreeMap, BTreeSet};
    let known_sorted = sort_known(names);
    let mut edges: Vec<(String, String)> = extra_edges.to_vec();
    let mut fts: BTreeSet<String> = extra_fts.iter().cloned().collect();
    let mut plain: BTreeSet<String> = extra_fts.iter().cloned().collect();
    for s in stmts {
        let (k, g) = classify_kind(s);
        match k {
            "subtype_of" => {
                edges.push((g[0].trim().to_string(), g[1].trim().to_string()));
            }
            "brace_subtypes" => {
                for sub in g[0].split(',') {
                    edges.push((sub.trim().to_string(), g[2].trim().to_string()));
                }
            }
            "fact_type_reading" => {
                if !g[0].contains('\'') && !prose_suspect(&g[0]) {
                    let ft = fact_type_slug(
                        &strip_derivation_name(&g[0]),
                        &known_sorted,
                        names,
                    );
                    fts.insert(ft.clone());
                    plain.insert(ft);
                }
            }
            "rule_if" | "rule_iff" => {
                // a rule HEAD is a declaration (NORMA's starred reading)
                let head = if k == "rule_if" { &g[0] } else { &g[1] };
                let cleaned: String =
                    head.chars().filter(|c| !c.is_ascii_digit()).collect();
                let ft = fact_type_slug(cleaned.trim(), &known_sorted, names);
                fts.insert(ft);
            }
            "uniqueness" => {
                let ft = fact_type_slug(
                    &format!("{} {}", g[0], g[2]),
                    &known_sorted,
                    names,
                );
                fts.insert(ft.clone());
                plain.insert(ft);
            }
            "mandatory" => {
                let ft = fact_type_slug(
                    &format!("{} {}", g[0], g[1]),
                    &known_sorted,
                    names,
                );
                fts.insert(ft.clone());
                plain.insert(ft);
            }
            _ => {}
        }
    }
    let mut parents: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (a, b) in &edges {
        parents.entry(a.clone()).or_default().insert(b.clone());
    }
    let mut closure: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for start in parents.keys() {
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut todo: Vec<String> = vec![start.clone()];
        while let Some(cur) = todo.pop() {
            if let Some(ps) = parents.get(&cur) {
                for p in ps {
                    if seen.insert(p.clone()) {
                        todo.push(p.clone());
                    }
                }
            }
        }
        closure.insert(start.clone(), seen);
    }
    (closure, fts, plain)
}

// _context_of (compiler.py:404): the known context READ OFF a compiled store —
// declared type names, subtype edges, fact-type slugs — so a model compiles
// ATOP a preloaded base. The resident op's base is the RESIDENT store, asked
// for via {"context_from": "resident"}.
#[allow(clippy::type_complexity)]
fn context_of(
    cells: &[(Leaf, V)],
) -> (
    std::collections::HashSet<String>,
    Vec<(String, String)>,
    std::collections::HashSet<String>,
    std::collections::HashSet<String>,
) {
    let leaf = |s: &str| Leaf::S(s.to_string());
    let strv = |x: &V| aval(x).and_then(|l| leaf_str(&l));
    let mut names: std::collections::HashSet<String> = std::collections::HashSet::new();
    // #31: the VALUE-TYPE names ride the context too — a quoted instance-fact
    // literal filling a value-typed role coerces via _num at the cook boundary
    let mut vals: std::collections::HashSet<String> = std::collections::HashSet::new();
    for r in pop_rows(cells, &leaf("instanceOf")) {
        let it = items(&list_of(&r));
        if it.len() >= 2 {
            if let (Some(a), Some(b)) = (strv(&it[0]), strv(&it[1])) {
                if b == "ObjectType" || b == "ValueType" {
                    if b == "ValueType" {
                        vals.insert(a.clone());
                    }
                    names.insert(a);
                }
            }
        }
    }
    let mut fts: std::collections::HashSet<String> = std::collections::HashSet::new();
    for r in pop_rows(cells, &leaf("factType")) {
        let it = items(&list_of(&r));
        if let Some(f) = it.first().and_then(|x| strv(x)) {
            fts.insert(f);
        }
    }
    let mut edges: Vec<(String, String)> = Vec::new();
    for r in pop_rows(cells, &leaf("subtype")) {
        let it = items(&list_of(&r));
        if it.len() >= 2 {
            if let (Some(a), Some(b)) = (strv(&it[0]), strv(&it[1])) {
                edges.push((a, b));
            }
        }
    }
    (names, edges, fts, vals)
}

// _known_vals (compiler.py, #31): the VALUE-TYPE names declared in-text —
// explicit value-type readings plus each reference scheme's identifying value
fn known_vals(stmts: &[String]) -> std::collections::HashSet<String> {
    let mut vals: std::collections::HashSet<String> = std::collections::HashSet::new();
    for s in stmts {
        let (k, g) = classify_kind(s);
        match k {
            "value_type" => {
                vals.insert(name_refmode(&g[0]));
            }
            "ref_scheme" => {
                vals.insert(g[1].clone());
            }
            _ => {}
        }
    }
    vals
}

// ======================= the grammar thaw (gap 1) =============================
// Python classifies over grammar_D() — shared/forml2-grammar.md ingested once
// and frozen-thawed per process (compiler.grammar_D → persist.ingest_frozen,
// a content-keyed sqlite snapshot). The resident's equivalent snapshot is a
// SERVE-PROTOCOL SIDECAR: the exact payload Registry._sidecar writes beside
// every app db ({"d": …, "process": …, "overrides": 1, "cases": []}), holding
// the compiled grammar store. CONVENTION: <root>/shared/forml2-grammar.store.json
// beside the grammar source itself, <root> found by walking up from the
// executable exactly as find_cli walks to cli.py (the exe lives under
// <root>/rust/target/<profile>/). An explicit "grammar_sidecar" op arg
// overrides the walk. When the file is absent, generate it via the Python
// engine (the same bootstrap cli.py performs):
//   python -c "import importlib.util,sys,os,json; root=r'<repo>/engine'; \
//     spec=importlib.util.spec_from_file_location('pyarest', \
//       os.path.join(root,'python','__init__.py'), \
//       submodule_search_locations=[os.path.join(root,'python')]); \
//     m=importlib.util.module_from_spec(spec); sys.modules['pyarest']=m; \
//     spec.loader.exec_module(m); import pyarest.prims; \
//     from pyarest import forml, defs, polyglot; from pyarest.lam import from_lam; \
//     D=forml.grammar_D(); \
//     proc=[[n,polyglot._conv(from_lam(o))] for n,(k,o) in defs.latest.items() if k=='compiled']; \
//     p=os.path.join(root,'shared','forml2-grammar.store.json'); \
//     json.dump({'d':polyglot._conv(from_lam(D)),'process':proc,'overrides':1,'cases':[]}, \
//       open(p,'w',encoding='utf-8'), ensure_ascii=False)"
// The thawed store lives ONLY in the classification scratch: op_compile_model
// swaps it in for the batch derive and restores the resident store whole.
type GrammarScratch = (V, Vec<(Leaf, V)>, N, Vec<(Leaf, N)>, Vec<(String, N)>);

fn load_grammar_scratch(j: &J) -> Result<(GrammarScratch, String), String> {
    let path: std::path::PathBuf = match jget(j, "grammar_sidecar") {
        Some(J::S(p)) => std::path::PathBuf::from(p),
        Some(_) => return Err("grammar_sidecar must be a string path".to_string()),
        None => {
            let exe = std::env::current_exe().map_err(|e| {
                format!("no executable path to walk for the grammar sidecar: {}", e)
            })?;
            let mut found: Option<std::path::PathBuf> = None;
            for dir in exe.ancestors().skip(1) {
                let cand = dir.join("shared").join("forml2-grammar.store.json");
                if cand.is_file() {
                    found = Some(cand);
                    break;
                }
            }
            match found {
                Some(p) => p,
                None => {
                    return Err(
                        "grammar store not resident and no grammar sidecar found: pass \
                         grammar_sidecar or generate <root>/shared/forml2-grammar.store.json \
                         from the Python engine (forml.grammar_D() serialized as \
                         Registry._sidecar does — see the comment above load_grammar_scratch)"
                            .to_string(),
                    )
                }
            }
        }
    };
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("unreadable grammar sidecar {}: {}", path.display(), e))?;
    let payload = match parse_json(&text) {
        Some(p) if jget(&p, "d").is_some() => p,
        _ => return Err(format!("unparseable grammar sidecar {}", path.display())),
    };
    let dj = jget(&payload, "d").unwrap();
    let gd = to_v(dj);
    let gcells = cells_of(&gd);
    let gnd = j_to_n(dj);
    let gncells = n_cells_of(&gnd);
    let mut gproc: Vec<(String, N)> = Vec::new();
    if let Some(J::A(procs)) = jget(&payload, "process") {
        for entry in procs {
            if let J::A(pair) = entry {
                if pair.len() >= 2 {
                    if let J::S(name) = &pair[0] {
                        gproc.push((name.clone(), j_to_n(&pair[1])));
                    }
                }
            }
        }
    }
    Ok(((gd, gcells, gnd, gncells, gproc), path.display().to_string()))
}

// the grammar data read off a store's cells: the dispatch table
// (Classification_has_Translator) and the stage-1 vocabulary (classLit)
#[allow(clippy::type_complexity)]
fn grammar_tables(
    cells: &[(Leaf, V)],
) -> (
    std::collections::HashMap<String, Vec<String>>,
    Vec<(String, String)>,
) {
    let leaf = |s: &str| Leaf::S(s.to_string());
    let strv = |x: &V| aval(x).and_then(|l| leaf_str(&l));
    let mut dispatch: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for r in pop_rows(cells, &leaf("Classification_has_Translator")) {
        let it = items(&list_of(&r));
        if it.len() >= 2 {
            if let (Some(c), Some(t)) = (strv(&it[0]), strv(&it[1])) {
                dispatch.entry(c).or_default().push(t);
            }
        }
    }
    let mut vocab: Vec<(String, String)> = Vec::new();
    for r in pop_rows(cells, &leaf("classLit")) {
        let it = items(&list_of(&r));
        if it.len() >= 2 {
            if let (Some(a), Some(b)) = (strv(&it[0]), strv(&it[1])) {
                vocab.push((a, b));
            }
        }
    }
    (dispatch, vocab)
}

// ============================ the model_d fold (#20, the port after cooks) ===
// meta.initial_D (compiler.py:71) / run_append (engine.py:224) / ast:DefineIn
// (engine.py:71, shared/ast.canon:58), native twins. op_compile_model's
// dispatch loop already produces per-statement Fires (cooks::translate,
// #20 cooks); this section folds them into an actual store, in emission
// order, for byte parity with python's g() (compiler.py's
// _stmt_translator_impl, ~:2240 — asserts THEN objs, per fire).
//
// Shape: a Vec<(Leaf, V)> in cell order (srv.cells' own shape), the same
// first-match-wins find. The re-top move is remove-first-match then
// push_front — the same complexity argument as engine.py:224's docstring
// (hot cells re-top to the front on first touch, so idx stays small). No
// sorting, no unordered maps: cell order and row order are deterministic
// consequences of these moves alone.

// initial_D(): one FILE cell, PHI contents (to_lam(()) on an empty tuple —
// SEQ(NIL), exactly phi()).
fn initial_d_cells() -> Vec<(Leaf, V)> {
    vec![(Leaf::S("FILE".to_string()), phi())]
}

// The RAW cell sequence off a Scott D, duplicates preserved — UNLIKE
// cells_of (which keeps only the first match per name, correct for its own
// resident-lookup-cache job but wrong as a fold SEED: it would silently
// drop a shadowed same-named cell). context_from:"resident" must seed from
// the store's actual sequence, or a later re-top could disagree with a
// dump of the untouched resident D underneath it.
fn raw_cells_of(d: &V) -> Vec<(Leaf, V)> {
    let mut out: Vec<(Leaf, V)> = Vec::new();
    for c in items(&list_of(d)) {
        let it = items(&list_of(&c));
        if it.len() == 3 {
            if let Some(l0) = aval(&it[0]) {
                if matches!(&*l0, Leaf::S(s) if s == "CELL") {
                    if let Some(k) = aval(&it[1]) {
                        out.push(((*k).clone(), it[2].clone()));
                    }
                }
            }
        }
    }
    out
}

// materialize the fold state as a Scott D: SEQ of ⟨CELL, name, contents⟩,
// front to back — the exact shape cells_of/write_v/reduce_over expect.
fn cells_to_d(cells: &[(Leaf, V)]) -> V {
    seq(from_vec(
        cells
            .iter()
            .map(|(name, contents)| {
                seq(from_vec(vec![
                    atom(Leaf::S("CELL".to_string())),
                    atom(name.clone()),
                    contents.clone(),
                ]))
            })
            .collect(),
    ))
}

// ast:Store / DefineIn's shared move: ↓name — pop THEN push (Backus
// §13.3.4/§13.3.5 verbatim). Pop removes the FIRST cell named `name` only;
// deeper same-named cells survive untouched underneath (first-match-wins
// reads see the new top). Absent name: pop is a no-op, so this degrades to
// a plain prepend.
fn store_move(cells: &mut Vec<(Leaf, V)>, name: &str, contents: V) {
    if let Some(idx) = cells
        .iter()
        .position(|(k, _)| matches!(k, Leaf::S(s) if s == name))
    {
        cells.remove(idx);
    }
    cells.insert(0, (Leaf::S(name.to_string()), contents));
}

// run_append (engine.py:224 — mirrored precisely): D' = Store(cell):
// ⟨resolve_default(fact, FetchPop(cell, D)), D⟩. Absent cell -> fresh
// singleton population, cell PREPENDED. Present -> _eqobj dedup
// (type-strict: 1 ≠ "1" ≠ 1.0, the cook differential's own discipline)
// REUSES the old population unchanged on a hit; otherwise the fact
// PREPENDS. Either way the cell re-tops via store_move. Non-plain contents
// (the decoded shape isn't a Seq) is python's canonical-`run`-fallback arm;
// every compile-time cell here is plain by construction — a hit means a
// genuine port gap, so this errors loudly rather than silently diverging.
fn run_append_native(
    fact: &cooks::Val,
    cells: &mut Vec<(Leaf, V)>,
    cell_name: &str,
) -> Result<(), String> {
    let fact_v = cooks::val_to_v(fact);
    let found = cells
        .iter()
        .position(|(k, _)| matches!(k, Leaf::S(s) if s == cell_name));
    let newpop = match found {
        None => seq(from_vec(vec![fact_v])),
        Some(idx) => match shape(&cells[idx].1) {
            Shape::Seq(l) => {
                let pop = items(&l);
                if pop.iter().any(|y| eqobj(&fact_v, y)) {
                    cells[idx].1.clone()
                } else {
                    let mut rows = Vec::with_capacity(pop.len() + 1);
                    rows.push(fact_v);
                    rows.extend(pop);
                    seq(from_vec(rows))
                }
            }
            _ => {
                return Err(format!(
                    "run_append: cell {:?} holds non-plain contents (canonical \
                     `run` fallback not ported for compile-time folds)",
                    cell_name
                ));
            }
        },
    };
    store_move(cells, cell_name, newpop);
    Ok(())
}

// DefineIn (engine.py:71 / shared/ast.canon's ast:DefineIn): the definition
// travels with the store as an ORDINARY cell — the SAME Store move as
// run_append's re-top, but the contents is the definition VALUE verbatim
// (no dedup, no population semantics). The cooks already deliver `obj` as
// the reduced canonical value (from_lam-equal proven); store it as-is.
fn define_in_native(name: &str, obj: &V, cells: &mut Vec<(Leaf, V)>) {
    store_move(cells, name, obj.clone());
}

// One Fire's fold: asserts THEN objs, each list in emission order
// (compiler.py's g(): "for cell,fact in asserts: D = ast.run_append(...);
// for name,obj in objs: D = DefineIn(...)").
fn fold_fire(fire: &cooks::Fire, cells: &mut Vec<(Leaf, V)>) -> Result<(), String> {
    for (cell, fact) in &fire.asserts {
        run_append_native(fact, cells, cell)?;
    }
    for (name, obj) in &fire.objs {
        define_in_native(name, obj, cells);
    }
    Ok(())
}

// ======================= rekey_transitions (#20, native pipeline tail slice 1) ===
// engine.py:1608 rekey_transitions, ported whole (NOT canon-backed: its body is
// pure host list/dict manipulation over from_lam(D), no _apply/reduce call
// anywhere in it — confirmed against every DEF in shared/*.canon; none is named
// or shaped for this). Machine-scope each Transition's IDENTITY: Core.png/
// GraphDL model Transition(.id) as a SURROGATE, not the readings' name, so a
// base-vs-app reuse of a transition NAME must not merge one entity carrying
// two machines' froms/tos. Runs PER COMPILE PASS (the base is rekeyed first,
// frozen; an app compiling atop it via context_from:"resident" sees the
// base's transitions ALREADY surrogate-keyed and skips them, so the name->SMD
// map stays unambiguous per pass): a bare-named transition gets the surrogate
// "txn:{SMD}\x1f{name}" keyed by its defined-in SMD; rows already
// surrogate-keyed are skipped. Rewrites EVERY Transition-typed position — the
// role metamodel's declared referencing fact types PLUS the hardcoded
// machinery cells (smFrom/smTo/smTrigger/smGuard/smEmit/smMoore) and
// Guard_prevents_Transition — so no reference dangles. A bare name mapping to
// more than one SMD in one pass (genuinely ambiguous) is left as-is, never a
// partial rekey.
const TXN_SUR: &str = "txn:";

fn rekey_transitions_native(cells: &mut Vec<(Leaf, V)>) {
    use std::collections::HashSet;
    let leaf = |s: &str| Leaf::S(s.to_string());
    // name_smd: bare transition-name key -> (name V, smd V), built from
    // Transition_is_defined_in_State_Machine_Definition rows; a name seen
    // with two DIFFERENT smd values anywhere in the population is ambiguous
    // and excluded whole (python's unconditional overwrite-then-pop)
    let mut name_smd: HashMap<String, (V, V)> = HashMap::new();
    let mut ambiguous: HashSet<String> = HashSet::new();
    for r in pop_rows(cells, &leaf("Transition_is_defined_in_State_Machine_Definition")) {
        let it = items(&list_of(&r));
        if it.len() >= 2 {
            let already_sur = match aval(&it[0]) {
                Some(l) => leaf_text(&l).starts_with(TXN_SUR),
                None => false,
            };
            if !already_sur {
                let k = key_of(&it[0]);
                if let Some((_, existing_smd)) = name_smd.get(&k) {
                    if !eqobj(existing_smd, &it[1]) {
                        ambiguous.insert(k.clone());
                    }
                }
                name_smd.insert(k, (it[0].clone(), it[1].clone()));
            }
        }
    }
    for k in &ambiguous {
        name_smd.remove(k);
    }
    if name_smd.is_empty() {
        return;
    }
    // surro: bare-name key -> the surrogate atom, ready to substitute in place
    let mut surro: HashMap<String, V> = HashMap::new();
    for (k, (nm, smd)) in &name_smd {
        if let (Some(nl), Some(sl)) = (aval(nm), aval(smd)) {
            let sur = format!("{}{}\x1f{}", TXN_SUR, leaf_text(&sl), leaf_text(&nl));
            surro.insert(k.clone(), atom(Leaf::S(sur)));
        }
    }
    // pos_of: fact-type-name key -> 0-based Transition column position, from
    // the role metamodel's Transition-typed declarations plus the hardcoded
    // machinery cells (python's pos_of.update literal, unconditional so it
    // overrides any role-derived entry for the same name)
    let mut pos_of: HashMap<String, i64> = HashMap::new();
    for r in pop_rows(cells, &leaf("role")) {
        let it = items(&list_of(&r));
        if it.len() >= 4 {
            let is_transition =
                matches!(aval(&it[3]).as_deref(), Some(Leaf::S(s)) if s == "Transition");
            if is_transition {
                if let Some(Leaf::I(p)) = aval(&it[2]).as_deref() {
                    pos_of.insert(key_of(&it[1]), *p - 1);
                }
            }
        }
    }
    for (name, pos) in [
        ("smFrom", 0i64),
        ("smTo", 0),
        ("smTrigger", 0),
        ("smGuard", 0),
        ("smEmit", 0),
        ("smMoore", 0),
        ("Guard_prevents_Transition", 1),
    ] {
        pos_of.insert(key_of(&atom(leaf(name))), pos);
    }
    // the walk: every cell in D, in place; only a cell named in pos_of has
    // its rows visited, and only the row's value AT that column, when it is
    // a bare name in surro, is replaced — everything else copies through
    for i in 0..cells.len() {
        let nk = key_of(&atom(cells[i].0.clone()));
        let p = match pos_of.get(&nk) {
            Some(&pp) if pp >= 0 => pp as usize,
            _ => continue,
        };
        let rows = items(&list_of(&cells[i].1));
        if rows.is_empty() {
            continue;
        }
        let mut changed = false;
        let mut new_rows: Vec<V> = Vec::with_capacity(rows.len());
        for row in rows {
            let mut out_row = row.clone();
            if let Shape::Seq(rl) = shape(&row) {
                let mut cols = items(&rl);
                if cols.len() > p {
                    let ck = key_of(&cols[p]);
                    if let Some(sur) = surro.get(&ck) {
                        cols[p] = sur.clone();
                        out_row = seq(from_vec(cols));
                        changed = true;
                    }
                }
            }
            new_rows.push(out_row);
        }
        if changed {
            cells[i].1 = seq(from_vec(new_rows));
        }
    }
}

// ======================= status_facts / machine_fold / layout_cells ==========
// (#20, the machine_fold port slice: docs/2026-07-11-machine-fold-port-spec.md)
// The protocol tail after the landed post-model fixpoint (protocol.py:1760-
// 1801): status_facts (engine.py:3516) -> machine_fold (engine.py:2780) ->
// run_rules IFF machine_fold changed anything -> layout_cells (engine.py:1693).
// replay (log-carried events) is explicitly NOT in this slice; these corpora's
// machine events arrive as READINGS (instance facts of trigger fact types),
// which status_facts/machine_fold read directly off the fact populations.
//
// The judgment-bearing pieces are ALREADY CANON (the spec's table): sm_triples
// via system:sm_join, the RMAP partition via system:partition, table columns
// via system:table_columns, the governed player via system:governed_player,
// and an absorbed fact type's reassembled population via system:ftpop_absorbed
// -- every helper below evaluates these through the resident NEval, the SAME
// pattern the view_menu/actions sites (main.rs, "actions" op) and op_run_rules's
// own canon-first partition fallback already use. These are NEW, STANDALONE
// helpers (not a refactor of that fallback block or of op_run_rules) -- the
// CONSTRAINT is to leave cooks.rs, the model_d fold functions, rekey, and the
// landed reassembly semantics untouched; duplicating the small amount of
// canon-eval plumbing costs nothing and keeps zero risk to those proven paths.

fn mf_na(s: &str) -> N {
    N::A(Rc::new(Leaf::S(s.to_string())))
}

// mf_lkey renders a leaf as a type-strict SET/MAP key (python's native
// hash/eq: an int and a float of equal value coalesce, a numeric-looking
// string never does) -- the existing set_key/key_of encoding, reused as-is
// for entity keys that may be int- or string-reference-moded per noun.
fn mf_lkey(l: &Leaf) -> String {
    key_of(&atom(l.clone()))
}

// mf_leaf_cmp orders two leaves the way Python's native `<` would for a
// per-noun-homogeneous entity key column (int-vs-int numeric, str-vs-str
// lexical, int-vs-float numeric like Python allows); a genuinely mixed
// str/num pair (which would raise TypeError in Python's own sorted() and so
// never arises on a corpus this differential accepts) falls back to a
// deterministic text compare rather than panicking.
fn mf_leaf_cmp(a: &Leaf, b: &Leaf) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a, b) {
        (Leaf::I(x), Leaf::I(y)) => x.cmp(y),
        (Leaf::F(x), Leaf::F(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (Leaf::I(x), Leaf::F(y)) => (*x as f64).partial_cmp(y).unwrap_or(Ordering::Equal),
        (Leaf::F(x), Leaf::I(y)) => x.partial_cmp(&(*y as f64)).unwrap_or(Ordering::Equal),
        (Leaf::S(x), Leaf::S(y)) => x.cmp(y),
        _ => leaf_text(a).cmp(&leaf_text(b)),
    }
}

// system:partition applied to D whole (rmap_partition, engine.py:1658): the
// ⟨table, ft⟩ canon pairs inverted to ft->table (python's `part`), plus the
// SAME pairs re-paired as ⟨ft, table⟩ (python's `partition.items()`, the
// operand table_columns' second argument expects) -- built ONCE per fold and
// threaded everywhere, mirroring rmap_partition's own per-D memoization intent
// without needing a weak-key cache here (one machine_fold/layout_cells call
// each, per compile).
fn mf_partition(ev: &NEval, nd: &N) -> (std::collections::HashMap<String, String>, V) {
    let pairs_v = n_to_v(&ev.mu(napp(mf_na("system:partition"), nd.clone())));
    let mut part: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut ft_table_pairs: Vec<V> = Vec::new();
    if let Shape::Seq(l) = shape(&pairs_v) {
        for p in items(&l) {
            let it = items(&list_of(&p));
            if it.len() >= 2 {
                if let (Some(t), Some(f)) = (aval(&it[0]), aval(&it[1])) {
                    part.insert(leaf_text(&f), leaf_text(&t));
                    ft_table_pairs.push(seqc(vec![atom((*f).clone()), atom((*t).clone())]));
                }
            }
        }
    }
    (part, seq(from_vec(ft_table_pairs)))
}

// table_columns(partition, table) (engine.py:2621): system:table_columns
// applied to ⟨table, ft-table pairs⟩, the absorbed fact types in column
// order (column j+2 == cols[j]).
fn mf_table_columns(ev: &NEval, table: &str, pairs_v: &V) -> Vec<String> {
    let cols_v = n_to_v(&ev.mu(napp(
        napp(mf_na("system:table_columns"), mf_na(table)),
        v_to_n(pairs_v),
    )));
    let mut out = Vec::new();
    if let Shape::Seq(l) = shape(&cols_v) {
        for c in items(&l) {
            if let Some(f) = aval(&c) {
                out.push(leaf_text(&f));
            }
        }
    }
    out
}

// the unary check every RMAP-absorbed read/write threads (engine.py's
// `max((r[2] for r in _pop_rows(D,"role") if len(r)>=3 and r[1]==ft),
// default=2) == 1`) -- note len(r)>=3, NOT >=4: only r[1]/r[2] are read here.
fn mf_role_max_pos(cells: &[(Leaf, V)], ft: &str) -> i64 {
    let leaf_role = Leaf::S("role".to_string());
    let mut mx: Option<i64> = None;
    for r in pop_rows(cells, &leaf_role) {
        let it = items(&list_of(&r));
        if it.len() >= 3 {
            if aval(&it[1]).map(|l| leaf_text(&l)).as_deref() == Some(ft) {
                if let Some(Leaf::I(p)) = aval(&it[2]).as_deref() {
                    mx = Some(mx.map_or(*p, |m| m.max(*p)));
                }
            }
        }
    }
    mx.unwrap_or(2)
}

// ft_view (engine.py:2671): an own-table fact type short-circuits to its
// pop (host-side, no canon eval needed -- FetchPop applied to D IS the pop);
// an absorbed one reassembles via system:ftpop_absorbed⟨table,col⟩ applied
// to D, then a unary fact type's boolean column reshapes (k,"T") -> (k,)
// host-side, filtering v != "T" out entirely -- the spec's exact contract.
fn mf_ft_view(
    cells: &[(Leaf, V)],
    ev: &NEval,
    nd: &N,
    ft: &str,
    part: &std::collections::HashMap<String, String>,
    pairs_v: &V,
) -> Vec<V> {
    let table = part.get(ft).cloned().unwrap_or_else(|| ft.to_string());
    if table == ft {
        return pop_rows(cells, &Leaf::S(ft.to_string()));
    }
    let cols = mf_table_columns(ev, &table, pairs_v);
    let col = 2 + cols.iter().position(|c| c == ft).unwrap_or(0);
    let unary = mf_role_max_pos(cells, ft) == 1;
    let pairs = n_to_v(&ev.mu(napp(
        napp(
            mf_na("system:ftpop_absorbed"),
            nseq(vec![mf_na(&table), N::A(Rc::new(Leaf::I(col as i64)))]),
        ),
        nd.clone(),
    )));
    let mut out = Vec::new();
    if let Shape::Seq(l) = shape(&pairs) {
        for r in items(&l) {
            let it = items(&list_of(&r));
            if unary {
                if it.len() >= 2 && matches!(aval(&it[1]).as_deref(), Some(Leaf::S(s)) if s == "T")
                {
                    out.push(seqc(vec![it[0].clone()]));
                }
            } else {
                out.push(r);
            }
        }
    }
    out
}

// _governed_player (engine.py:3024): system:governed_player applied to the
// PAIR ⟨ft, D⟩ (one application, not curried) -- the empty tuple means no
// governed player; otherwise the result is the player's name atom.
fn mf_governed_player(ev: &NEval, nd: &N, ft: &str) -> Option<String> {
    let r = ev.mu(napp(mf_na("system:governed_player"), nseq(vec![mf_na(ft), nd.clone()])));
    match r {
        N::A(l) => Some(leaf_text(&l)),
        _ => None,
    }
}

// sm_triples (engine.py:1536): system:sm_join applied to the 3-tuple of
// smFrom/smTrigger/smTo POPULATIONS (already-fetched pops, not cell names) --
// the exact resident-evaluator pattern the view_menu/actions sites use.
fn mf_sm_triples(cells: &[(Leaf, V)], ev: &NEval) -> Vec<(String, String, String)> {
    let pops: Vec<N> = ["smFrom", "smTrigger", "smTo"]
        .iter()
        .map(|c| v_to_n(&seq(from_vec(pop_rows(cells, &Leaf::S(c.to_string()))))))
        .collect();
    let triples_v = n_to_v(&ev.mu(napp(mf_na("system:sm_join"), nseq(pops))));
    let mut out = Vec::new();
    if let Shape::Seq(l) = shape(&triples_v) {
        for t in items(&l) {
            let it = items(&list_of(&t));
            if it.len() >= 3 {
                if let (Some(f), Some(g), Some(to)) = (aval(&it[0]), aval(&it[1]), aval(&it[2])) {
                    out.push((leaf_text(&f), leaf_text(&g), leaf_text(&to)));
                }
            }
        }
    }
    out
}

// mf_setcell_bare is bulk_absorbed_install's OWN `setcell` local (engine.py
// :2744), over the fold's bare Vec<(Leaf,V)> shape (not the srv d/nd/ncells
// quad setcell_into threads): replace IN PLACE when the name is already a
// cell, APPEND AT THE END when it is not -- deliberately NOT Store's
// remove-then-prepend (the fold's own driver writes use Store; the
// bulk-absorbed writes do not, mirroring each site's actual python primitive
// per the spec).
fn mf_setcell_bare(cells: &mut Vec<(Leaf, V)>, name: &str, contents: V) {
    match cells.iter_mut().find(|(k, _)| matches!(k, Leaf::S(s) if s == name)) {
        Some(slot) => slot.1 = contents,
        None => cells.push((Leaf::S(name.to_string()), contents)),
    }
}

// bulk_absorbed_install (engine.py:2725): the batch install of an absorbed
// fact type's rows onto the entity's 3NF row (fresh rows hole-padded), the
// table index joined, and the ft's view-cache cell unioned. `replace_keys`
// (#20, the replay slice: python's own default is replace_keys=False, a
// UNION -- machine_fold's caller below is the ONLY True; retract/migrate
// never pass it either) governs ONLY the view-cache step: True prunes the
// installed keys' stale view rows before the union (one status per
// entity); False unions raw, no pruning (python's own default -- a
// replayed fact type may legitimately carry more than one historical row
// per key). The per-row 3NF COLUMN write is unaffected either way (always
// last-write-wins within this one call, exactly as python's unconditional
// `row[col-1] = v`).
//
// `rows` carries RAW rows (#20, the replay slice -- the interface widened
// from the fold's (entity, status-string) pairs to python's own
// `_plain_rows(facts)` shape) in the caller's processing order
// (machine_fold: walk-phase then SM-init-phase, each internally sorted;
// replay: log/buffer order) -- that order drives the table index's APPEND
// order for freshly-seen keys, matching python's `tbl.append((k,))` inside
// the per-row loop. The RAW shape is load-bearing two ways python's own
// body makes explicit:
//   - the COLUMN value is `r[1] if len(r) >= 2 else "#"` taken RAW (any
//     ORM leaf type -- an int-valued attribute must stay an int), with the
//     unary override to "T";
//   - the VIEW union is `view |= {tuple(r) for r in rows}` -- the FULL row
//     at its ORIGINAL arity, so a unary fact type's plain/migrate entries
//     land 1-tuples in the view cell, and the real tasks log's mixed-arity
//     migrate batches (1- and 2-element rows of one unary ft in one entry)
//     land mixed -- a reconstructed fixed-arity ⟨key,value⟩ pair here (the
//     first draft's shape) diverges byte-wise on exactly those.
// An empty row is skipped (python `if not r: continue`); a row whose FIRST
// element is not an atom is skipped too (python would use the tuple as a
// dict key and str()-render it into the cell name -- no log writer
// produces such a row, and the native cellkey has no matching rendering,
// so a skip beats a silently wrong-named cell).
fn mf_bulk_absorbed_install(
    cells: &[(Leaf, V)],
    ev: &NEval,
    table: &str,
    ft: &str,
    rows: &[V],
    pairs_v: &V,
    replace_keys: bool,
) -> Vec<(Leaf, V)> {
    use std::collections::HashSet;
    let leaf = |s: &str| Leaf::S(s.to_string());

    let cols = mf_table_columns(ev, table, pairs_v);
    let col = 2 + cols.iter().position(|c| c == ft).unwrap_or(0);
    let width = 1 + cols.len();
    let unary = mf_role_max_pos(cells, ft) == 1;

    let mut out: Vec<(Leaf, V)> = cells.to_vec();
    let hole = || atom(Leaf::S("#".to_string()));

    let mut tbl: Vec<V> = pop_rows(&out, &leaf(table));
    let mut keys: HashSet<String> = tbl
        .iter()
        .filter_map(|r| {
            let it = items(&list_of(r));
            if !it.is_empty() {
                aval(&it[0]).map(|l| mf_lkey(&l))
            } else {
                None
            }
        })
        .collect();

    for r in rows {
        let it = items(&list_of(r));
        if it.is_empty() {
            continue;
        }
        let e = match aval(&it[0]) {
            Some(l) => l,
            None => continue,
        };
        let v = if unary {
            atom(Leaf::S("T".to_string()))
        } else if it.len() >= 2 {
            it[1].clone()
        } else {
            hole()
        };
        let rc = format!("{}:{}", table, leaf_text(&e));
        let mut row: Vec<V> = pop_rows(&out, &leaf(&rc));
        if row.is_empty() {
            row = vec![atom((*e).clone())];
        }
        while row.len() < width {
            row.push(hole());
        }
        row[col - 1] = v;
        mf_setcell_bare(&mut out, &rc, seq(from_vec(row)));
        if keys.insert(mf_lkey(&e)) {
            tbl.push(seqc(vec![atom((*e).clone())]));
        }
    }
    mf_setcell_bare(&mut out, table, seq(from_vec(tbl)));

    // the view-cache cell: replace_keys=true prunes the installed keys'
    // stale rows before the union (machine_fold's one-status-per-entity
    // mode); replace_keys=false (python's own default -- every replay call
    // site) unions raw, no pruning. Either way the fresh rows are added
    // WHOLE at their original arity (python `view |= {tuple(r) for r in
    // rows}` -- never the unary reshape, never a fixed-width rebuild),
    // deduped as a set, then _rowsort (the mixed-type key contract).
    let mut view: Vec<V> = pop_rows(&out, &leaf(ft));
    if replace_keys {
        let installed: HashSet<String> = rows
            .iter()
            .filter_map(|r| {
                let it = items(&list_of(r));
                it.first().and_then(|x| aval(x)).map(|l| mf_lkey(&l))
            })
            .collect();
        view.retain(|r| {
            let it = items(&list_of(r));
            match it.first().and_then(|x| aval(x)) {
                Some(k) => !installed.contains(&mf_lkey(&k)),
                None => true,
            }
        });
    }
    for r in rows {
        // NO empty-row guard here: python's `view |= {tuple(r) for r in
        // rows}` unions every row incl. a pathological empty one -- the
        // `if not r: continue` guard above is the COLUMN loop's alone
        view.push(r.clone());
    }
    let mut seen: HashSet<String> = HashSet::new();
    let mut view_dedup: Vec<V> = Vec::new();
    for r in view {
        if seen.insert(key_of(&r)) {
            view_dedup.push(r);
        }
    }
    sort_rows(&mut view_dedup);
    mf_setcell_bare(&mut out, ft, seq(from_vec(view_dedup)));

    out
}

// machine_fold (engine.py:2780): readings-carried machine events, folded at
// compile. Returns (folded cells, changed) -- changed mirrors protocol.py
// :1842's `D2 is not D` (python's OBJECT IDENTITY: machine_fold returns the
// SAME D untouched when there are no machines at all, or when the walk +
// SM-init produce no writes) so the caller can gate the post-fold run_rules
// the same way.
fn machine_fold_native(cells: &[(Leaf, V)], srv: &Srv) -> (Vec<(Leaf, V)>, bool) {
    use std::collections::{HashMap, HashSet};
    let leaf = |s: &str| Leaf::S(s.to_string());

    let d0 = cells_to_d(cells);
    let nd = v_to_n(&d0);
    let ncells = n_cells_of(&nd);
    let ev = NEval {
        cells: ncells,
        process: srv.nprocess.clone(),
        defs_n: nd.clone(),
        fuel: std::cell::Cell::new(-1),
    };

    let triples = mf_sm_triples(cells, &ev);
    if triples.is_empty() {
        return (cells.to_vec(), false);
    }

    let mut trig_fts: Vec<String> = {
        let mut s: HashSet<String> = HashSet::new();
        for r in pop_rows(cells, &leaf("smTrigger")) {
            let it = items(&list_of(&r));
            if it.len() >= 2 {
                if let Some(f) = aval(&it[1]) {
                    s.insert(leaf_text(&f));
                }
            }
        }
        s.into_iter().collect()
    };
    trig_fts.sort();

    // initials: SMD -> initial status (rows are ⟨status, SMD⟩: r[1]->r[0])
    let mut initials: HashMap<String, String> = HashMap::new();
    for r in pop_rows(cells, &leaf("Status_is_initial_in_State_Machine_Definition")) {
        let it = items(&list_of(&r));
        if it.len() >= 2 {
            if let (Some(status), Some(smd)) = (aval(&it[0]), aval(&it[1])) {
                initials.insert(leaf_text(&smd), leaf_text(&status));
            }
        }
    }
    // status_fts: noun -> status fact type (rows ⟨noun, ft⟩)
    let mut status_fts: HashMap<String, String> = HashMap::new();
    for r in pop_rows(cells, &leaf("smStatusFt")) {
        let it = items(&list_of(&r));
        if it.len() >= 2 {
            if let (Some(n), Some(f)) = (aval(&it[0]), aval(&it[1])) {
                status_fts.insert(leaf_text(&n), leaf_text(&f));
            }
        }
    }
    // machines: noun -> SMD (rows ⟨SMD, noun⟩: r[1]->r[0])
    let mut machines: HashMap<String, String> = HashMap::new();
    for r in pop_rows(cells, &leaf("smDef")) {
        let it = items(&list_of(&r));
        if it.len() >= 2 {
            if let (Some(smd), Some(n)) = (aval(&it[0]), aval(&it[1])) {
                machines.insert(leaf_text(&n), leaf_text(&smd));
            }
        }
    }
    // gov: noun -> governing noun (rows ⟨noun, governor⟩)
    let mut gov: HashMap<String, String> = HashMap::new();
    for r in pop_rows(cells, &leaf("governedBy")) {
        let it = items(&list_of(&r));
        if it.len() >= 2 {
            if let (Some(a), Some(b)) = (aval(&it[0]), aval(&it[1])) {
                gov.insert(leaf_text(&a), leaf_text(&b));
            }
        }
    }

    // ---- event collection (engine.py:2807-2818): BARE pop reads (never
    // ft_view -- the trigger ft's own cell, not the RMAP-reassembled view),
    // keyed by (noun, entity); duplicates matter (two rows of the same
    // trigger ft = two events, appended, never deduped) ----
    let mut events: HashMap<(String, String), Vec<String>> = HashMap::new();
    let mut event_entity: HashMap<(String, String), Leaf> = HashMap::new();
    for ft in &trig_fts {
        let noun = match mf_governed_player(&ev, &nd, ft) {
            Some(n) => n,
            None => continue,
        };
        let pos = pop_rows(cells, &leaf("role")).iter().find_map(|r| {
            let it = items(&list_of(r));
            if it.len() >= 4
                && aval(&it[1]).map(|l| leaf_text(&l)).as_deref() == Some(ft.as_str())
                && aval(&it[3]).map(|l| leaf_text(&l)).as_deref() == Some(noun.as_str())
            {
                if let Some(Leaf::I(p)) = aval(&it[2]).as_deref() {
                    return Some(*p);
                }
            }
            None
        });
        let pos = match pos {
            Some(p) if p >= 1 => p,
            _ => continue,
        };
        let idx = (pos - 1) as usize;
        for row in pop_rows(cells, &leaf(ft)) {
            let it = items(&list_of(&row));
            if it.len() >= pos as usize {
                if let Some(ekey) = aval(&it[idx]) {
                    let et = leaf_text(&ekey);
                    if et.is_empty() || et == "\u{3c6}" {
                        continue;
                    }
                    let k = (noun.clone(), mf_lkey(&ekey));
                    events.entry(k.clone()).or_default().push(ft.clone());
                    event_entity.entry(k).or_insert_with(|| (*ekey).clone());
                }
            }
        }
    }

    let (part, pairs_v) = mf_partition(&ev, &nd);

    // ---- current status, read through ft_view (RMAP-aware: the status ft
    // is usually absorbed by fold time) ----
    let mut current: HashMap<(String, String), String> = HashMap::new();
    let mut current_nouns: Vec<String> =
        events.keys().map(|(n, _)| n.clone()).collect::<HashSet<_>>().into_iter().collect();
    current_nouns.sort();
    for noun in &current_nouns {
        let sft = match status_fts.get(gov.get(noun).unwrap_or(noun)) {
            Some(s) => s.clone(),
            None => continue,
        };
        for row in mf_ft_view(cells, &ev, &nd, &sft, &part, &pairs_v) {
            let it = items(&list_of(&row));
            if it.len() >= 2 {
                if let (Some(k), Some(v)) = (aval(&it[0]), aval(&it[1])) {
                    current.insert((noun.clone(), mf_lkey(&k)), leaf_text(&v));
                }
            }
        }
    }

    // ---- the greedy walk: per (noun, entity) in SORTED order (python's
    // native tuple sort -- numeric for a homogeneous int-keyed noun, lexical
    // for a string-keyed one), fire the FIRST fireable event, remove it,
    // restart the scan; stop when a full scan fires nothing ----
    struct ChangedRow {
        sft: String,
        ekey: String,
        entity: Leaf,
        status: String,
    }
    let mut changed: Vec<ChangedRow> = Vec::new();

    let mut event_keys: Vec<(String, String)> = events.keys().cloned().collect();
    event_keys.sort_by(|a, b| {
        a.0.cmp(&b.0).then_with(|| mf_leaf_cmp(&event_entity[a], &event_entity[b]))
    });

    for k in &event_keys {
        let (noun, ekey) = k;
        let sft = match status_fts.get(gov.get(noun).unwrap_or(noun)) {
            Some(s) => s.clone(),
            None => continue,
        };
        let m: Option<String> = machines
            .get(gov.get(noun).unwrap_or(noun))
            .or_else(|| machines.get(noun))
            .cloned();
        let start: Option<String> = current.get(k).cloned().or_else(|| {
            m.as_ref().and_then(|mm| initials.get(mm)).cloned()
        });
        let mut cur = start;
        let mut evs: Vec<String> = events[k].clone();
        evs.sort();
        let mut fired_any = false;
        loop {
            if evs.is_empty() {
                break;
            }
            let mut fired_idx: Option<(usize, String)> = None;
            for (i, ev_ft) in evs.iter().enumerate() {
                if let Some(c) = &cur {
                    if let Some((_, _, to)) =
                        triples.iter().find(|(f, g, _)| g == ev_ft && f == c)
                    {
                        fired_idx = Some((i, to.clone()));
                        break;
                    }
                }
            }
            match fired_idx {
                Some((i, to)) => {
                    cur = Some(to);
                    fired_any = true;
                    evs.remove(i);
                }
                None => break,
            }
        }
        // write iff the machine RAN for this entity -- a round-trip back to
        // the RECORDED current does not write; an entity with no recorded
        // status that walks back to the initial DOES write (current.get is
        // None, never equal to a real status string)
        if fired_any {
            let existing = current.get(k).cloned();
            if cur != existing {
                if let Some(cv) = cur {
                    changed.push(ChangedRow {
                        sft,
                        ekey: ekey.clone(),
                        entity: event_entity[k].clone(),
                        status: cv,
                    });
                }
            }
        }
    }

    // ---- SM init: every governed entity with no status row materializes
    // the machine's initial. Entity source = the noun's own table UNIONED
    // with role-1 keys of every OTHER fact type the noun heads. Keys sorted
    // BY STR (python's `key=str`, unlike the walk's native-type sort) ----
    let mut written: HashSet<(String, String)> =
        changed.iter().map(|c| (c.sft.clone(), c.ekey.clone())).collect();

    let mut machine_pairs: Vec<(String, String)> =
        machines.iter().map(|(n, m)| (n.clone(), m.clone())).collect();
    machine_pairs.sort();

    for (noun, m) in &machine_pairs {
        let sft = match status_fts.get(noun) {
            Some(s) => s.clone(),
            None => continue,
        };
        let init = match initials.get(m) {
            Some(i) => i.clone(),
            None => continue,
        };

        let have: HashSet<String> = mf_ft_view(cells, &ev, &nd, &sft, &part, &pairs_v)
            .iter()
            .filter_map(|r| {
                let it = items(&list_of(r));
                if !it.is_empty() {
                    aval(&it[0]).map(|l| mf_lkey(&l))
                } else {
                    None
                }
            })
            .collect();

        let mut keys: HashMap<String, Leaf> = HashMap::new();
        for r in pop_rows(cells, &leaf(noun)) {
            let it = items(&list_of(&r));
            if !it.is_empty() {
                if let Some(k) = aval(&it[0]) {
                    keys.entry(mf_lkey(&k)).or_insert_with(|| (*k).clone());
                }
            }
        }
        for r in pop_rows(cells, &leaf("role")) {
            let it = items(&list_of(&r));
            if it.len() >= 4
                && matches!(aval(&it[2]).as_deref(), Some(Leaf::I(1)))
                && aval(&it[3]).map(|l| leaf_text(&l)).as_deref() == Some(noun.as_str())
            {
                if let Some(oft) = aval(&it[1]).map(|l| leaf_text(&l)) {
                    if oft != *sft {
                        for x in pop_rows(cells, &leaf(&oft)) {
                            let xit = items(&list_of(&x));
                            if !xit.is_empty() {
                                if let Some(k) = aval(&xit[0]) {
                                    keys.entry(mf_lkey(&k)).or_insert_with(|| (*k).clone());
                                }
                            }
                        }
                    }
                }
            }
        }

        let mut keys_vec: Vec<(String, Leaf)> = keys.into_iter().collect();
        keys_vec.sort_by(|a, b| leaf_text(&a.1).cmp(&leaf_text(&b.1)));

        for (lk, kleaf) in keys_vec {
            let kt = leaf_text(&kleaf);
            // python's `if k and k not in ("", "φ")`: a TRUTHINESS check,
            // not membership alone -- an integer 0 key is falsy too
            let falsy = match &kleaf {
                Leaf::I(i) => *i == 0,
                Leaf::F(f) => *f == 0.0,
                Leaf::S(s) => s.is_empty(),
                Leaf::AppTag => false,
            };
            if falsy || kt == "\u{3c6}" {
                continue;
            }
            if have.contains(&lk) {
                continue;
            }
            if written.contains(&(sft.clone(), lk.clone())) {
                continue;
            }
            written.insert((sft.clone(), lk.clone()));
            changed.push(ChangedRow {
                sft: sft.clone(),
                ekey: lk,
                entity: kleaf,
                status: init.clone(),
            });
        }
    }

    if changed.is_empty() {
        return (cells.to_vec(), false);
    }

    // ---- the commit: group by status ft (preserving each group's relative
    // order), sorted by ft name. Absorbed -> mf_bulk_absorbed_install
    // (replace_keys=true). Own-table -> union-overwrite the pop directly
    // through STORE semantics (python's ast.Store) ----
    let mut by_sft: Vec<(String, Vec<(Leaf, String)>)> = Vec::new();
    {
        let mut idx: HashMap<String, usize> = HashMap::new();
        for c in &changed {
            let i = *idx.entry(c.sft.clone()).or_insert_with(|| {
                by_sft.push((c.sft.clone(), Vec::new()));
                by_sft.len() - 1
            });
            by_sft[i].1.push((c.entity.clone(), c.status.clone()));
        }
    }
    by_sft.sort_by(|a, b| a.0.cmp(&b.0));

    let mut out_cells: Vec<(Leaf, V)> = cells.to_vec();
    for (sft, rows) in &by_sft {
        let table = part.get(sft).cloned().unwrap_or_else(|| sft.clone());
        if &table == sft {
            let new_keys: HashSet<String> = rows.iter().map(|(e, _)| mf_lkey(e)).collect();
            let mut keep: Vec<V> = Vec::new();
            for r in pop_rows(&out_cells, &leaf(sft)) {
                let it = items(&list_of(&r));
                if !it.is_empty() {
                    if let Some(k) = aval(&it[0]) {
                        if !new_keys.contains(&mf_lkey(&k)) {
                            keep.push(r.clone());
                        }
                    }
                }
            }
            let new_rows: Vec<V> = rows
                .iter()
                .map(|(e, cur)| seqc(vec![atom(e.clone()), atom(Leaf::S(cur.clone()))]))
                .collect();
            let mut seen: HashSet<String> = HashSet::new();
            let mut union_rows: Vec<V> = Vec::new();
            for r in keep.into_iter().chain(new_rows.into_iter()) {
                if seen.insert(key_of(&r)) {
                    union_rows.push(r);
                }
            }
            sort_rows(&mut union_rows);
            store_move(&mut out_cells, sft, seq(from_vec(union_rows)));
        } else {
            // #20 (replay slice): mf_bulk_absorbed_install now takes RAW
            // rows (python's own interface -- see its comment); the fold's
            // ⟨entity, status⟩ pairs become the same 2-rows python's
            // machine_fold passes it, replace_keys=true (its one mode)
            let raw_rows: Vec<V> = rows
                .iter()
                .map(|(e, s)| seqc(vec![atom(e.clone()), atom(Leaf::S(s.clone()))]))
                .collect();
            out_cells =
                mf_bulk_absorbed_install(&out_cells, &ev, &table, sft, &raw_rows, &pairs_v, true);
        }
    }

    (out_cells, true)
}

// layout_cells (engine.py:1693): materializes the RMAP layout as data --
// rows ⟨table, 2+j, ft⟩ for every absorbed fact type. Unconditional (no
// machine gating): a no-machine corpus still gets its ORDINARY RMAP
// absorptions laid out. REPLACES any existing rmapColumns cell WHOLESALE --
// python filters the old cell out then appends the fresh one at the END
// (`cells + (new,)`), never Store's re-top-to-front.
fn layout_cells_native(cells: &[(Leaf, V)], srv: &Srv) -> Vec<(Leaf, V)> {
    use std::collections::HashSet;
    let d0 = cells_to_d(cells);
    let nd = v_to_n(&d0);
    let ncells = n_cells_of(&nd);
    let ev = NEval {
        cells: ncells,
        process: srv.nprocess.clone(),
        defs_n: nd.clone(),
        fuel: std::cell::Cell::new(-1),
    };

    let (part, pairs_v) = mf_partition(&ev, &nd);
    let mut tables: Vec<String> = part
        .iter()
        .filter(|(f, t)| f != t)
        .map(|(_, t)| t.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    tables.sort();

    let mut rows: Vec<V> = Vec::new();
    for table in &tables {
        let cols = mf_table_columns(&ev, table, &pairs_v);
        for (j, ft) in cols.iter().enumerate() {
            rows.push(seqc(vec![
                atom(Leaf::S(table.clone())),
                atom(Leaf::I((2 + j) as i64)),
                atom(Leaf::S(ft.clone())),
            ]));
        }
    }

    let mut out: Vec<(Leaf, V)> = cells
        .iter()
        .filter(|(k, _)| !matches!(k, Leaf::S(s) if s == "rmapColumns"))
        .cloned()
        .collect();
    out.push((Leaf::S("rmapColumns".to_string()), seq(from_vec(rows))));
    out
}

// scheduler_cells_native (engine.py:1797 scheduler_cells, #20 the final
// pipeline slice): materializes the SCHEDULE as data -- the passHeads cell,
// rows <pass, head> from classify_heads_native (the SAME classifier
// op_run_rules' absent-cell fallback shares, above it in this file: "share
// the code, do not duplicate"), plus passOrder/passBound evaluated through
// their canon defs (system:pass_order / system:pass_bound, shared/
// system.canon) exactly as python's own
// `from_lam(_ap(_A("system:pass_order"), to_lam(())))` reduces them -- the
// constants of doctrine, materialized beside the membership so a reader
// holds the whole schedule without ever needing kindmap. Recompile replaces
// the three cells wholesale; a store without them classifies/dispatches
// live, which op_run_rules already does (both the with-cell and the
// absent-cell paths).
fn scheduler_cells_native(cells: &[(Leaf, V)], srv: &Srv) -> Vec<(Leaf, V)> {
    let d0 = cells_to_d(cells);
    let nd = v_to_n(&d0);
    let ncells = n_cells_of(&nd);
    let ev = NEval {
        cells: ncells,
        process: srv.nprocess.clone(),
        defs_n: nd.clone(),
        fuel: std::cell::Cell::new(-1),
    };
    let hc = classify_heads_native(cells);
    let mut rows: Vec<V> = Vec::new();
    for (pass_name, list) in [
        ("agg", &hc.agg),
        ("keyed", &hc.keyed),
        ("sweep", &hc.sweep),
        ("dred", &hc.dred),
        ("aggwhole", &hc.aggwhole),
    ] {
        for (_hk, hl) in list {
            rows.push(seqc(vec![atom(Leaf::S(pass_name.to_string())), atom(hl.clone())]));
        }
    }
    let order_v = n_to_v(&ev.mu(napp(mf_na("system:pass_order"), nseq(vec![]))));
    let bound_v = n_to_v(&ev.mu(napp(mf_na("system:pass_bound"), nseq(vec![]))));
    let mut out: Vec<(Leaf, V)> = cells
        .iter()
        .filter(|(k, _)| {
            !matches!(k, Leaf::S(s) if s == "passHeads" || s == "passOrder" || s == "passBound")
        })
        .cloned()
        .collect();
    out.push((Leaf::S("passHeads".to_string()), seq(from_vec(rows))));
    out.push((Leaf::S("passOrder".to_string()), order_v));
    out.push((Leaf::S("passBound".to_string()), bound_v));
    out
}

// format_reading mirrors python's str.format(*players) applied to a FORML
// reading template ("{0} has {1}."-style placeholders): each "{N}"
// substitutes players[N]; "{{"/"}}" escape to a literal brace (python's own
// str.format convention). ANY placeholder whose index is out of range (or
// non-numeric, or unterminated) reverts the WHOLE result to the RAW
// template unchanged -- python's try/except IndexError/KeyError wraps the
// WHOLE .format() call, not a per-placeholder fallback.
fn format_reading(template: &str, players: &[String]) -> String {
    let chars: Vec<char> = template.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '{' if i + 1 < chars.len() && chars[i + 1] == '{' => {
                out.push('{');
                i += 2;
            }
            '}' if i + 1 < chars.len() && chars[i + 1] == '}' => {
                out.push('}');
                i += 2;
            }
            '{' => {
                let mut j = i + 1;
                while j < chars.len() && chars[j] != '}' {
                    j += 1;
                }
                if j >= chars.len() {
                    return template.to_string();
                }
                let idxs: String = chars[i + 1..j].iter().collect();
                match idxs.parse::<usize>() {
                    Ok(idx) if idx < players.len() => {
                        out.push_str(&players[idx]);
                        i = j + 1;
                    }
                    _ => return template.to_string(),
                }
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

// generator_cells_native (engine.py:1825 generator_cells): the generator
// family's base member -- dsl:<Noun> cells, the per-noun model summary
// (noun, object/value kind, sorted verbalized readings, verbalized
// constraints as kind-text pairs, sorted deduped machine transitions). THE
// OPT-IN XSD/OWL/EDM/HTML/DTD/WSDL/XFORMS/PLIX/NAV/Solidity FAMILY
// (engine.py:1881-2200, gated on App_uses_Generator instance facts) IS
// DELIBERATELY NOT PORTED: a repo-wide grep of "uses Generator" against
// every acceptance corpus (naming/kinds/rp-fixture/the tasks app's
// readings) found zero hits, so python's own `active` set is always empty
// and that whole block is dead code for every corpus this slice certifies
// against -- descoped, not silently skipped; see the task report's own
// section. The dsl: cells (the base member every corpus CAN reach) are
// ported in full.
fn generator_cells_native(cells: &[(Leaf, V)], srv: &Srv) -> Vec<(Leaf, V)> {
    use std::collections::HashMap;
    let leaf = |s: &str| Leaf::S(s.to_string());
    let sval = |v: &V| -> Option<String> {
        match aval(v) {
            Some(l) => match &*l {
                Leaf::S(s) => Some(s.clone()),
                _ => None,
            },
            None => None,
        }
    };

    let mut kinds: Vec<(String, &'static str)> = {
        let mut seen: HashMap<String, &'static str> = HashMap::new();
        for r in pop_rows(cells, &leaf("instanceOf")) {
            let it = items(&list_of(&r));
            if it.len() >= 2 {
                if let (Some(n), Some(k)) = (sval(&it[0]), sval(&it[1])) {
                    let kind = if k == "ObjectType" {
                        Some("entity")
                    } else if k == "ValueType" {
                        Some("value")
                    } else {
                        None
                    };
                    if let Some(kind) = kind {
                        seen.insert(n, kind);
                    }
                }
            }
        }
        let mut v: Vec<(String, &'static str)> = seen.into_iter().collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    };

    // roles: ft -> [(pos, player)], sorted per read site (python's own
    // `sorted(roles[ft])`/`sorted(roles.get(ft, []))` at each use)
    let mut roles: HashMap<String, Vec<(i64, String)>> = HashMap::new();
    for r in pop_rows(cells, &leaf("role")) {
        let it = items(&list_of(&r));
        if it.len() >= 4 {
            if let (Some(ft), Some(player)) = (sval(&it[1]), sval(&it[3])) {
                if let Some(Leaf::I(pos)) = aval(&it[2]).as_deref() {
                    roles.entry(ft).or_default().push((*pos, player));
                }
            }
        }
    }
    let sorted_players = |ft: &str| -> Vec<String> {
        let mut ps = roles.get(ft).cloned().unwrap_or_default();
        ps.sort();
        ps.into_iter().map(|(_, p)| p).collect()
    };

    // readings: ft -> the {0}/{1}-formatted reading, only for fact types
    // that actually have roles (python's `if f[0] in roles`)
    let mut readings: HashMap<String, String> = HashMap::new();
    for f in pop_rows(cells, &leaf("factType")) {
        let it = items(&list_of(&f));
        if it.len() >= 2 {
            if let Some(ft) = sval(&it[0]) {
                if roles.contains_key(&ft) {
                    if let Some(l) = aval(&it[1]) {
                        let template = leaf_text(&l);
                        let players = sorted_players(&ft);
                        readings.insert(ft, format_reading(&template, &players));
                    }
                }
            }
        }
    }

    // cons: (players, kind_tag, text) -- UC/MC/deontic verbalization, exactly
    // python's uniqueness/mandatory/deontic* dispatch over constraint rows
    struct ConEntry {
        players: Vec<String>,
        kind_tag: &'static str,
        text: String,
    }
    let mut cons: Vec<ConEntry> = Vec::new();
    for c in pop_rows(cells, &leaf("constraint")) {
        let it = items(&list_of(&c));
        if it.len() < 3 {
            continue;
        }
        let cid = match aval(&it[0]) {
            Some(l) => leaf_text(&l),
            None => continue,
        };
        let kind = match sval(&it[1]) {
            Some(k) => k,
            None => continue,
        };
        let ft = match sval(&it[2]) {
            Some(f) => f,
            None => continue,
        };
        let players = sorted_players(&ft);
        if kind == "uniqueness" && players.len() >= 2 {
            cons.push(ConEntry {
                players: players.clone(),
                kind_tag: "UC",
                text: format!("Each {} has at most one {}.", players[0], players[1]),
            });
        } else if kind == "mandatory" && players.len() >= 2 {
            cons.push(ConEntry {
                players: players.clone(),
                kind_tag: "MC",
                text: format!("Each {} has some {}.", players[0], players[1]),
            });
        } else if kind.starts_with("deontic") {
            cons.push(ConEntry { players, kind_tag: "UC", text: format!("{}.", cid) });
        }
    }

    // sms: noun -> ALL (trigger, from, to) triples of every governed machine
    // (python: a triple carries no machine id, so every governed noun sees
    // the union -- exact for the single-machine case, every app in the
    // fleet today)
    let d0 = cells_to_d(cells);
    let nd = v_to_n(&d0);
    let ncells = n_cells_of(&nd);
    let ev = NEval {
        cells: ncells,
        process: srv.nprocess.clone(),
        defs_n: nd.clone(),
        fuel: std::cell::Cell::new(-1),
    };
    let triples: Vec<(String, String, String)> = mf_sm_triples(cells, &ev)
        .into_iter()
        .map(|(frm, trig, to)| (trig, frm, to))
        .collect();
    let mut sms: HashMap<String, Vec<(String, String, String)>> = HashMap::new();
    for r in pop_rows(cells, &leaf("smDef")) {
        let it = items(&list_of(&r));
        if it.len() >= 2 {
            if let Some(noun) = sval(&it[1]) {
                sms.entry(noun).or_default().extend(triples.iter().cloned());
            }
        }
    }

    // the dsl:<Noun> cells, one row per noun, in noun-sorted order (matching
    // both python's `for noun, kind in sorted(kinds.items())` and the final
    // `sorted(cells.items())` re-sort -- the two coincide since the cell key
    // is always "dsl:" + noun)
    let mut fresh: Vec<(Leaf, V)> = Vec::new();
    for (noun, kind) in &kinds {
        let my_fts: Vec<&String> = roles
            .iter()
            .filter(|(_ft, ps)| ps.iter().any(|(_i, p)| p == noun))
            .map(|(ft, _)| ft)
            .collect();
        let mut my_readings: Vec<String> =
            my_fts.iter().filter_map(|ft| readings.get(*ft).cloned()).collect();
        my_readings.sort();
        let my_cons: Vec<V> = cons
            .iter()
            .filter(|c| c.players.iter().any(|p| p == noun))
            .map(|c| {
                seqc(vec![atom(Leaf::S(c.kind_tag.to_string())), atom(Leaf::S(c.text.clone()))])
            })
            .collect();
        let mut my_trans: Vec<(String, String, String)> = sms.get(noun).cloned().unwrap_or_default();
        my_trans.sort();
        my_trans.dedup();
        let trans_v: Vec<V> = my_trans
            .iter()
            .map(|(a, b, c)| {
                seqc(vec![atom(Leaf::S(a.clone())), atom(Leaf::S(b.clone())), atom(Leaf::S(c.clone()))])
            })
            .collect();
        let row = seqc(vec![
            atom(Leaf::S(noun.clone())),
            atom(Leaf::S(kind.to_string())),
            seq(from_vec(my_readings.into_iter().map(|s| atom(Leaf::S(s))).collect())),
            seq(from_vec(my_cons)),
            seq(from_vec(trans_v)),
        ]);
        fresh.push((Leaf::S(format!("dsl:{}", noun)), seq(from_vec(vec![row]))));
    }

    const GEN_PREFIXES: [&str; 11] = [
        "dsl:", "xsd:", "owl:", "edm:", "html:", "dtd:", "wsdl:", "xforms:", "plix:", "nav:",
        "solidity:",
    ];
    let mut out: Vec<(Leaf, V)> = cells
        .iter()
        .filter(|(k, _)| match k {
            Leaf::S(s) => !GEN_PREFIXES.iter().any(|p| s.starts_with(p)),
            _ => true,
        })
        .cloned()
        .collect();
    out.extend(fresh);
    out
}

// compile_lines_native is a SELF-CONTAINED twin of op_compile_model's own
// dispatch loop (grammar load through rekey_transitions_native), used ONLY
// by status_facts_native's nested "compile these synthesized lines atop the
// in-progress model" call -- the exact recursive shape python's status_facts
// takes (`forml.compile_model(text, D=D, context_from=D)`, engine.py:3541,
// itself compile_model_selfhost + rekey_transitions, NEVER run_rules --
// run_rules is the pipeline's own separate call, not part of compile_model).
// Deliberately a SEPARATE function, not a refactor of op_compile_model's own
// (already byte-parity-certified across ten corpora + identity, by two
// earlier slices) loop -- duplicating this logic costs a couple hundred
// lines but keeps zero risk to that proven path. Every helper it calls
// (split_statements/split_modality/context_of/known_names/known_vals/
// prepass_context/stage1_rows_of/op_run_rules/store_into/pop_rows/
// reduce_over/native_cook/fold_fire/rekey_transitions_native) is REUSED
// verbatim, never modified. Diagnostics (classified/unclassified/prose/
// missing/blocked/trace) are dropped -- only model_cells and folded_any are
// this boundary's contract.
fn compile_lines_native(
    j: &J,
    text: &str,
    seed_cells: &[(Leaf, V)],
    srv: &mut Srv,
    fuel: Option<i64>,
) -> Result<(Vec<(Leaf, V)>, bool), String> {
    use std::collections::{BTreeMap, HashMap, HashSet};
    let leaf = |s: &str| Leaf::S(s.to_string());

    // ---- grammar (mirrors op_compile_model's own load verbatim) ----
    let (mut dispatch, mut vocab) = grammar_tables(&srv.cells);
    let mut scratch: Option<GrammarScratch> = None;
    if vocab.is_empty() {
        match load_grammar_scratch(j) {
            Ok((g, _path)) => {
                let (d2, v2) = grammar_tables(&g.1);
                dispatch = d2;
                vocab = v2;
                scratch = Some(g);
            }
            Err(e) => return Err(format!("status_facts sub-compile: {}", e)),
        }
    }
    if dispatch.is_empty() {
        return Err(
            "status_facts sub-compile: no Classification_has_Translator rows".to_string(),
        );
    }

    let stmts = split_statements(text);
    let mut work: Vec<(String, &'static str, String, &'static str)> = Vec::new();
    for stmt in &stmts {
        let (m, sg, inner) = split_modality(stmt);
        if sg != "possibility" {
            work.push((stmt.clone(), m, inner, sg));
        }
    }

    // context_from = seed_cells (status_facts' own D, NOT the resident base)
    let (b_names, b_edges, b_fts, b_vals) = context_of(seed_cells);
    let mut names = known_names(&stmts);
    for n in b_names {
        names.insert(n);
    }
    let mut vals = known_vals(&stmts);
    for v in b_vals {
        vals.insert(v);
    }
    let b_fts_vec: Vec<String> = b_fts.into_iter().collect();
    let (subs, fts, plain) = prepass_context(&stmts, &names, &b_edges, &b_fts_vec);
    let mut nouns: Vec<String> = names.iter().cloned().collect();
    nouns.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));

    // ---- batch classification (the SAME save/swap/restore discipline
    // op_compile_model's own classify step uses) ----
    let mut by_cell: BTreeMap<String, Vec<V>> = BTreeMap::new();
    for (i, (_stmt, _m, inner, _sg)) in work.iter().enumerate() {
        let sid = format!("s{}", i + 1);
        for (ftb, s, v) in stage1_rows_of(inner, &vocab, &nouns, &sid) {
            by_cell
                .entry(ftb)
                .or_default()
                .push(seq(from_vec(vec![atom(Leaf::S(s)), atom(Leaf::S(v))])));
        }
    }
    let saved_d = srv.d.clone();
    let saved_cells = srv.cells.clone();
    let saved_nd = srv.nd.clone();
    let saved_ncells = srv.ncells.clone();
    let saved_nprocess = srv.nprocess.clone();
    let mut cls_by_sid: HashMap<String, HashSet<String>> = HashMap::new();
    if !by_cell.is_empty() {
        if let Some((gd, gcells, gnd, gncells, gproc)) = scratch {
            srv.d = gd;
            srv.cells = gcells;
            srv.nd = gnd;
            srv.ncells = gncells;
            srv.nprocess = gproc;
        }
        for (ftb, rows) in &by_cell {
            let name = leaf(ftb);
            let old = pop_rows(&srv.cells, &name);
            let mut merged: Vec<V> = Vec::new();
            let mut keys: HashSet<String> = HashSet::new();
            for r in old.iter().chain(rows.iter()) {
                if keys.insert(key_of(r)) {
                    merged.push(r.clone());
                }
            }
            sort_rows(&mut merged);
            store_into(
                &mut srv.d,
                &mut srv.cells,
                &mut srv.nd,
                &mut srv.ncells,
                &name,
                seq(from_vec(merged)),
            );
        }
        let frontier_req = J::O(vec![(
            "changed".to_string(),
            J::A(by_cell.keys().map(|k| J::S(k.clone())).collect()),
        )]);
        let derived = op_run_rules(&frontier_req, srv);
        if derived.is_ok() {
            for r in pop_rows(&srv.cells, &leaf("Statement_has_Classification")) {
                let it = items(&list_of(&r));
                if it.len() >= 2 {
                    if let (Some(s), Some(c)) = (
                        aval(&it[0]).and_then(|l| leaf_str(&l)),
                        aval(&it[1]).and_then(|l| leaf_str(&l)),
                    ) {
                        cls_by_sid.entry(s).or_default().insert(c);
                    }
                }
            }
        }
        srv.d = saved_d;
        srv.cells = saved_cells;
        srv.nd = saved_nd;
        srv.ncells = saved_ncells;
        srv.nprocess = saved_nprocess;
        derived?;
    }

    // ---- the dispatch loop (op_compile_model's own, diagnostics dropped) ----
    const GENERIC: [&str; 2] = ["Fact Type Reading", "Instance Fact"];
    let mut model_cells: Vec<(Leaf, V)> = seed_cells.to_vec();
    let atom_s = |s: &str| atom(Leaf::S(s.to_string()));
    let mut names_sorted: Vec<String> = names.iter().cloned().collect();
    names_sorted.sort();
    let subs_pairs: Vec<V> = subs
        .iter()
        .map(|(s, anc)| {
            seqc(vec![
                atom_s(s),
                seq(from_vec(anc.iter().map(|a| atom_s(a)).collect())),
            ])
        })
        .collect();
    let mut vals_sorted: Vec<String> = vals.iter().cloned().collect();
    vals_sorted.sort();
    let ctx = seqc(vec![
        seq(from_vec(names_sorted.iter().map(|n| atom_s(n)).collect())),
        seq(from_vec(subs_pairs)),
        seq(from_vec(fts.iter().map(|f| atom_s(f)).collect())),
        seq(from_vec(plain.iter().map(|f| atom_s(f)).collect())),
        seq(from_vec(vals_sorted.iter().map(|v| atom_s(v)).collect())),
    ]);
    let empty_cls: HashSet<String> = HashSet::new();
    let mut folded_any = false;
    let kn = cooks::Known::new(&names, &subs, &fts, &plain, &vals);

    for (i, (stmt, m, inner, sg)) in work.iter().enumerate() {
        let sid = format!("s{}", i + 1);
        let cls = cls_by_sid.get(&sid).unwrap_or(&empty_cls);
        let mut residual = cls.clone();
        residual.remove("Prose");
        residual.remove("Derivation Rule");
        for g in GENERIC {
            residual.remove(g);
        }
        if cls.contains("Prose") && residual.is_empty() {
            continue;
        }
        let specific: Vec<String> = cls
            .iter()
            .filter(|c| !GENERIC.contains(&c.as_str()))
            .cloned()
            .collect();
        if specific.is_empty() && *sg == "negative" && *m == "alethic" {
            continue;
        }
        let mut sorted_cls: Vec<String> = if specific.is_empty() {
            cls.iter().cloned().collect()
        } else {
            specific
        };
        sorted_cls.sort();
        let mut translators: Vec<String> = Vec::new();
        for c in &sorted_cls {
            if let Some(ts) = dispatch.get(c) {
                for t in ts {
                    if !translators.contains(t) {
                        translators.push(t.clone());
                    }
                }
            }
        }
        if translators.is_empty() {
            continue;
        }
        let mfield = if *m == "deontic" {
            format!("{}:{}", m, sg)
        } else {
            (*m).to_string()
        };
        for t in &translators {
            if translator_kinds(t).is_empty() {
                continue;
            }
            let operand = seqc(vec![
                atom(Leaf::S(inner.clone())),
                atom(Leaf::S(mfield.clone())),
                ctx.clone(),
                cells_to_d(&model_cells),
            ]);
            let res = reduce_over(srv, atom(Leaf::S(t.clone())), operand, fuel);
            if matches!(shape(&res), Shape::Seq(_)) && !isapp(&res) {
                model_cells = raw_cells_of(&res);
                folded_any = true;
                continue;
            }
            let cooked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                native_cook(t, inner, &mfield, &kn, srv)
            }))
            .unwrap_or_else(|p| {
                let msg = p
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| p.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "non-string panic payload".into());
                Err(format!("cook panicked: {}", msg))
            });
            if let Ok(Some(fire)) = cooked {
                fold_fire(&fire, &mut model_cells).map_err(|e| {
                    format!("status_facts fold: {} (statement: {})", e, stmt)
                })?;
                folded_any = true;
            }
        }
    }
    rekey_transitions_native(&mut model_cells);
    Ok((model_cells, folded_any))
}

// status_facts (engine.py:3516): each governed Object Type gets its "is
// currently in Status" fact type, generated through the ordinary reading
// path (compile_lines_native above) so the name is compiled, never
// hand-built. Runs BEFORE machine_fold so the status column is laid out
// (RMAP absorbs it, since "Each Noun is currently in at most one Status" is
// a role-1 UC) before the fold writes.
fn status_facts_native(
    j: &J,
    cells: &[(Leaf, V)],
    srv: &mut Srv,
) -> Result<Vec<(Leaf, V)>, String> {
    use std::collections::HashMap;
    let leaf = |s: &str| Leaf::S(s.to_string());

    // nouns: smDef rows' r[1] (governed noun), IN POP ORDER, duplicates
    // preserved (engine.py:3529's plain list comprehension -- no dedup)
    let nouns: Vec<String> = pop_rows(cells, &leaf("smDef"))
        .iter()
        .filter_map(|r| {
            let it = items(&list_of(r));
            if it.len() >= 2 {
                aval(&it[1]).map(|l| leaf_text(&l))
            } else {
                None
            }
        })
        .collect();
    if nouns.is_empty() {
        return Ok(cells.to_vec());
    }

    let has_status_value = pop_rows(cells, &leaf("instanceOf")).iter().any(|r| {
        let it = items(&list_of(r));
        it.len() >= 2
            && aval(&it[0]).map(|l| leaf_text(&l)).as_deref() == Some("Status")
            && aval(&it[1]).map(|l| leaf_text(&l)).as_deref() == Some("ValueType")
    });

    let mut lines: Vec<String> = Vec::new();
    if !has_status_value {
        lines.push("Status is a value type.".to_string());
    }
    for noun in &nouns {
        lines.push(format!("{} is currently in Status.", noun));
        lines.push(format!("Each {} is currently in at most one Status.", noun));
    }
    let text = format!("{}\n", lines.join("\n"));

    // engine.py's nested `forml.compile_model(text, D=D, context_from=D)`
    // never inherits the caller's own fuel budget (compile_model takes none
    // from status_facts) -- unbounded here regardless of the outer op's fuel
    let (cells2, _folded) = compile_lines_native(j, &text, cells, srv, None)?;

    let mut role1: HashMap<String, String> = HashMap::new();
    for r in pop_rows(&cells2, &leaf("role")) {
        let it = items(&list_of(&r));
        if it.len() >= 4 && matches!(aval(&it[2]).as_deref(), Some(Leaf::I(1))) {
            if let (Some(f), Some(pl)) = (aval(&it[1]), aval(&it[3])) {
                role1.insert(leaf_text(&f), leaf_text(&pl));
            }
        }
    }
    let mut templ: HashMap<String, String> = HashMap::new();
    for r in pop_rows(&cells2, &leaf("factType")) {
        let it = items(&list_of(&r));
        if it.len() >= 2 {
            if let (Some(f), Some(rd)) = (aval(&it[0]), aval(&it[1])) {
                templ.insert(leaf_text(&f), leaf_text(&rd));
            }
        }
    }
    let existing_smstatusft = pop_rows(&cells2, &leaf("smStatusFt"));
    let mut have: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    for r in &existing_smstatusft {
        let it = items(&list_of(r));
        if it.len() >= 2 {
            if let (Some(n), Some(f)) = (aval(&it[0]), aval(&it[1])) {
                have.insert((leaf_text(&n), leaf_text(&f)));
            }
        }
    }
    let mut templ_keys: Vec<String> = templ.keys().cloned().collect();
    templ_keys.sort();

    let mut rows: Vec<V> = existing_smstatusft;
    for noun in &nouns {
        for ft in &templ_keys {
            if role1.get(ft).map(|s| s.as_str()) == Some(noun.as_str())
                && templ.get(ft).map(|t| t.contains("is currently in")).unwrap_or(false)
                && !have.contains(&(noun.clone(), ft.clone()))
            {
                rows.push(seqc(vec![atom(Leaf::S(noun.clone())), atom(Leaf::S(ft.clone()))]));
            }
        }
    }
    let mut out = cells2;
    store_move(&mut out, "smStatusFt", seq(from_vec(rows)));
    Ok(out)
}

// ======================= the model-D seed (#20, native-compile mission) ======
// docs/2026-07-11-native-pipeline-tail-spec.md §3. Python's every app compile
// seeds from Registry._base_D() (protocol.py:1782), which is JUST
// persist.ingest_frozen(base_text, cache_dir=self.cache_dir) with no
// `compiler` override -- and ingest_frozen's cache-miss body (protocol.py:200)
// is `(compiler or forml.compile_model)(text)[0]`, i.e. bare
// `forml.compile_model(text)` with D=None, context_from=None. `forml` IS
// `compiler` (engine/python/__init__.py's alias table), and compile_model
// (compiler.py:2478) is `compile_model_selfhost(...) + system.rekey_
// transitions(D2)` -- nothing else. So _base_D() is fold+rekey ONLY: no
// run_rules, no status_facts, no machine_fold, no layout_cells -- those are
// the PIPELINE's own later calls, made in Registry.compile() on the
// base-seeded APP store, never on the base alone. (This narrower boundary
// supersedes the task brief's literal "classify -> cooks -> fold -> rekey ->
// rules -> status_facts -> machine_fold -> rules-iff-changed -> layout_cells"
// list, which was that brief's hypothesis pending exactly this check.)
//
// That boundary already has a native twin: compile_lines_native (above,
// #20 machine_fold slice) IS compile_model_selfhost + rekey_transitions_
// native, byte-parity-certified as status_facts_native's own nested compile.
// Calling it with seed_cells = initial_d_cells() reproduces D=None/context_
// from=None exactly: context_of(&initial_d_cells()) reads instanceOf/
// factType/subtype off a store holding only the FILE:phi cell, finding none
// of those three names and returning the same empty (HashSet::new(),
// Vec::new(), HashSet::new(), HashSet::new()) python's own `context_from is
// None` shortcut returns. So base_seed below is a THIN wrapper: assemble the
// base text, then hand it to the already-certified compile_lines_native --
// zero new fold/rekey logic, per the constraint against touching those
// functions.

// read_base_text mirrors Registry._base_D's own text assembly EXACTLY
// (protocol.py:1787-1789): every *.md file directly under base_dir,
// filename-sorted (plain string sort; all filenames here are lowercase
// ASCII, so byte order and codepoint order agree), joined by a blank line.
//
// The one native ADDITION, load-bearing: Python's open(path,
// encoding="utf-8").read() is TEXT MODE, which performs universal-newline
// translation on every read ("\r\n" and a lone "\r" both fold to "\n").
// Rust's std::fs::read_to_string does not. This repo checks out with
// core.autocrlf=true and engine/shared/base/*.md carry real CRLF on disk
// (verified empirically: core.md alone holds 1,053 "\r\n" pairs, zero lone
// "\r"). Every EXISTING native pipeline entry point (op_compile_model,
// compile_lines_native) takes "text" as a pre-supplied JSON string -- every
// caller reads the file in Python first, so this normalization has always
// happened upstream, invisibly. base_seed is the FIRST native code path that
// reads readings files itself, so it is the first that must do this
// normalization on purpose; skipped, every base statement would carry a
// trailing '\r' baked into its stored text -- a silent divergence from
// python's D, not a crash.
fn normalize_newlines(s: &str) -> String {
    if !s.contains('\r') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    out
}

fn read_base_text(base_dir: &std::path::Path) -> Result<String, String> {
    let rd = std::fs::read_dir(base_dir)
        .map_err(|e| format!("unreadable base dir {}: {}", base_dir.display(), e))?;
    let mut names: Vec<String> = Vec::new();
    for entry in rd {
        let entry = entry.map_err(|e| format!("base dir walk {}: {}", base_dir.display(), e))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".md") && entry.path().is_file() {
            names.push(name);
        }
    }
    names.sort();
    let mut parts: Vec<String> = Vec::with_capacity(names.len());
    for name in &names {
        let raw = std::fs::read_to_string(base_dir.join(name))
            .map_err(|e| format!("unreadable base reading {}: {}", name, e))?;
        parts.push(normalize_newlines(&raw));
    }
    Ok(parts.join("\n\n"))
}

// ============================ sha256 (zero-dep base-seed fingerprint) ========
// A minimal from-scratch SHA-256 (FIPS 180-4), standard constants and
// algorithm -- Cargo.toml is explicit that the HOST build stays zero-dep
// ("the crate's first dependency, deliberate and OPTIONAL" is wasm-bindgen,
// worker-only), so the base-seed key hashes with this instead of pulling a
// sha2 crate. Verified against the two standard test vectors (sha256("") and
// sha256("abc")) in a standalone scratch build before landing here
// (seed-sha256-check.rs, both matched byte for byte); not re-checked by a
// #[test] because this crate carries none (differential scripts against the
// python engine are its existing acceptance discipline, kept consistent
// rather than introducing a new one for one helper).
const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

fn sha256_hex(data: &[u8]) -> String {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let bitlen: u64 = (data.len() as u64) * 8;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bitlen.to_be_bytes());

    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(SHA256_K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut out = String::with_capacity(64);
    for x in h {
        out.push_str(&format!("{:08x}", x));
    }
    out
}

// exe_fingerprint_hex is the base-seed key's "did the engine change" half:
// the running executable's own bytes, sha256'd -- "the exe's own hash", the
// cheaper of the spec's two named options (the other, a compile-time env,
// needs a build.rs: a build-system change this crate's dependency policy
// steers away from, ditto Cargo.toml's zero-dep comment). It is, if
// anything, MORE honest than python's _engine_fingerprint (hashes only
// engine/python/*.py + engine/shared/*.py): a binary hash also invalidates
// on a toolchain bump or a Cargo.lock change, any input that changes the
// compiled output, not just main.rs's own text. Memoized per process
// (OnceLock, stable std, no new dependency) -- the exe cannot change under a
// running process, and base_seed may be called more than once per --serve
// session.
fn exe_fingerprint_hex() -> Result<String, String> {
    static FP: std::sync::OnceLock<Result<String, String>> = std::sync::OnceLock::new();
    FP.get_or_init(|| {
        let exe = std::env::current_exe()
            .map_err(|e| format!("no executable path to fingerprint: {}", e))?;
        let bytes = std::fs::read(&exe)
            .map_err(|e| format!("unreadable executable {}: {}", exe.display(), e))?;
        Ok(sha256_hex(&bytes))
    })
    .clone()
}

// base_seed_key: sha256(exe_fingerprint || 0x00 || base_text) -- the same
// "fingerprint + NUL + text" shape ingest_frozen hashes on the python side
// (_engine_fingerprint() + "\x00" + text), unrelated in VALUE (this key
// never needs to equal python's own; each host invalidates only its own
// cache) but matched in FORM since it is the same idea for the same reason.
fn base_seed_key(base_text: &str) -> Result<String, String> {
    let fp = exe_fingerprint_hex()?;
    let mut buf = fp.into_bytes();
    buf.push(0);
    buf.extend_from_slice(base_text.as_bytes());
    Ok(sha256_hex(&buf))
}

// base_seed_paths resolves the base readings directory and the base store
// sidecar path. CONVENTION, matching load_grammar_scratch's own exactly:
// walk up from the executable for a "shared" directory carrying
// forml2-grammar.store.json (the grammar sidecar's own landmark file), then
// base_dir = <that>/base and store_path = <that>/base.store.json -- "beside
// the grammar sidecar" per the spec, literally the same directory. Explicit
// "base_dir"/"base_store" op args override either half independently (the
// differential scripts use this to point at a worktree's own engine/shared
// without relying on exe ancestry, exactly as grammar_sidecar does).
fn base_seed_paths(j: &J) -> Result<(std::path::PathBuf, std::path::PathBuf), String> {
    let explicit_dir = match jget(j, "base_dir") {
        Some(J::S(p)) => Some(std::path::PathBuf::from(p)),
        Some(_) => return Err("base_dir must be a string path".to_string()),
        None => None,
    };
    let explicit_store = match jget(j, "base_store") {
        Some(J::S(p)) => Some(std::path::PathBuf::from(p)),
        Some(_) => return Err("base_store must be a string path".to_string()),
        None => None,
    };
    let shared: Option<std::path::PathBuf> = if explicit_dir.is_none() || explicit_store.is_none() {
        let exe = std::env::current_exe()
            .map_err(|e| format!("no executable path to walk for the base seed: {}", e))?;
        exe.ancestors()
            .skip(1)
            .map(|dir| dir.join("shared"))
            .find(|cand| cand.join("forml2-grammar.store.json").is_file())
    } else {
        None
    };
    let base_dir = match explicit_dir {
        Some(d) => d,
        None => shared
            .clone()
            .ok_or_else(|| {
                "base readings dir not found by walking the executable's ancestors for \
                 shared/forml2-grammar.store.json; pass base_dir"
                    .to_string()
            })?
            .join("base"),
    };
    let store_path = match explicit_store {
        Some(s) => s,
        None => shared
            .ok_or_else(|| {
                "base store path not found by walking the executable's ancestors for \
                 shared/forml2-grammar.store.json; pass base_store"
                    .to_string()
            })?
            .join("base.store.json"),
    };
    Ok((base_dir, store_path))
}

// op_base_seed: the native model-D seed (task #20). Ensures the resident
// store holds the CURRENT base (thawing base.store.json when its embedded
// key matches a fresh recompute-would-produce-this key, else recomputing
// through compile_lines_native and persisting tmp-then-rename) and answers
// which path was taken. "Current" is exactly the regen rule the spec names
// (the sidecar lesson, 310404b4): the key is sha256(exe bytes, NUL, base
// text), so a changed reading OR a rebuilt engine changes the key and a
// stale thaw is impossible by construction, never a silent hit.
//
// The persisted shape is the SAME {"d": ...} the serve loop's generic
// preamble already accepts (handle()'s `if let Some(dj) = jget(j, "d")`
// arm) -- base.store.json can be fed as an ordinary preamble line by any
// caller that does not want the key-check convenience this op adds; this op
// is that convenience plus the write path. A "key" field rides alongside
// "d" (the spec's "sibling key ... or embedded field" alternative); generic
// preamble consumers ignore the extra field, so the two consumption paths
// coexist over one file.
fn op_base_seed(j: &J, srv: &mut Srv) -> Result<String, String> {
    let fuel = match jget(j, "fuel") {
        Some(J::I(n)) if *n > 0 => Some(*n),
        _ => None,
    };
    let dump_store_on = matches!(jget(j, "dump_store"), Some(J::I(1)));

    let (base_dir, store_path) = base_seed_paths(j)?;
    let base_text = read_base_text(&base_dir)?;
    let key = base_seed_key(&base_text)?;

    // ---- the thaw attempt: a key match proves THIS binary already wrote
    // this exact base text's store, so trusting the file is exactly as
    // correct as recomputing (never a stale shortcut). Any failure along
    // this path (missing file, bad JSON, no "d", mismatched key) falls
    // through to the recompute below instead of erroring.
    if let Ok(text) = std::fs::read_to_string(&store_path) {
        if let Some(payload) = parse_json(&text) {
            let existing_key = match jget(&payload, "key") {
                Some(J::S(k)) => Some(k.as_str()),
                _ => None,
            };
            if existing_key == Some(key.as_str()) {
                if let Some(dj) = jget(&payload, "d") {
                    srv.d = to_v(dj);
                    srv.cells = cells_of(&srv.d);
                    srv.nd = j_to_n(dj);
                    srv.ncells = n_cells_of(&srv.nd);
                    let mut r = String::from("{\"source\":\"thawed\",\"cells\":");
                    r.push_str(&srv.cells.len().to_string());
                    r.push_str(",\"key\":");
                    esc(&key, &mut r);
                    r.push_str(",\"path\":");
                    esc(&store_path.display().to_string(), &mut r);
                    if dump_store_on {
                        r.push_str(",\"store\":");
                        write_v(&srv.d, &mut r);
                    }
                    r.push('}');
                    return Ok(r);
                }
            }
        }
    }

    // ---- recompute: ingest_frozen's exact boundary (compile_model_selfhost
    // + rekey_transitions, D=None/context_from=None), via the ALREADY
    // byte-parity-certified compile_lines_native -- no new fold/rekey logic
    // (the constraint against touching those functions is honored by
    // reusing this one wholesale, not by re-deriving it).
    let (model_cells, folded_any) =
        compile_lines_native(j, &base_text, &initial_d_cells(), srv, fuel)
            .map_err(|e| format!("base_seed compile: {}", e))?;
    srv.d = cells_to_d(&model_cells);
    srv.cells = model_cells;
    srv.nd = v_to_n(&srv.d);
    srv.ncells = n_cells_of(&srv.nd);

    let mut payload = String::from("{\"d\":");
    write_v(&srv.d, &mut payload);
    payload.push_str(",\"key\":");
    esc(&key, &mut payload);
    payload.push('}');
    let mut tmp = store_path.clone().into_os_string();
    // the tmp name carries the pid, matching write_sidecar/_sidecar: two
    // writers racing to reseed the SAME base.store.json must never share one
    // tmp path and tear the file
    tmp.push(format!(".{}.tmp", std::process::id()));
    let tmp = std::path::PathBuf::from(tmp);
    std::fs::write(&tmp, payload.as_bytes())
        .map_err(|e| format!("base_seed write {}: {}", tmp.display(), e))?;
    std::fs::rename(&tmp, &store_path)
        .map_err(|e| format!("base_seed rename to {}: {}", store_path.display(), e))?;

    let mut r = String::from("{\"source\":\"recomputed\",\"cells\":");
    r.push_str(&srv.cells.len().to_string());
    r.push_str(",\"key\":");
    esc(&key, &mut r);
    r.push_str(",\"path\":");
    esc(&store_path.display().to_string(), &mut r);
    r.push_str(",\"folded\":");
    r.push_str(if folded_any { "true" } else { "false" });
    if dump_store_on {
        r.push_str(",\"store\":");
        write_v(&srv.d, &mut r);
    }
    r.push('}');
    Ok(r)
}

// ============================ replay (#20, the replay port slice) ============
// persist.replay_entries (protocol.py:251), native: rebuild state by
// re-ingesting the app's event log through the SAME create the live apply
// path (apply_core, below) uses. Facts are the source of truth; set
// semantics make replay idempotent. Sink-agnostic (protocol.py's own
// promise): replay_entries_native takes the ENTRY LIST already resolved by
// the caller (op_compile_model's own "replay_entries"/"replay_path"
// handling), never a path itself.
//
// Four arms, exactly python's:
//   1. PLAIN entries (no "op") BATCH: buffered by fact type, flushed
//      through the bulk paths a migrate op also rides -- absorbed via
//      mf_bulk_absorbed_install(replace_keys=false, python's own default:
//      UNION, never overwrite), own-table via a rowsorted Store union.
//   2. "retract": flush first, then Store(ft) of the pop minus the exact
//      row (eqobj: type-strict equality, NO _rowsort -- protocol.py:318
//      skips it, and this port mirrors that omission deliberately).
//   3. "migrate": flush first, then one bulk install (absorbed) or one
//      rowsorted-union Store (own-table) of entry["facts"] -- UNCONDITIONAL,
//      never checking smTrigger (confirmed by reading protocol.py:320-341
//      line by line: no `_triggers` call anywhere in this arm), so a
//      migrate entry never fires a machine even when its ft happens to be
//      one -- the REAL tasks.events.jsonl exercises exactly this (three of
//      Task SM's own trigger fact types arrive ONLY as migrate entries in
//      that log).
//   4. TRIGGER entries (a plain entry whose ft is in the frozen smTrigger
//      set): flush first (log order: an entity's initial status lands
//      before its events), then the GATED CREATE through rp_create_spec +
//      rp_create_from_spec -- the schema recipe (memoized per ft, python's
//      spec_box) and the ast:build_system handler assembly, reduced through
//      rp_reduce_apply, THE SAME create internals apply_core's live-write
//      path (below) rides. apply_core itself cannot be called unmodified
//      here: it looks up an ALREADY-BUILT create:<ft> cell (populated by
//      system.create_handlers, engine.py:3167), and create_handlers runs
//      LATE in the real pipeline (protocol.py:1852, well after replay and
//      even after layout_cells/scheduler_cells/generator_cells) -- no such
//      cell exists yet at this point in a compile. So the handler is built
//      FRESH here, the same way python's own create()/_create_from_spec
//      does on ITS OWN live-apply path (system.create never reads a
//      create:<ft> cell either -- create_spec recomputes the recipe every
//      call; create_handlers is purely the RESIDENT's later optimization,
//      not part of the create semantics itself). Only the REDUCTION step
//      (apply(handler, <fact,D>)) is shared with apply_core, factored out
//      as rp_reduce_apply -- "the same create internals", not a second
//      create.
//
// ONE partition + ONE trigger-ft-set for the whole replay (python's
// part_box/trig_box, protocol.py:276-293): the schema never changes here
// (replay only ever writes ORDINARY fact-type populations -- never
// smTrigger/role/governedBy/smStatusFt/factType/valueConstraint, all of
// which are established once at compile time, before replay ever runs), so
// computing the partition and the trigger set once off the STARTING cells
// and reusing them for every flush/spec/fire is not an approximation, it is
// exact -- and it is the fix for the traced 2026-07-09 cost (~12s per
// migrate entry going to partition recomputation; the 71-83s replay phase
// on a support-scale compile).

// rp_machine_step / rp_mealy_step (engine.py:2996/3012): thin canon
// wrappers -- napp builds the UNREDUCED application node, embedded as DATA
// into the create record exactly as python's own `_apply(...)` return value
// is (build_system's record slots carry these unreduced; ast:build_system's
// own canon body invokes them in-step, against the addressed entity's row,
// when the ASSEMBLED handler is later applied to <fact, D> -- never here).
fn rp_machine_step(ft: &str, row_col: Option<i64>) -> N {
    let rc = match row_col {
        None => nseq(vec![]),
        Some(c) => nseq(vec![N::A(Rc::new(Leaf::I(c)))]),
    };
    napp(mf_na("system:machine_step"), nseq(vec![mf_na(ft), rc]))
}

fn rp_mealy_step(ft: &str, row_col: Option<i64>) -> N {
    let rc = match row_col {
        None => nseq(vec![]),
        Some(c) => nseq(vec![N::A(Rc::new(Leaf::I(c)))]),
    };
    napp(mf_na("system:mealy_step"), nseq(vec![mf_na(ft), rc]))
}

// rp_transitions_of (engine.py:929): `sm` rides RAW (already a native
// value -- to_lam'd triples -- never re-atomized), matching links_of's own
// documented warning about this exact shape.
fn rp_transitions_of(sm: N, status_pos: i64) -> N {
    napp(
        mf_na("system:transitions_of"),
        nseq(vec![sm, N::A(Rc::new(Leaf::I(status_pos)))]),
    )
}

// rp_row_resolve (engine.py:2634): thin canon wrapper.
fn rp_row_resolve(col: i64, width: i64, unary: bool) -> N {
    napp(
        mf_na("system:row_resolve"),
        nseq(vec![
            N::A(Rc::new(Leaf::I(col))),
            N::A(Rc::new(Leaf::I(width))),
            mf_na(if unary { "T" } else { "F" }),
        ]),
    )
}

// rp_row_validate (engine.py:3321): the M-fact lookups (role/valueConstraint)
// stay host, schema-only pop scans over the FROZEN initial cells (safe: both
// fact types are compile-time-only, replay never writes them); `col` is
// passed in already computed by the caller (create_spec's own col, reused
// rather than re-deriving table_columns a second time -- same numeric
// result, cheaper). python's `if table == ft: return None` guard is
// unreachable from create_spec's own call site (only called when absorbed
// is already established) and is not reproduced.
fn rp_row_validate(cells0: &[(Leaf, V)], ft: &str, col: i64) -> Option<N> {
    use std::collections::HashMap;
    let leaf = |s: &str| Leaf::S(s.to_string());
    let players: Vec<String> = pop_rows(cells0, &leaf("role"))
        .iter()
        .filter_map(|r| {
            let it = items(&list_of(r));
            if it.len() >= 4 && aval(&it[1]).map(|l| leaf_text(&l)).as_deref() == Some(ft) {
                aval(&it[3]).map(|l| leaf_text(&l))
            } else {
                None
            }
        })
        .collect();
    if players.is_empty() {
        return None;
    }
    // vcs: player -> modality; LAST occurrence per player wins, mirroring
    // python's `{r[0]: r for r in _pop_rows(D,"valueConstraint")}` dict
    // comprehension over pop order (vt == the player name itself, since the
    // dict is keyed by r[0] and hits[0][0] reads that same r[0] back)
    let mut vcs: HashMap<String, String> = HashMap::new();
    for r in pop_rows(cells0, &leaf("valueConstraint")) {
        let it = items(&list_of(&r));
        if it.len() >= 3 {
            if let (Some(k), Some(m)) = (aval(&it[0]), aval(&it[2])) {
                vcs.insert(leaf_text(&k), leaf_text(&m));
            }
        }
    }
    let (vt, modality) = players.iter().find_map(|p| vcs.get(p).map(|m| (p.clone(), m.clone())))?;
    Some(napp(
        mf_na("system:row_validate"),
        nseq(vec![
            N::A(Rc::new(Leaf::I(col))),
            mf_na(&format!("{}_vc", vt)),
            mf_na(if modality == "alethic" { "T" } else { "F" }),
        ]),
    ))
}

#[derive(Clone)]
struct RpMachine {
    status_table: String,
    status_col: i64,
    status_width: i64,
    sm_obj: N,
    role_pos: Option<i64>,
}

#[derive(Clone)]
struct RpSpec {
    table: String,
    absorbed: bool,
    machine: Option<RpMachine>,
    mealy: Option<N>,
    links: Option<N>,
    col: Option<i64>,
    width: Option<i64>,
    unary: Option<bool>,
    validate: Option<N>,
}

// rp_create_spec (engine.py:3063 create_spec): the schema-determined create
// recipe for a fact type. `is_trigger` gates the noun/machine/mealy/links
// computation exactly as python's own unconditional
// `any(r[1]==fact_type for r in _pop_rows(D,"smTrigger"))` guard does --
// added for create_handlers_native (#20, the final pipeline slice), which
// must call this for EVERY fact type in the schema, not just known triggers
// (replay's own call site, below, only ever reaches a ft ALREADY known to be
// in the frozen trigger set -- see `trig_fts` in replay_entries_native --
// and passes `is_trigger: true` unconditionally, so this parameter is a
// pure widening: replay's byte-for-byte behavior is unchanged, since
// `if true {...}` is exactly the unconditional call the prior slice shipped).
// The gate is LOAD-BEARING for create_handlers: system:governed_player finds
// a governed noun for ANY fact type with a role pointing at one (a plain
// data attribute of a governed entity qualifies), NOT only its smTrigger
// fact types -- without this gate every such attribute would wrongly grow
// machine/mealy/links wiring in its create:<ft> handler. Every read below is
// SCHEMA-only (smTrigger/role/governedBy/smStatusFt/valueConstraint are
// compile-time populations replay never writes), so the FROZEN initial
// ev0/nd0/cells0 -- computed once by the caller, the same part_box
// precedent extended to the whole spec computation -- are exact, not an
// approximation, for every ft this is called with, not just the partition.
fn rp_create_spec(
    cells0: &[(Leaf, V)],
    ev0: &NEval,
    nd0: &N,
    part: &HashMap<String, String>,
    pairs_v: &V,
    ft: &str,
    is_trigger: bool,
) -> Result<RpSpec, String> {
    let leaf = |s: &str| Leaf::S(s.to_string());
    let table = part.get(ft).cloned().unwrap_or_else(|| ft.to_string());
    let absorbed = table != ft;
    let row_col: Option<i64> = if absorbed {
        let cols = mf_table_columns(ev0, &table, pairs_v);
        let pos = cols
            .iter()
            .position(|c| c == ft)
            .ok_or_else(|| format!("create_spec: {} not a column of {}", ft, table))?;
        Some((2 + pos) as i64)
    } else {
        None
    };

    let noun = if is_trigger { mf_governed_player(ev0, nd0, ft) } else { None };
    let mut machine: Option<RpMachine> = None;
    let mut mealy: Option<N> = None;
    let mut links: Option<N> = None;
    if let Some(noun) = &noun {
        let role_pos: Option<i64> = pop_rows(cells0, &leaf("role")).iter().find_map(|r| {
            let it = items(&list_of(r));
            if it.len() >= 4
                && aval(&it[1]).map(|l| leaf_text(&l)).as_deref() == Some(ft)
                && aval(&it[3]).map(|l| leaf_text(&l)).as_deref() == Some(noun.as_str())
            {
                if let Some(Leaf::I(p)) = aval(&it[2]).as_deref() {
                    return Some(*p);
                }
            }
            None
        });
        let mut gov: HashMap<String, String> = HashMap::new();
        for r in pop_rows(cells0, &leaf("governedBy")) {
            let it = items(&list_of(&r));
            if it.len() >= 2 {
                if let (Some(a), Some(b)) = (aval(&it[0]), aval(&it[1])) {
                    gov.insert(leaf_text(&a), leaf_text(&b));
                }
            }
        }
        let gov_target = gov.get(noun).cloned().unwrap_or_else(|| noun.clone());
        let status_ft_name: Option<String> =
            pop_rows(cells0, &leaf("smStatusFt")).iter().find_map(|r| {
                let it = items(&list_of(r));
                if it.len() >= 2
                    && aval(&it[0]).map(|l| leaf_text(&l)).as_deref() == Some(gov_target.as_str())
                {
                    aval(&it[1]).map(|l| leaf_text(&l))
                } else {
                    None
                }
            });
        let (status_ft, status_table) = match status_ft_name
            .as_ref()
            .and_then(|sft| part.get(sft).map(|t| (sft.clone(), t.clone())))
        {
            Some(pair) => pair,
            None => {
                return Err(format!(
                    "machine on {:?} without its status column: run system.status_facts \
                     (then layout_cells) before create -- status(e) IS the \
                     '<Noun> is currently in Status' fact type",
                    noun
                ))
            }
        };
        let scols = mf_table_columns(ev0, &status_table, pairs_v);
        let spos = scols
            .iter()
            .position(|c| c == &status_ft)
            .ok_or_else(|| format!("create_spec: status ft {} missing from its own columns", status_ft))?;
        let status_col = (2 + spos) as i64;
        let status_width = (1 + scols.len()) as i64;
        machine = Some(RpMachine {
            status_table,
            status_col,
            status_width,
            sm_obj: rp_machine_step(ft, row_col),
            role_pos,
        });
        mealy = Some(rp_mealy_step(ft, row_col));
        if !absorbed && role_pos.is_some() {
            let triples = mf_sm_triples(cells0, ev0);
            let sm_n = nseq(
                triples
                    .iter()
                    .map(|(f, t, to)| nseq(vec![mf_na(f), mf_na(t), mf_na(to)]))
                    .collect(),
            );
            links = Some(rp_transitions_of(sm_n, 2));
        }
    }

    let mut col = None;
    let mut width = None;
    let mut unary = None;
    let mut validate = None;
    if absorbed {
        let cols = mf_table_columns(ev0, &table, pairs_v);
        let pos = cols
            .iter()
            .position(|x| x == ft)
            .ok_or_else(|| format!("create_spec: {} missing from {} columns", ft, table))?;
        col = Some((2 + pos) as i64);
        width = Some((1 + cols.len()) as i64);
        unary = Some(mf_role_max_pos(cells0, ft) == 1);
        validate = rp_row_validate(cells0, ft, col.unwrap());
    }

    Ok(RpSpec { table, absorbed, machine, mealy, links, col, width, unary, validate })
}

// rp_reduce_apply is the create-internals' raw reduction step apply_core (a
// handler looked up from a stored create:<ft> cell) and replay's trigger arm
// (a handler freshly reduced from ast:build_system -- create:<ft> cells
// don't exist yet at replay time, create_handlers runs LATE, protocol.py
// :1852) both ride: apply(handler, <fact, D>) on the native carrier. Each
// caller interprets the raw N result its own way (apply_core delegates to
// the CLI on a malformed pair; replay falls back to <ERROR, D> unchanged,
// engine.py's own _transition contract, since replay has nowhere to
// delegate to) -- this fn performs only the ONE reduction step both share,
// "the same create internals", per the task's own instruction not to write
// a second create.
fn rp_reduce_apply(ev: &NEval, handler: &N, fact_n: &N, d_n: &N) -> N {
    ev.mu(napp(handler.clone(), nseq(vec![fact_n.clone(), d_n.clone()])))
}

// rp_create_from_spec (engine.py:3115 _create_from_spec + :208 run + :79
// build_system): assembles the create-cell record (build_system's exact
// 9-slot shape: cell_name, validate, resolve, derive(always empty -- create
// never sets it), links, machine, mealy, index_cell, append_cell), reduces
// ast:build_system over it (the ONE handler-materialization python's own
// build_system call performs -- ast:build_system IS a loaded canon def,
// shared/ast.canon, included verbatim at main.rs's canon_defs()), then
// reduces THE SAME create internals apply_core rides (rp_reduce_apply) over
// <fact, D>. Returns D' ONLY (python's `_ap(_A(2), ...)` -- the trigger arm
// discards the representation o and any violations outright; these facts
// already committed once, live, when the log was written, and replay's own
// docstring calls the log "history, already validated at commit time").
fn rp_create_from_spec(ev: &NEval, nd: &N, ft: &str, fact_n: N, spec: &RpSpec) -> N {
    let slot = |v: Option<N>| match v {
        None => nseq(vec![]),
        Some(x) => nseq(vec![x]),
    };
    let machine_slot = match &spec.machine {
        None => nseq(vec![]),
        Some(m) => {
            let target = nseq(vec![
                mf_na(&m.status_table),
                N::A(Rc::new(Leaf::I(m.status_col))),
                N::A(Rc::new(Leaf::I(m.status_width))),
            ]);
            // role_pos is None only if a governed trigger fact type carries
            // no role row for its own governed noun -- unreachable given
            // governed_player's own definition already found such a role to
            // establish `noun` in the first place (rp_create_spec above);
            // N::Bot is the documented, deliberately-inert fallback if it
            // ever is
            let role_atom = match m.role_pos {
                Some(p) => N::A(Rc::new(Leaf::I(p))),
                None => N::Bot,
            };
            nseq(vec![target, m.sm_obj.clone(), role_atom])
        }
    };

    let (cell_name, resolve_slot, validate_slot, index_slot, append_slot) = if !spec.absorbed {
        (ft.to_string(), nseq(vec![]), nseq(vec![]), nseq(vec![]), nseq(vec![]))
    } else {
        let key_leaf: Leaf = match &fact_n {
            N::S(items) => items.first().and_then(|x| match x {
                N::A(l) => Some((**l).clone()),
                _ => None,
            }),
            _ => None,
        }
        .unwrap_or_else(|| Leaf::S(String::new()));
        let col = spec.col.expect("rp_create_spec always sets col when absorbed");
        let width = spec.width.expect("rp_create_spec always sets width when absorbed");
        let unary = spec.unary.unwrap_or(false);
        let resolve = rp_row_resolve(col, width, unary);
        let cname = format!("{}:{}", spec.table, leaf_text(&key_leaf));
        (
            cname,
            slot(Some(resolve)),
            slot(spec.validate.clone()),
            slot(Some(mf_na(&spec.table))),
            slot(Some(mf_na(ft))),
        )
    };

    let record = nseq(vec![
        mf_na(&cell_name),
        validate_slot,
        resolve_slot,
        nseq(vec![]), // derive_obj: create never sets it
        slot(spec.links.clone()),
        machine_slot,
        slot(spec.mealy.clone()),
        index_slot,
        append_slot,
    ]);

    let handler = ev.mu(napp(mf_na("ast:build_system"), record));
    let rn = rp_reduce_apply(ev, &handler, &fact_n, nd);
    match &rn {
        N::S(v) if v.len() == 2 => v[1].clone(),
        _ => nd.clone(), // _transition's fallback: a non-pair answer is <ERROR, D unchanged>
    }
}

// rp_absorbed_handler_tree (engine.py:3134 _absorbed_handler): the
// fact-DEPENDENT create handler stored WHOLE, UNREDUCED -- the nine-slot
// build_system record computes its cell name from the fact at REDUCE time
// (apply(cellkey, <table, key>), key = N1(N1(fact)) since the row's first
// element is the entity's key), so ONE stored cell serves every future fact
// of this absorbed type. Every OTHER slot is spec's own constant, CONST-
// wrapped (K_) so the record ignores the fact for them; only the cell_name
// slot (cellfn) is a real function of the input. Built at the N level
// (napp/nseq/mf_na, the same vocabulary rp_create_spec's siblings use) and
// converted to V only at the storage site (create_handlers_native), never
// reduced here -- apply_core/rp_reduce_apply reduce it LATER, once per
// actual write, computing ast:build_system fresh with THAT write's cell
// name (create_spec's own docstring: "any host builds the handler and
// reduces it natively on apply").
fn rp_absorbed_handler_tree(ev: &NEval, spec: &RpSpec, ft: &str) -> N {
    let k_ = |x: N| nseq(vec![mf_na("CONST"), x]);
    let slot = |v: Option<N>| match v {
        None => nseq(vec![]),
        Some(x) => nseq(vec![x]),
    };
    // EAGERLY reduced (ev.mu), matching python's own eager `apply()` calls
    // inside row_resolve/machine_step/mealy_step/row_validate (engine.py's
    // "thin canon wrapper" functions all call the REAL reducer, not a lazy
    // AST constructor -- confirmed empirically: create_handlers' python
    // output embeds the FULLY EXPANDED combinator tree at these slots, never
    // an unreduced "apply(name, operand)" reference). rp_create_spec's own
    // siblings (rp_machine_step/rp_mealy_step/rp_transitions_of/
    // rp_row_resolve/rp_row_validate) build UNREDUCED nodes because
    // replay's call sites (rp_create_from_spec) immediately fold them into
    // ONE outer apply-to-a-fact reduction that forces everything needed
    // regardless (Church-Rosser: same normal form either way) -- but HERE
    // the handler tree is PERSISTED AS DATA, never further reduced as a
    // whole, so any unreduced sub-node would show up verbatim in the
    // dumped store. Forcing here, once, at the point of embedding, is
    // provably behavior-preserving for every OTHER consumer of these
    // helpers (same Leaf/N result either way) and is what fixed the first
    // real byte divergence this slice's differential caught.
    let resolve = ev.mu(rp_row_resolve(
        spec.col.expect("absorbed spec always sets col"),
        spec.width.expect("absorbed spec always sets width"),
        spec.unary.unwrap_or(false),
    ));
    let m = match &spec.machine {
        None => nseq(vec![]),
        Some(mach) => {
            let target = nseq(vec![
                mf_na(&mach.status_table),
                N::A(Rc::new(Leaf::I(mach.status_col))),
                N::A(Rc::new(Leaf::I(mach.status_width))),
            ]);
            let role_atom = match mach.role_pos {
                Some(p) => N::A(Rc::new(Leaf::I(p))),
                None => N::Bot,
            };
            nseq(vec![target, ev.mu(mach.sm_obj.clone()), role_atom])
        }
    };
    let mealy_reduced = spec.mealy.clone().map(|n| ev.mu(n));
    let validate_reduced = spec.validate.clone().map(|n| ev.mu(n));
    let one = N::A(Rc::new(Leaf::I(1)));
    // key = COMP(1,1): first-of-first over the reduce-time operand <fact,D>
    // -- N1(N1(P)) reads the fact's own first element, its key
    let key = nseq(vec![mf_na("COMP"), one.clone(), one]);
    // cellfn = COMP(apply, CONS(K(cellkey), CONS(K(table), key))): at reduce
    // time, apply(cellkey, <table, key(P)>) -- the entity's routed cell name
    let cellfn = nseq(vec![
        mf_na("COMP"),
        mf_na("apply"),
        nseq(vec![
            mf_na("CONS"),
            k_(mf_na("cellkey")),
            nseq(vec![mf_na("CONS"), k_(mf_na(&spec.table)), key]),
        ]),
    ]);
    // rec: the 9-slot build_system record, cellfn RAW (a function of P),
    // every other slot CONST-wrapped -- build_system's own canonical order:
    // cell_name, validate, resolve, derive(always empty), links(always
    // empty for an absorbed handler -- create_spec never sets it when
    // absorbed), machine, mealy, index_cell=table, append_cell=ft
    let rec = nseq(vec![
        mf_na("CONS"),
        cellfn,
        k_(slot(validate_reduced)),
        k_(slot(Some(resolve))),
        k_(nseq(vec![])),
        k_(nseq(vec![])),
        k_(m),
        k_(slot(mealy_reduced)),
        k_(slot(Some(mf_na(&spec.table)))),
        k_(slot(Some(mf_na(ft)))),
    ]);
    // build = COMP(apply, CONS(K(ast:build_system), rec)): at reduce time,
    // apply(ast:build_system, rec(P)) -- builds the CONCRETE handler for
    // THIS P's computed cell name
    let build = nseq(vec![
        mf_na("COMP"),
        mf_na("apply"),
        nseq(vec![mf_na("CONS"), k_(mf_na("ast:build_system")), rec]),
    ]);
    // the whole handler = COMP(apply, CONS(build, id)): apply(build(P), P)
    // -- build the concrete handler, THEN apply it to the same P
    nseq(vec![mf_na("COMP"), mf_na("apply"), nseq(vec![mf_na("CONS"), build, mf_na("id")])])
}

// rp_own_table_handler mirrors engine.py:3167 create_handlers' own-table
// branch (`ast.build_system(cell_name=ft, machine=..., mealy_obj=...,
// links_obj=...)`), which is build_system's OWN tail (engine.py:79-103): the
// 9-slot record with a FIXED cell_name=ft and no validate/resolve/index/
// append (create_handlers passes none of those for an own-table fact type),
// reduced through ast:build_system ONCE -- the SAME record shape
// rp_create_from_spec's own non-absorbed branch assembles, stopping short of
// that function's final apply-to-a-fact step (there is no fact yet; this
// runs at compile time, before any write). The REDUCED handler is stored
// whole -- an own-table handler is fact-INDEPENDENT, unlike the absorbed
// tree above.
fn rp_own_table_handler(ev: &NEval, ft: &str, spec: &RpSpec) -> N {
    let slot = |v: Option<N>| match v {
        None => nseq(vec![]),
        Some(x) => nseq(vec![x]),
    };
    // EAGERLY reduced (ev.mu) -- see rp_absorbed_handler_tree's own comment:
    // python's machine_step/mealy_step/transitions_of are "thin canon
    // wrappers" that call the REAL reducer eagerly, so the record
    // build_system reduces here must never carry an unreduced reference in
    // these slots (this function's own OUTER ev.mu(napp(ast:build_system,
    // record)) reduces the RECORD's own top-level shape, but does not
    // itself guarantee every embedded CONS-carried sub-value gets forced --
    // found empirically by this slice's own byte differential, on the
    // ABSORBED sibling; applied here too since rp-fixture's own machine
    // exercises an own-table trigger ('reset') alongside the absorbed one).
    let machine_slot = match &spec.machine {
        None => nseq(vec![]),
        Some(m) => {
            let target = nseq(vec![
                mf_na(&m.status_table),
                N::A(Rc::new(Leaf::I(m.status_col))),
                N::A(Rc::new(Leaf::I(m.status_width))),
            ]);
            let role_atom = match m.role_pos {
                Some(p) => N::A(Rc::new(Leaf::I(p))),
                None => N::Bot,
            };
            nseq(vec![target, ev.mu(m.sm_obj.clone()), role_atom])
        }
    };
    let links_reduced = spec.links.clone().map(|n| ev.mu(n));
    let mealy_reduced = spec.mealy.clone().map(|n| ev.mu(n));
    let record = nseq(vec![
        mf_na(ft),
        nseq(vec![]), // validate: none for own-table (create_handlers passes none)
        nseq(vec![]), // resolve: none for own-table
        nseq(vec![]), // derive_obj: create never sets it
        slot(links_reduced),
        machine_slot,
        slot(mealy_reduced),
        nseq(vec![]), // index_cell: none for own-table
        nseq(vec![]), // append_cell: none for own-table
    ]);
    ev.mu(napp(mf_na("ast:build_system"), record))
}

// create_handlers_native (engine.py:3167 create_handlers): store create:<ft>
// handler cells for EVERY declared fact type -- the goal being full native
// for every part but lambda and defs, a create handler is a DEF the
// resident reduces over the fact, no host orchestration at write time. An
// own-table handler is fact-INDEPENDENT and stores build_system's reduction
// whole (rp_own_table_handler); an absorbed handler computes its cell name
// from the fact at reduce time (rp_absorbed_handler_tree), so apply_core
// serves BOTH shapes natively off the cell's presence alone -- already true
// today (apply_core/native_apply predate this slice and already consume
// whatever create:<ft> cells a store carries; this function is what MINTS
// them natively for the first time, closing the loop). Called at compile
// beside the layout, scheduler and generator cells; recompile replaces the
// family wholesale.
fn create_handlers_native(cells: &[(Leaf, V)], srv: &Srv) -> Result<Vec<(Leaf, V)>, String> {
    use std::collections::HashSet;
    let leaf = |s: &str| Leaf::S(s.to_string());
    let d0 = cells_to_d(cells);
    let nd = v_to_n(&d0);
    let ncells = n_cells_of(&nd);
    let ev = NEval {
        cells: ncells,
        process: srv.nprocess.clone(),
        defs_n: nd.clone(),
        fuel: std::cell::Cell::new(-1),
    };
    let (part, pairs_v) = mf_partition(&ev, &nd);
    let trig_fts: HashSet<String> = pop_rows(cells, &leaf("smTrigger"))
        .iter()
        .filter_map(|r| {
            let it = items(&list_of(r));
            if it.len() >= 2 {
                aval(&it[1]).map(|l| leaf_text(&l))
            } else {
                None
            }
        })
        .collect();

    let mut fresh: Vec<(Leaf, V)> = Vec::new();
    for f in pop_rows(cells, &leaf("factType")) {
        let it = items(&list_of(&f));
        if it.is_empty() {
            continue;
        }
        let ft = match aval(&it[0]) {
            Some(l) => leaf_text(&l),
            None => continue,
        };
        let is_trig = trig_fts.contains(&ft);
        let spec = rp_create_spec(cells, &ev, &nd, &part, &pairs_v, &ft, is_trig)
            .map_err(|e| format!("create_handlers: {}", e))?;
        let handler_n = if spec.absorbed {
            rp_absorbed_handler_tree(&ev, &spec, &ft)
        } else {
            rp_own_table_handler(&ev, &ft, &spec)
        };
        fresh.push((Leaf::S(format!("create:{}", ft)), n_to_v(&handler_n)));
    }

    let mut out: Vec<(Leaf, V)> = cells
        .iter()
        .filter(|(k, _)| !matches!(k, Leaf::S(s) if s.starts_with("create:")))
        .cloned()
        .collect();
    out.extend(fresh);
    Ok(out)
}

// with_watermark_native (protocol.py:364 _with_watermark): filter any
// existing eventWatermark cell(s) out, append a FRESH one at the END --
// layout_cells' own ordering discipline (python's `to_lam(cells + (new,))`,
// never Store's re-top-to-front), contents a ONE-ROW population ((n,),).
fn with_watermark_native(cells: &[(Leaf, V)], n: i64) -> Vec<(Leaf, V)> {
    let mut out: Vec<(Leaf, V)> = cells
        .iter()
        .filter(|(k, _)| !matches!(k, Leaf::S(s) if s == "eventWatermark"))
        .cloned()
        .collect();
    out.push((
        Leaf::S("eventWatermark".to_string()),
        seq(from_vec(vec![seqc(vec![atom(Leaf::I(n))])])),
    ));
    out
}

// rp_flush mirrors protocol.py:295-309 _flush: own-table union via Store
// (a rowsorted set union, VALUE-level, no key-based pruning), absorbed via
// mf_bulk_absorbed_install with replace_keys=false (python's own default:
// UNION -- retract/migrate never pass replace_keys=True either; only
// machine_fold's internal caller does, one status per entity). A no-op
// when buf is empty, matching python's `if not buf: return D` short circuit
// (skips the unconditional ev/nd rebuild too).
fn rp_flush(
    cells: &[(Leaf, V)],
    srv: &Srv,
    part: &HashMap<String, String>,
    pairs_v: &V,
    buf: &mut Vec<(String, Vec<V>)>,
) -> Vec<(Leaf, V)> {
    use std::collections::HashSet;
    if buf.is_empty() {
        return cells.to_vec();
    }
    let leaf = |s: &str| Leaf::S(s.to_string());
    let d0 = cells_to_d(cells);
    let nd = v_to_n(&d0);
    let ncells = n_cells_of(&nd);
    let ev = NEval {
        cells: ncells,
        process: srv.nprocess.clone(),
        defs_n: nd.clone(),
        fuel: std::cell::Cell::new(-1),
    };
    let mut out: Vec<(Leaf, V)> = cells.to_vec();
    for (ft, rows) in buf.drain(..) {
        let table = part.get(&ft).cloned().unwrap_or_else(|| ft.clone());
        if table != ft {
            out = mf_bulk_absorbed_install(&out, &ev, &table, &ft, &rows, pairs_v, false);
        } else {
            let mut have: Vec<V> = pop_rows(&out, &leaf(&ft));
            let mut seen: HashSet<String> = have.iter().map(key_of).collect();
            for r in &rows {
                if seen.insert(key_of(r)) {
                    have.push(r.clone());
                }
            }
            sort_rows(&mut have);
            store_move(&mut out, &ft, seq(from_vec(have)));
        }
    }
    out
}

// replay_entries_native (protocol.py:251 replay_entries): see the section
// header above for the four arms. `entries` is the parsed JSON entry list,
// sink-agnostic exactly as python's own signature promises -- the caller
// (op_compile_model) resolves "replay_entries" or "replay_path" into this
// slice before calling in.
fn replay_entries_native(
    cells: &[(Leaf, V)],
    srv: &Srv,
    entries: &[J],
) -> Result<Vec<(Leaf, V)>, String> {
    use std::collections::HashSet;
    let leaf = |s: &str| Leaf::S(s.to_string());
    let row_v = |xs: &[J]| seq(from_vec(xs.iter().map(to_v).collect()));
    let row_n = |xs: &[J]| N::S(Rc::new(xs.iter().map(j_to_n).collect()));

    // the FROZEN partition + trigger-ft set (python's part_box/trig_box) --
    // see the section header's justification
    let d0 = cells_to_d(cells);
    let nd0 = v_to_n(&d0);
    let ncells0 = n_cells_of(&nd0);
    let ev0 = NEval {
        cells: ncells0,
        process: srv.nprocess.clone(),
        defs_n: nd0.clone(),
        fuel: std::cell::Cell::new(-1),
    };
    let (part, pairs_v) = mf_partition(&ev0, &nd0);
    let trig_fts: HashSet<String> = pop_rows(cells, &leaf("smTrigger"))
        .iter()
        .filter_map(|r| {
            let it = items(&list_of(r));
            if it.len() >= 2 {
                aval(&it[1]).map(|l| leaf_text(&l))
            } else {
                None
            }
        })
        .collect();

    let mut spec_box: HashMap<String, RpSpec> = HashMap::new();
    // buf preserves FIRST-SEEN fact-type order (python dict insertion
    // order matters: it drives the table index's append order for freshly-
    // seen keys across DIFFERENT fact types sharing one table) -- a manual
    // assoc-vec, since a replay's distinct buffered fact types number in
    // the tens at most
    let mut buf: Vec<(String, Vec<V>)> = Vec::new();
    let mut out: Vec<(Leaf, V)> = cells.to_vec();

    for entry in entries {
        let op_str = match jget(entry, "op") {
            Some(J::S(s)) => Some(s.as_str()),
            _ => None,
        };

        if op_str == Some("retract") {
            out = rp_flush(&out, srv, &part, &pairs_v, &mut buf);
            let ft = match jget(entry, "ft") {
                Some(J::S(s)) => s.clone(),
                _ => return Err("retract entry missing ft".to_string()),
            };
            let fact = match jget(entry, "fact") {
                Some(J::A(xs)) => xs.clone(),
                _ => return Err("retract entry missing fact".to_string()),
            };
            let target = row_v(&fact);
            let mut rows: Vec<V> = pop_rows(&out, &leaf(&ft));
            rows.retain(|r| !eqobj(r, &target));
            // NO _rowsort -- protocol.py:318 skips it for retract
            store_move(&mut out, &ft, seq(from_vec(rows)));
            continue;
        }

        if op_str == Some("migrate") {
            out = rp_flush(&out, srv, &part, &pairs_v, &mut buf);
            let ft = match jget(entry, "ft") {
                Some(J::S(s)) => s.clone(),
                _ => return Err("migrate entry missing ft".to_string()),
            };
            let facts = match jget(entry, "facts") {
                Some(J::A(xs)) => xs.clone(),
                _ => return Err("migrate entry missing facts".to_string()),
            };
            let rows: Vec<V> = facts
                .iter()
                .map(|f| match f {
                    J::A(xs) => row_v(xs),
                    other => seq(cons(to_v(other), nil())),
                })
                .collect();
            let table = part.get(&ft).cloned().unwrap_or_else(|| ft.clone());
            if table != ft {
                let d0m = cells_to_d(&out);
                let ndm = v_to_n(&d0m);
                let ncellsm = n_cells_of(&ndm);
                let evm = NEval {
                    cells: ncellsm,
                    process: srv.nprocess.clone(),
                    defs_n: ndm.clone(),
                    fuel: std::cell::Cell::new(-1),
                };
                out = mf_bulk_absorbed_install(&out, &evm, &table, &ft, &rows, &pairs_v, false);
            } else {
                let mut have: Vec<V> = pop_rows(&out, &leaf(&ft));
                let mut seen: HashSet<String> = have.iter().map(key_of).collect();
                for r in &rows {
                    if seen.insert(key_of(r)) {
                        have.push(r.clone());
                    }
                }
                sort_rows(&mut have);
                store_move(&mut out, &ft, seq(from_vec(have)));
            }
            continue;
        }

        // plain or trigger (op absent, or any OTHER value -- python's exact
        // fallthrough: only "retract"/"migrate" are special-cased, anything
        // else -- including no "op" key at all -- reads entry["ft"]/["fact"])
        let ft = match jget(entry, "ft") {
            Some(J::S(s)) => s.clone(),
            _ => return Err("entry missing ft".to_string()),
        };
        let fact = match jget(entry, "fact") {
            Some(J::A(xs)) => xs.clone(),
            _ => return Err("entry missing fact".to_string()),
        };

        if trig_fts.contains(&ft) {
            out = rp_flush(&out, srv, &part, &pairs_v, &mut buf);
            let spec = match spec_box.get(&ft) {
                Some(s) => s.clone(),
                None => {
                    // is_trigger: true -- this arm is only ever reached for
                    // ft in trig_fts (the `if trig_fts.contains(&ft)` guard
                    // above), the exact call shape rp_create_spec's own
                    // is_trigger parameter documents; unchanged behavior.
                    let s = rp_create_spec(cells, &ev0, &nd0, &part, &pairs_v, &ft, true)?;
                    spec_box.insert(ft.clone(), s.clone());
                    s
                }
            };
            let d0t = cells_to_d(&out);
            let ndt = v_to_n(&d0t);
            let ncellst = n_cells_of(&ndt);
            let evt = NEval {
                cells: ncellst,
                process: srv.nprocess.clone(),
                defs_n: ndt.clone(),
                fuel: std::cell::Cell::new(-1),
            };
            let fact_n = row_n(&fact);
            let d2n = rp_create_from_spec(&evt, &ndt, &ft, fact_n, &spec);
            out = cells_of(&n_to_v(&d2n));
        } else {
            let row = row_v(&fact);
            match buf.iter_mut().find(|(f, _)| f == &ft) {
                Some((_, rows)) => rows.push(row),
                None => buf.push((ft, vec![row])),
            }
        }
    }
    Ok(rp_flush(&out, srv, &part, &pairs_v, &mut buf))
}

fn op_compile_model(j: &J, srv: &mut Srv) -> Result<String, String> {
    use std::collections::{BTreeMap, HashMap, HashSet};
    // args parse before anything runs (op_run_rules' discipline: a malformed
    // request mutates nothing)
    let text = match jget(j, "text") {
        Some(J::S(t)) => t.clone(),
        _ => return Err("compile_model needs a string text".to_string()),
    };
    let fuel = match jget(j, "fuel") {
        Some(J::I(n)) if *n > 0 => Some(*n),
        _ => None,
    };
    let leaf = |s: &str| Leaf::S(s.to_string());
    let strv = |x: &V| aval(x).and_then(|l| leaf_str(&l));

    // the grammar data: the dispatch table (Classification_has_Translator,
    // python compile_model_selfhost's first move) and the stage-1 vocabulary
    // (classLit, python stage1_vocabulary — the tokenizer knows nothing else).
    // Read off the RESIDENT store first (an ingested-grammar resident serves
    // itself); when the resident lacks the grammar cells (the classLit probe),
    // THAW the compiled grammar sidecar into a classification SCRATCH — the
    // resident-kernel grammar_D(). The scratch swaps in only around the batch
    // derive below; the resident store is restored whole.
    let (mut dispatch, mut vocab) = grammar_tables(&srv.cells);
    let mut grammar_src = String::from("resident");
    let mut scratch: Option<GrammarScratch> = None;
    let mut missing: Vec<String> = Vec::new();
    if vocab.is_empty() {
        match load_grammar_scratch(j) {
            Ok((g, path)) => {
                let (d2, v2) = grammar_tables(&g.1);
                dispatch = d2;
                vocab = v2;
                grammar_src = path;
                scratch = Some(g);
                if vocab.is_empty() {
                    missing.push(format!(
                        "grammar sidecar {} carries no classLit rows (not a compiled grammar store?)",
                        grammar_src
                    ));
                }
            }
            Err(e) => missing.push(e),
        }
    }
    if dispatch.is_empty() {
        missing.push(
            "no Classification_has_Translator rows (grammar store absent or partial)"
                .to_string(),
        );
    }
    // (a) statements + (b) modality split; a possibility statement is the
    // absence of a constraint (informational) and never enters the g-loop
    let stmts = split_statements(&text);
    let total = stmts.len();
    let mut work: Vec<(String, &'static str, String, &'static str)> = Vec::new();
    for stmt in &stmts {
        let (m, sg, inner) = split_modality(stmt);
        if sg != "possibility" {
            work.push((stmt.clone(), m, inner, sg));
        }
    }

    // the PREPASS (python: _known + _prepass_context, the context seam):
    // declared names (plus the base's, read off the RESIDENT store when the
    // caller passes context_from:"resident" — python's context_from store),
    // the subtype closure, fact-type slugs, and the plain reading set
    let context_resident = matches!(jget(j, "context_from"), Some(J::S(s)) if s == "resident");
    let (b_names, b_edges, b_fts, b_vals) = if context_resident {
        context_of(&srv.cells)
    } else {
        (HashSet::new(), Vec::new(), HashSet::new(), HashSet::new())
    };
    let mut names = known_names(&stmts);
    for n in b_names {
        names.insert(n);
    }
    // #31: the value-type names (in-text ∪ base) ride the context so the fact
    // cook can coerce quoted literals on value-typed roles
    let mut vals = known_vals(&stmts);
    for v in b_vals {
        vals.insert(v);
    }
    let b_fts_vec: Vec<String> = b_fts.into_iter().collect();
    let (subs, fts, plain) = prepass_context(&stmts, &names, &b_edges, &b_fts_vec);
    // the nouns Stage-1 scans for Role References: the known names, ordered
    // longest-first like _known's return (rows dedup + sort below, so order
    // never reaches the store)
    let mut nouns: Vec<String> = names.iter().cloned().collect();
    nouns.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));

    // (c) BATCH classification (classify_all_via_M, compiler.py:1204): every
    // statement's field facts land first under s1..sN, ONE derive answers all
    // classifications.
    let mut by_cell: BTreeMap<String, Vec<V>> = BTreeMap::new();
    for (i, (_stmt, _m, inner, _sg)) in work.iter().enumerate() {
        let sid = format!("s{}", i + 1);
        for (ftb, s, v) in stage1_rows_of(inner, &vocab, &nouns, &sid) {
            by_cell
                .entry(ftb)
                .or_default()
                .push(seq(from_vec(vec![atom(Leaf::S(s)), atom(Leaf::S(v))])));
        }
    }
    // the SCRATCH discipline: field facts and the derived classifications are
    // never part of the model — python threads an immutable D and discards it;
    // the resident kernel saves the store whole, swaps in the thawed grammar
    // when one was loaded (python's gD), and restores the resident after the
    // read. nprocess joins the save because the grammar sidecar carries its
    // own compiled process defs for the derive's native carrier.
    let saved_d = srv.d.clone();
    let saved_cells = srv.cells.clone();
    let saved_nd = srv.nd.clone();
    let saved_ncells = srv.ncells.clone();
    let saved_nprocess = srv.nprocess.clone();
    let mut cls_by_sid: HashMap<String, HashSet<String>> = HashMap::new();
    if !by_cell.is_empty() {
        if let Some((gd, gcells, gnd, gncells, gproc)) = scratch {
            srv.d = gd;
            srv.cells = gcells;
            srv.nd = gnd;
            srv.ncells = gncells;
            srv.nprocess = gproc;
        }
        for (ftb, rows) in &by_cell {
            let name = leaf(ftb);
            let old = pop_rows(&srv.cells, &name);
            let mut merged: Vec<V> = Vec::new();
            let mut keys: HashSet<String> = HashSet::new();
            for r in old.iter().chain(rows.iter()) {
                if keys.insert(key_of(r)) {
                    merged.push(r.clone());
                }
            }
            sort_rows(&mut merged);
            store_into(
                &mut srv.d,
                &mut srv.cells,
                &mut srv.nd,
                &mut srv.ncells,
                &name,
                seq(from_vec(merged)),
            );
        }
        // ONE run_rules over the grammar's recognizer rules, the frontier the
        // field cells (python run_rules(D, changed=set(by_cell)))
        let frontier_req = J::O(vec![(
            "changed".to_string(),
            J::A(by_cell.keys().map(|k| J::S(k.clone())).collect()),
        )]);
        let derived = op_run_rules(&frontier_req, srv);
        if derived.is_ok() {
            for r in pop_rows(&srv.cells, &leaf("Statement_has_Classification")) {
                let it = items(&list_of(&r));
                if it.len() >= 2 {
                    if let (Some(s), Some(c)) = (strv(&it[0]), strv(&it[1])) {
                        cls_by_sid.entry(s).or_default().insert(c);
                    }
                }
            }
        }
        srv.d = saved_d;
        srv.cells = saved_cells;
        srv.nd = saved_nd;
        srv.ncells = saved_ncells;
        srv.nprocess = saved_nprocess;
        derived?;
    }

    // (d) the dispatch loop (compile_model_selfhost's per-statement body):
    // Prose beats the GENERIC fallbacks only; a negative alethic statement no
    // specific rule claimed is a constraint by definition and goes loud; the
    // classification set's translators dispatch in sorted order through rho.
    const GENERIC: [&str; 2] = ["Fact Type Reading", "Instance Fact"];
    // meta.initial_D() (compiler.py:71): one FILE cell — OR, under
    // context_from:"resident", the resident store's OWN raw cell sequence
    // (shadowed duplicates included; see raw_cells_of), so the fold
    // continues exactly where the preloaded store leaves off
    let mut model_cells: Vec<(Leaf, V)> = if context_resident {
        raw_cells_of(&srv.d)
    } else {
        initial_d_cells()
    };
    // the context operand, python's to_lam((tuple(sorted(names)),
    // tuple(sorted((s, tuple(sorted(a))) for s, a in subs.items())),
    // tuple(sorted(fts)), tuple(sorted(plain)))) — all four sorted, the
    // subtype closure as ⟨name, ancestors⟩ pairs
    let atom_s = |s: &str| atom(Leaf::S(s.to_string()));
    let mut names_sorted: Vec<String> = names.iter().cloned().collect();
    names_sorted.sort();
    let subs_pairs: Vec<V> = subs
        .iter()
        .map(|(s, anc)| {
            seqc(vec![
                atom_s(s),
                seq(from_vec(anc.iter().map(|a| atom_s(a)).collect())),
            ])
        })
        .collect();
    let mut vals_sorted: Vec<String> = vals.iter().cloned().collect();
    vals_sorted.sort();
    let ctx = seqc(vec![
        seq(from_vec(names_sorted.iter().map(|n| atom_s(n)).collect())),
        seq(from_vec(subs_pairs)),
        seq(from_vec(fts.iter().map(|f| atom_s(f)).collect())),
        seq(from_vec(plain.iter().map(|f| atom_s(f)).collect())),
        seq(from_vec(vals_sorted.iter().map(|v| atom_s(v)).collect())),
    ]);
    let empty_cls: HashSet<String> = HashSet::new();
    let mut unclassified: Vec<String> = Vec::new();
    let mut prose: Vec<String> = Vec::new();
    let mut blocked: Vec<String> = Vec::new();
    let mut classified = 0usize;
    // slice 1's identity-skip witness: true the instant ANY fold_fire or
    // canon-DEF adoption actually mutates model_cells — "the fold produced
    // no cells beyond the seed" (the probe-app case) is exactly !folded_any
    let mut folded_any = false;
    // the native cook context (#20): the SAME names/subs/fts/plain/vals the
    // ctx operand carries, in cooks form, built once per compile
    let kn = cooks::Known::new(&names, &subs, &fts, &plain, &vals);
    // {"trace":1} answers the per-statement ⟨asserts, objs⟩ emissions — the
    // differential's dump (python compile per-statement fires, verbatim)
    let trace_on = matches!(jget(j, "trace"), Some(J::I(1)));
    // {"dump_store":1} answers the folded D itself, write_v form — the
    // model_d fold's own acceptance surface (python compile_model_selfhost's
    // returned D, full from_lam)
    let dump_store_on = matches!(jget(j, "dump_store"), Some(J::I(1)));
    let mut translated: Vec<String> = Vec::new();
    for (i, (stmt, m, inner, sg)) in work.iter().enumerate() {
        let sid = format!("s{}", i + 1);
        let cls = cls_by_sid.get(&sid).unwrap_or(&empty_cls);
        if !cls.is_empty() {
            classified += 1;
        }
        // Prose beats the generics AND the rule claim — except machine-keyword
        // statements (the arrow-glue-loud class), reported instead of prosed
        let mut residual = cls.clone();
        residual.remove("Prose");
        residual.remove("Derivation Rule");
        for g in GENERIC {
            residual.remove(g);
        }
        // every WORK statement earns a trace entry, the guard exits included
        // (the differential aligns per statement; a skipped entry misaligns)
        let trace_empty = |stmt: &str, translated: &mut Vec<String>| {
            let mut e = String::from("{\"stmt\":");
            esc(stmt, &mut e);
            e.push_str(",\"fires\":[]}");
            translated.push(e);
        };
        if cls.contains("Prose") && residual.is_empty() {
            if sm_suspect(stmt) {
                unclassified.push(stmt.clone());
            } else {
                prose.push(stmt.clone());
            }
            if trace_on {
                trace_empty(stmt, &mut translated);
            }
            continue;
        }
        let specific: Vec<String> = cls
            .iter()
            .filter(|c| !GENERIC.contains(&c.as_str()))
            .cloned()
            .collect();
        if specific.is_empty() && *sg == "negative" && *m == "alethic" {
            // a NEGATIVE alethic statement is a constraint by definition; the
            // generic fallbacks must never declare a fact type from it
            unclassified.push(stmt.clone());
            if trace_on {
                trace_empty(stmt, &mut translated);
            }
            continue;
        }
        let mut sorted_cls: Vec<String> = if specific.is_empty() {
            cls.iter().cloned().collect()
        } else {
            specific
        };
        sorted_cls.sort();
        let mut translators: Vec<String> = Vec::new();
        for c in &sorted_cls {
            if let Some(ts) = dispatch.get(c) {
                for t in ts {
                    if !translators.contains(t) {
                        translators.push(t.clone());
                    }
                }
            }
        }
        if translators.is_empty() {
            unclassified.push(stmt.clone());
            if trace_on {
                trace_empty(stmt, &mut translated);
            }
            continue;
        }
        // deontic carries its operator sign through the modality field
        let mfield = if *m == "deontic" {
            format!("{}:{}", m, sg)
        } else {
            (*m).to_string()
        };
        let mut accepted = false;
        let mut fires: Vec<String> = Vec::new();
        for t in &translators {
            if translator_kinds(t).is_empty() {
                // python's graceful absence: a name M declares that this host
                // has not registered is intentionally absent — handled, never
                // refused (the gate-three contract)
                accepted = true;
                continue;
            }
            // rho: dispatch through DEFS via the reducer — the direct analog
            // of python D = _apply(_A(t), operand). When #18 lands a canon
            // translator DEF this arm runs it for free; a host-only name
            // reduces to ⊥ (or stays a stuck app) and the NATIVE cook path
            // below translates instead (#20: the ported _COOK boundary).
            let operand = seqc(vec![
                atom(Leaf::S(inner.clone())),
                atom(Leaf::S(mfield.clone())),
                ctx.clone(),
                cells_to_d(&model_cells),
            ]);
            let res = reduce_over(srv, atom(Leaf::S(t.clone())), operand, fuel);
            if matches!(shape(&res), Shape::Seq(_)) && !isapp(&res) {
                // a canon translator DEF answered D' directly (python's own
                // D = _apply(_A(t), operand)) — adopt it whole, raw (a
                // canon def may thread shadowed duplicates too; #18's
                // future seam, kept correct even though dormant today)
                model_cells = raw_cells_of(&res);
                accepted = true;
                folded_any = true;
                continue;
            }
            // the panic fence (#32): a cook tripping a guarded-by-construction
            // unwrap on an adversarial reading must degrade to python's
            // per-statement semantics (raise -> unclassified, compile
            // continues), never kill the resident. Payload -> the Err lane.
            let cooked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || native_cook(t, inner, &mfield, &kn, srv),
            ))
            .unwrap_or_else(|p| {
                let msg = p
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| p.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "non-string panic payload".into());
                Err(format!("cook panicked: {}", msg))
            });
            match cooked {
                Ok(Some(fire)) => {
                    // the translator fired: python's _plan answered its
                    // ⟨asserts, objs⟩ (the acceptance surface of the
                    // differential); acceptance mirrors _apply succeeding.
                    // The fold (#20, this slice): asserts THEN objs, in
                    // emission order (compiler.py's g()) — a native
                    // run_append/DefineIn twin per entry.
                    accepted = true;
                    if let Err(e) = fold_fire(&fire, &mut model_cells) {
                        return Err(format!(
                            "compile_model fold: {} (statement: {})",
                            e, stmt
                        ));
                    }
                    folded_any = true;
                    if trace_on {
                        let mut f = String::new();
                        cooks::fire_json(t, &fire, &mut f);
                        fires.push(f);
                    }
                }
                Ok(None) => {
                    // no production matched: the translator's own refusal
                    // (python raise ValueError -> the dispatcher's except);
                    // dispatch continues to the next translator
                }
                Err(reason) => {
                    // a handler refusing its statement is that handler's
                    // verdict (python except ValueError: continue). A canon
                    // reduction failing is a PORT gap instead — reported.
                    if trace_on {
                        let mut f = String::from("{\"t\":");
                        esc(t, &mut f);
                        f.push_str(",\"refused\":");
                        esc(&reason, &mut f);
                        f.push('}');
                        fires.push(f);
                    }
                    if reason.starts_with("canon ") || reason.starts_with("no canon def") {
                        if !blocked.contains(&reason) {
                            blocked.push(reason);
                        }
                    }
                }
            }
        }
        if !accepted {
            // NO translator accepted: reported loudly — never a silent vanish
            unclassified.push(stmt.clone());
        }
        if trace_on {
            let mut e = String::from("{\"stmt\":");
            esc(stmt, &mut e);
            e.push_str(",\"fires\":[");
            e.push_str(&fires.join(","));
            e.push_str("]}");
            translated.push(e);
        }
    }
    // slice 1 tail (native pipeline tail, #20): rekey_transitions
    // (machine-scope transition identity — compiler.py's compile_model
    // wrapper applies it right after the fold: D2 = system.rekey_transitions(D2))
    rekey_transitions_native(&mut model_cells);
    // then the post-model rules fixpoint (protocol.py:1815's separate
    // system.run_rules(D, ...) call, made after compile_model returns) through
    // the EXISTING native op_run_rules machinery — the SAME save/swap/restore
    // discipline the batch-classification derive above already uses, so this
    // op stays pure: the resident's own store is read for dispatch throughout
    // and left exactly as found, never overwritten with the compiled model.
    // Identity-skip: a fold that never fired (no cell mutation, no canon-DEF
    // adoption — the probe-app case) leaves model_cells identical to the seed,
    // and running the fixpoint over an unchanged, already-derived seed is a
    // proven no-op (round one finds nothing new and breaks immediately) — so
    // it is skipped here to save the setup cost, never to change the answer.
    if folded_any {
        let saved_d2 = srv.d.clone();
        let saved_cells2 = srv.cells.clone();
        let saved_nd2 = srv.nd.clone();
        let saved_ncells2 = srv.ncells.clone();
        srv.d = cells_to_d(&model_cells);
        srv.cells = model_cells.clone();
        srv.nd = v_to_n(&srv.d);
        srv.ncells = n_cells_of(&srv.nd);
        let rules_req = J::O(Vec::new());
        let derived2 = op_run_rules(&rules_req, srv);
        if derived2.is_ok() {
            // harvest from the TRUE store (srv.d), not the index Vec: python's
            // Store re-tops every write, so a head cell NEW in the rules phase
            // sits at the FRONT of D — the index Vec appends it at the end,
            // which is a different (wrong) dump order (core.md's two
            // new-in-rules heads, found by the order forensics 2026-07-11)
            model_cells = raw_cells_of(&srv.d);
        }
        srv.d = saved_d2;
        srv.cells = saved_cells2;
        srv.nd = saved_nd2;
        srv.ncells = saved_ncells2;
        derived2?;
    }
    // ======================= status_facts -> machine_fold -> (rules iff
    // changed) -> layout_cells (#20, the machine_fold port slice) =========
    // protocol.py:1760-1801's phase order, continued from the post-model
    // fixpoint above: status_facts (engine.py:3516) mints each governed
    // Object Type's "is currently in Status" fact type via a NESTED compile
    // (status_facts_native's own compile_lines_native sub-call) so RMAP
    // absorbs it as a column before the fold writes; machine_fold
    // (engine.py:2780) then folds every readings-carried machine event to
    // its final status, greedy per entity; the conditional run_rules
    // mirrors protocol.py:1842's `if D2 is not D`; layout_cells
    // (engine.py:1693) materializes the rmapColumns cell unconditionally.
    // machine_fold itself reads only whatever fact populations already
    // exist when it runs — readings-carried machine events land there at
    // fold time (this section, unchanged since the machine_fold slice);
    // LOG-carried events (#20, the replay slice, immediately below) land
    // there one phase earlier, between status_facts and machine_fold, so
    // by the time machine_fold runs the two sources are indistinguishable
    // population rows.
    model_cells = status_facts_native(j, &model_cells, srv)
        .map_err(|e| format!("status_facts: {}", e))?;
    // ======================= replay (#20, the replay port slice) ===========
    // protocol.py:1773-1778's pipeline seat, between status_facts and
    // machine_fold: the log-carried event stream replays through the SAME
    // create the live apply path uses (replay_entries_native, above), then
    // an UNBOUNDED post-replay run_rules (protocol.py:1777 -- no frontier,
    // the reverted-frontier lesson: mirror exactly, never innovate
    // boundaries). Gated on the request carrying either "replay_entries"
    // (an inline JSON array -- keeps the op pure, entries resolved by the
    // caller) or "replay_path" (a jsonl path the op reads itself, one JSON
    // object per line in file order, mirroring append_event's writer format
    // -- the resident MCP flow's own seat, since apps_compile still
    // delegates whole today and has no entries to hand in-process).
    //
    // Absent either field, OR present but resolving to zero entries: this
    // whole phase (replay AND the watermark stamp) is SKIPPED outright --
    // not merely a no-op walk through replay_entries_native, but never
    // entered at all, so the ten standing corpora's existing dump_store
    // contract (mf-compare.py et al, no field ever passed) is untouched
    // byte for byte, and an app with an event sink but nothing in it yet
    // (or a caller not yet replay-aware) reads identically to one with no
    // sink at all -- naming.md's "replay_entries":[] acceptance case. This
    // is a deliberate, narrow departure from protocol.py's own literal
    // always-watermark (Registry.compile stamps eventWatermark
    // unconditionally, even at len(entries)==0): here the stamp only
    // appears once there is something to have replayed. The next slice
    // that ports the FULL Registry.compile boundary (scheduler_cells/
    // generator_cells/create_handlers/save) will lift this gate and stamp
    // watermark(0) unconditionally, matching python exactly again --
    // flagged here so that slice expects the boundary shift.
    let replay_entries_json: Option<Vec<J>> = match jget(j, "replay_entries") {
        Some(J::A(xs)) => Some(xs.clone()),
        _ => match jget(j, "replay_path") {
            Some(J::S(p)) => {
                let text = std::fs::read_to_string(p)
                    .map_err(|e| format!("replay_path {}: {}", p, e))?;
                let mut out = Vec::new();
                for raw_line in text.split('\n') {
                    // mirror append_event's writer: one JSON object per
                    // line, file order. CRLF-normalized (trim a trailing
                    // \r): the FIRST native path reading a caller-external
                    // jsonl file directly rather than pre-supplied text --
                    // read_base_text's own CRLF lesson (#20, the seed
                    // slice) applies here too.
                    let line = raw_line.trim_end_matches('\r');
                    if line.trim().is_empty() {
                        continue;
                    }
                    match parse_json(line) {
                        Some(v) => out.push(v),
                        None => {
                            return Err(format!("replay_path {}: malformed json line", p))
                        }
                    }
                }
                Some(out)
            }
            _ => None,
        },
    };
    if let Some(entries) = &replay_entries_json {
        if !entries.is_empty() {
            model_cells = replay_entries_native(&model_cells, srv, entries)
                .map_err(|e| format!("replay: {}", e))?;
            // post-replay run_rules, UNBOUNDED (protocol.py:1777) -- the
            // SAME save/swap/restore discipline the post-model/post-fold
            // blocks above already use
            let saved_d4 = srv.d.clone();
            let saved_cells4 = srv.cells.clone();
            let saved_nd4 = srv.nd.clone();
            let saved_ncells4 = srv.ncells.clone();
            srv.d = cells_to_d(&model_cells);
            srv.cells = model_cells.clone();
            srv.nd = v_to_n(&srv.d);
            srv.ncells = n_cells_of(&srv.nd);
            let rules_req4 = J::O(Vec::new());
            let derived4 = op_run_rules(&rules_req4, srv);
            if derived4.is_ok() {
                model_cells = raw_cells_of(&srv.d);
            }
            srv.d = saved_d4;
            srv.cells = saved_cells4;
            srv.nd = saved_nd4;
            srv.ncells = saved_ncells4;
            derived4?;
        }
    }
    let (mf_cells, mf_changed) = machine_fold_native(&model_cells, srv);
    model_cells = mf_cells;
    if mf_changed {
        let saved_d3 = srv.d.clone();
        let saved_cells3 = srv.cells.clone();
        let saved_nd3 = srv.nd.clone();
        let saved_ncells3 = srv.ncells.clone();
        srv.d = cells_to_d(&model_cells);
        srv.cells = model_cells.clone();
        srv.nd = v_to_n(&srv.d);
        srv.ncells = n_cells_of(&srv.nd);
        let rules_req3 = J::O(Vec::new());
        let derived3 = op_run_rules(&rules_req3, srv);
        if derived3.is_ok() {
            model_cells = raw_cells_of(&srv.d);
        }
        srv.d = saved_d3;
        srv.cells = saved_cells3;
        srv.nd = saved_nd3;
        srv.ncells = saved_ncells3;
        derived3?;
    }
    // the watermark stamp (protocol.py:1847, `persist._with_watermark`):
    // filters the old eventWatermark cell, appends the new one LAST, rows
    // ((len entries,),). UNCONDITIONAL (#20, the final pipeline slice --
    // THE GATE REWORKED): the replay slice's own draft gated this stamp on
    // "did the request actually carry entries", because that slice's
    // acceptance (naming.md's "replay_entries":[] case) demanded byte
    // identity with the PRE-replay boundary, which had no eventWatermark
    // cell at all -- a deliberate, narrow, forward-flagged departure from
    // protocol.py's own literal always-watermark (Registry.compile stamps
    // eventWatermark unconditionally, even at len(entries)==0; entries =
    // self._sink(name).read(), always a list, never None). THIS slice lifts
    // that gate, matching protocol.py:1847 exactly: watermark(0) now stamps
    // even when the request never carried a replay field at all. The
    // naming.md corpus's fresh acceptance dump reflects this directly -- an
    // eventWatermark((0,)) cell that the pre-lift dump never carried -- the
    // lifted gate's own proof, named in the task's acceptance list.
    let watermark_n: i64 = replay_entries_json.as_ref().map(|e| e.len()).unwrap_or(0) as i64;
    model_cells = with_watermark_native(&model_cells, watermark_n);
    model_cells = layout_cells_native(&model_cells, srv);
    // ======================= scheduler_cells -> generator_cells ->
    // create_handlers (#20, the final pipeline slice, completing the
    // boundary) =============================================================
    // protocol.py:1849-1852's own order, continued right after layout_cells:
    // scheduler_cells (engine.py:1797) materializes the passHeads/passOrder/
    // passBound cells through classify_heads_native -- the SAME classifier
    // op_run_rules' own absent-passHeads fallback shares, defined above in
    // this file ("share the code, do not duplicate" per the task); generator_
    // cells (engine.py:1825) mints the dsl:<Noun> cells (the opt-in XSD/OWL/
    // EDM/etc. family is deliberately not ported -- see generator_cells_
    // native's own comment: zero acceptance corpus ever sets
    // App_uses_Generator, so python's own `active` set is always empty
    // there too); create_handlers (engine.py:3167) mints create:<ft> handler
    // cells -- consumed by the resident's ALREADY-SHIPPED write path
    // (apply_core/native_apply, landed well before this slice), which until
    // now has only ever served stores some OTHER writer (the python CLI)
    // minted create: cells for. This closes that loop natively.
    model_cells = scheduler_cells_native(&model_cells, srv);
    model_cells = generator_cells_native(&model_cells, srv);
    model_cells = create_handlers_native(&model_cells, srv)
        .map_err(|e| format!("create_handlers: {}", e))?;
    // ======================= save: <app>.store.json sidecar (#20) =========
    // protocol.py:1854-1857's `drv.save(D); self._sidecar(name, D)` -- the
    // ONE side effect this otherwise-pure op ever performs, and only when
    // the request explicitly asks for it via "save_path": "<path>" (an
    // explicit path field, matching the base_seed/replay_path convention:
    // these ops resolve every path from the request itself, never from an
    // Apps registry object op_compile_model was never handed). Reuses
    // write_sidecar's core (sidecar_payload, defined beside write_sidecar)
    // with the SAME <d, process> shape python's own _sidecar(name, D)
    // serializes: process is NCANON (the compiled canon this binary loads
    // once at startup -- the exact analog of python's _defs.latest filtered
    // to kind=="compiled" entries, which canon.load_all() populates once at
    // package import, corpus-independent) PLUS host_bootstrap_defs (the 8
    // python-only names engine.py binds directly at module level, outside
    // canon.load_all() -- transcribed by hand, see that function's own
    // comment) -- 325 + 8 = 333, python's own total. Not reflected in the
    // response JSON (python's own save/sidecar are side effects too, never
    // rep fields); "saved" rides only as a diagnostic convenience.
    let mut saved_path: Option<String> = None;
    if let Some(J::S(path)) = jget(j, "save_path") {
        let d_final = cells_to_d(&model_cells);
        let process: Vec<(String, N)> = NCANON
            .with(|nc| nc.borrow().clone())
            .into_iter()
            .chain(host_bootstrap_defs())
            .collect();
        let payload = sidecar_payload(&d_final, &process);
        let out_path = std::path::PathBuf::from(path);
        let mut tmp = out_path.clone().into_os_string();
        tmp.push(format!(".{}.tmp", std::process::id()));
        let tmp_path = std::path::PathBuf::from(tmp);
        std::fs::write(&tmp_path, payload.as_bytes())
            .map_err(|e| format!("save_path {}: {}", out_path.display(), e))?;
        std::fs::rename(&tmp_path, &out_path)
            .map_err(|e| format!("save_path rename {}: {}", out_path.display(), e))?;
        saved_path = Some(out_path.display().to_string());
    }
    // rule_diagnostics: the ruleDiag population AFTER rules (python's own
    // compiler.py:2489 reads it right after rekey, BEFORE the pipeline's
    // separate run_rules call — but ruleDiag is a STAGE-1 COMPILE diagnostic,
    // written only by the rule_if/rule_iff cook when a rule body fails to
    // compile, never a derivation rule's HEAD in any known corpus, so
    // run_rules never adds to it; reading it here, after everything, equals
    // python's snapshot by construction)
    let rulediag_rows: Vec<V> = pop_rows(&model_cells, &leaf("ruleDiag"));
    let mut rulediag_json = String::from("[");
    for (i, row) in rulediag_rows.iter().enumerate() {
        if i > 0 {
            rulediag_json.push(',');
        }
        write_v(row, &mut rulediag_json);
    }
    rulediag_json.push(']');
    // the report: the seed contract's surviving keys (total/unclassified/prose)
    // plus the honest diagnostics (classified/grammar/missing/blocked)
    let mut r = String::from("{\"total\":");
    r.push_str(&total.to_string());
    r.push_str(",\"classified\":");
    r.push_str(&classified.to_string());
    r.push_str(",\"grammar\":");
    esc(&grammar_src, &mut r);
    let arr = |xs: &[String], out: &mut String| {
        out.push('[');
        for (i, s) in xs.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            esc(s, out);
        }
        out.push(']');
    };
    r.push_str(",\"unclassified\":");
    arr(&unclassified, &mut r);
    r.push_str(",\"prose\":");
    arr(&prose, &mut r);
    r.push_str(",\"missing\":");
    arr(&missing, &mut r);
    r.push_str(",\"blocked\":");
    arr(&blocked, &mut r);
    // slice 2 (native pipeline tail, #20): the Python rep contract's exact
    // field names (compiler.py:2490), alongside the skeleton's own honest
    // diagnostics above — total/prose already match; kinds is always the
    // selfhost path's empty dict (a seed leftover, emitted verbatim, never
    // innovated on); unparsed mirrors unclassified under python's external
    // name; rule_diagnostics is the ruleDiag rows built above
    r.push_str(",\"kinds\":{}");
    r.push_str(",\"unparsed\":");
    arr(&unclassified, &mut r);
    r.push_str(",\"rule_diagnostics\":");
    r.push_str(&rulediag_json);
    if trace_on {
        // the differential dump: per statement, the fires' ⟨asserts, objs⟩
        r.push_str(",\"translated\":[");
        r.push_str(&translated.join(","));
        r.push(']');
    }
    if dump_store_on {
        // the store as of THIS op's own boundary (#20, NOW THE COMPLETE
        // Registry.compile boundary, minus the sql projection): fold ->
        // rekey_transitions -> post-model run_rules -> status_facts ->
        // [replay -> run_rules, IFF the request carried "replay_entries"/
        // "replay_path" resolving to at least one entry -- see the replay
        // section's own comment for the gate's exact shape and the
        // naming.md-empty-list acceptance case it exists for] ->
        // machine_fold -> run_rules IFF machine_fold changed anything ->
        // watermark (UNCONDITIONAL, #20) -> layout_cells -> scheduler_cells
        // -> generator_cells -> create_handlers, full from_lam — python's
        // compile_model (WITH rekey) + protocol.py's own run_rules/
        // status_facts/replay/(run_rules)/machine_fold/(run_rules)/
        // watermark/layout_cells/scheduler_cells/generator_cells/
        // create_handlers calls (protocol.py:1793-1852 read in full). Only
        // drv.save/_sidecar (side effects, see "save_path" above) and the
        // sql projection (see "sql_project" below) sit outside dump_store's
        // own "store" field, matching python's own rep shape (drv.save and
        // _sidecar are never reflected in rep either). (Earlier slices'
        // narrower boundaries are no longer what dump_store answers; those
        // comparisons live in their own reference scripts calling the
        // narrower python boundary directly.)
        r.push_str(",\"store\":");
        write_v(&cells_to_d(&model_cells), &mut r);
    }
    if let Some(p) = &saved_path {
        r.push_str(",\"saved\":");
        esc(p, &mut r);
    }
    // ======================= optional sql projection (#20) ================
    // protocol.py:1862-1864's `if drv.sql: rep["projected"] = drv.project(D)`
    // -- mirrored behind an EXPLICIT flag ("sql_project":1) since a native op
    // has no storage-driver object to read a `.sql` attribute from (every
    // object-backend/SQL-backend choice a caller would otherwise make
    // through `drv.sql` collapses to this one boolean here). op_sql_project
    // already exists at certified byte parity (an earlier slice) and is
    // reused verbatim, over model_cells via the SAME save/swap/restore
    // discipline this op uses throughout -- never re-implemented. op_sql_
    // project reads only srv.cells (no NEval/reduction involved in a 3NF
    // projection), so only that field needs swapping. Explicitly OUT of
    // this slice's acceptance ("the FULL Registry.compile minus the sql
    // projection") -- wired for completeness, not re-certified here.
    if matches!(jget(j, "sql_project"), Some(J::I(1))) {
        let saved_cells5 = srv.cells.clone();
        srv.cells = model_cells.clone();
        let projected = op_sql_project(&J::O(Vec::new()), srv);
        srv.cells = saved_cells5;
        r.push_str(",\"projected\":");
        r.push_str(&projected?);
    }
    r.push('}');
    Ok(r)
}

// ============================ the 3NF SQL projection ==========================
// Transplant #2 (docs/2026-07-10-old-engine-mining.md): the old engine's native
// RMAP + ONE projection plan — rmap_from_state -> Vec<TableDef> (rmap.rs:181),
// create_table_sql (:1455) as the single DDL source, and projection_plan
// (cli/entry.rs:115/376) whose Kahn parents-first order drove both the sql
// verb and persist — re-grown over the NEW store. The STRUCTURE is the old
// engine's (one table walk answers DDL + rows + order); the SEMANTICS is the
// python side verbatim: protocol.py ddl.generate/ddl.project are the source of
// truth for WHAT tables, columns, and rows exist, and byte-parity with
// drv.project over the same store is the acceptance. So the op reads the
// RESIDENT schema cells exactly as ddl._analyze reads D:
//   rmapColumns ⟨table, col, ft⟩ — the RMAP partition AS DATA (layout_cells
//     materialized it at compile; facts all the way down). The partition is
//     NOT re-derived here: a store without the cell reads as all-own-table,
//     which is layout_cells' own reading of such a store.
//   factType ⟨ft, reading⟩ — the partition's domain (== rmap_partition's keys;
//     checked against the identity store, diag 2026-07-10)
//   role ⟨id, ft, pos, player⟩ (pos int, 1-based), refScheme/refMode ⟨noun,
//     mode⟩ (refScheme wins, refMode fills), instanceOf ⟨noun, kind⟩ (kind
//     "ObjectType" names the entities), constraint ⟨id, kind, ft, player, ..⟩
//     (kind "mandatory" hardens columns — soft-stripped below, like project).
// NO sqlite here: the answer carries the CREATE TABLE statements (project's
// EXECUTED soft form — " TEXT NOT NULL" already relaxed to " TEXT", CREATE
// TABLE IF NOT EXISTS — so the strings equal what ddl.project feeds its
// connection byte for byte) plus the insertion rows; the consumer
// materializes them (the old sql verb's :memory: move, sql.rs:48).

fn ddl_sql_name(name: &str) -> String {
    // ddl._sql_name: non-alnum RUNS collapse to one "_", strip, lower ( ==
    // this host's sql_name/slug_str pair) PLUS the sqlite_ namespace guard
    // (the codex app's 'SQLite Fact Base' noun projected to sqlite_fact_base
    // and the CREATE refused — prefix our way out)
    let s = sql_name(name);
    if s.starts_with("sqlite_") {
        format!("t_{}", s)
    } else {
        s
    }
}

fn sql_q(name: &str) -> String {
    // ddl._q verbatim: every emitted identifier is quoted — the base metamodel
    // projects tables named constraint, transition, view (SQL reserved words)
    format!("\"{}\"", name)
}

struct ProjTable {
    key: String,          // the raw generate key (entity noun or fact type id)
    sql: String,          // sql_name(key), the materialized table's name
    create: String,       // ddl.project's executed CREATE (the soft form)
    cols: Vec<String>,    // final column names in DDL order
    rows: Vec<Vec<V>>,    // insertion rows (values as store leaves; ⊥ = NULL)
    parents: Vec<String>, // referenced tables' sql names (the Kahn edges)
}

fn op_sql_project(_j: &J, srv: &Srv) -> Result<String, String> {
    use std::collections::{HashMap, HashSet};
    // ---- the cell reads (ddl._analyze over the resident index) ----
    // one pass over the cached first-match-wins index; every later read is a
    // map hit (python's project memoizes pop() the same way)
    let mut pops: HashMap<String, Vec<V>> = HashMap::new();
    for (k, contents) in &srv.cells {
        if let Leaf::S(name) = k {
            let rows = match shape(contents) {
                Shape::Seq(l) => items(&l),
                _ => Vec::new(), // an atom cell has no population (_pop_rows)
            };
            pops.entry(name.clone()).or_insert(rows);
        }
    }
    let empty: Vec<V> = Vec::new();
    let pop = |name: &str| pops.get(name).unwrap_or(&empty);
    let row_items = |r: &V| items(&list_of(r));
    // schema NAMES are strings in the store; a non-string where a name belongs
    // drops the row (python would carry it into re.sub and crash — a store
    // that malformed never reaches project)
    let sstr = |v: &V| match aval(v) {
        Some(l) => match &*l {
            Leaf::S(s) => Some(s.clone()),
            _ => None,
        },
        None => None,
    };
    let ipos = |v: &V| {
        aval(v).and_then(|l| match &*l {
            Leaf::I(i) => Some(*i),
            Leaf::S(s) => s.parse::<i64>().ok(),
            _ => None,
        })
    };
    // sorted(ids, key=str): python's str() over the leaf; a structured id
    // (never minted by the engine) falls back to its set encoding
    let str_form = |v: &V| match shape(v) {
        Shape::Atom(l) => leaf_text(&l),
        _ => key_of(v),
    };

    // the partition FROM rmapColumns: absorbed ft -> table, per-table columns
    // in cell order (layout_cells wrote ⟨table, 2+j, ft⟩ in table_columns
    // order, so sorting by the col number reproduces it exactly)
    let mut absorbed: HashMap<String, String> = HashMap::new();
    let mut table_cols: HashMap<String, Vec<(i64, String)>> = HashMap::new();
    for r in pop("rmapColumns") {
        let it = row_items(r);
        if it.len() >= 3 {
            if let (Some(t), Some(c), Some(ft)) = (sstr(&it[0]), ipos(&it[1]), sstr(&it[2])) {
                absorbed.insert(ft.clone(), t.clone());
                table_cols.entry(t).or_default().push((c, ft));
            }
        }
    }
    for v in table_cols.values_mut() {
        v.sort();
    }
    // the domain: declared fact types (partition keys == factType rows); own
    // = the non-absorbed remainder, exactly python's {ft: key} where key == ft
    let mut declared: Vec<String> = Vec::new();
    let mut seen_ft: HashSet<String> = HashSet::new();
    for r in pop("factType") {
        let it = row_items(r);
        if let Some(ft) = it.first().and_then(&sstr) {
            if seen_ft.insert(ft.clone()) {
                declared.push(ft);
            }
        }
    }
    let mut own: Vec<String> = declared
        .iter()
        .filter(|ft| !absorbed.contains_key(*ft))
        .cloned()
        .collect();
    own.sort();
    let own_set: HashSet<String> = own.iter().cloned().collect();
    // roles: ft -> [(pos, player)] sorted (python tuple sort: pos then player),
    // with the ft FIRST-SEEN order kept — the entity-id sweep below walks
    // roles.items() in python's dict insertion order, and a store carrying two
    // ==-equal ids of different display (5 beside 5.0) keeps the first seen
    let mut roles: HashMap<String, Vec<(i64, String)>> = HashMap::new();
    let mut role_ft_order: Vec<String> = Vec::new();
    for r in pop("role") {
        let it = row_items(r);
        if it.len() >= 4 {
            if let (Some(ft), Some(p), Some(player)) = (sstr(&it[1]), ipos(&it[2]), sstr(&it[3])) {
                if !roles.contains_key(&ft) {
                    role_ft_order.push(ft.clone());
                }
                roles.entry(ft).or_default().push((p, player));
            }
        }
    }
    for v in roles.values_mut() {
        v.sort();
    }
    // reference modes: refScheme wins (dict build, last row wins), refMode
    // fills the gaps (setdefault, first row wins); absent -> "id"
    let mut refm: HashMap<String, String> = HashMap::new();
    for r in pop("refScheme") {
        let it = row_items(r);
        if it.len() >= 2 {
            if let (Some(n), Some(m)) = (sstr(&it[0]), sstr(&it[1])) {
                refm.insert(n, m);
            }
        }
    }
    for r in pop("refMode") {
        let it = row_items(r);
        if it.len() >= 2 {
            if let (Some(n), Some(m)) = (sstr(&it[0]), sstr(&it[1])) {
                refm.entry(n).or_insert(m);
            }
        }
    }
    let mut entities: HashSet<String> = HashSet::new();
    for r in pop("instanceOf") {
        let it = row_items(r);
        if it.len() >= 2 && matches!(sstr(&it[1]).as_deref(), Some("ObjectType")) {
            if let Some(n) = sstr(&it[0]) {
                entities.insert(n);
            }
        }
    }
    // NOTE ddl._analyze also reads the mandatory constraints (generate's
    // NOT NULL), but project executes the SOFT form — every " TEXT NOT NULL"
    // relaxed to " TEXT" (visibility over cascade on migrated populations) —
    // so the op, answering the EXECUTED statements, never consults them.
    // every declared entity gets a table, plus every absorbing table that is
    // not itself an own-table fact type (python: entities | (partition.values()
    // - set(own)))
    let mut entity_tables: HashSet<String> = entities.clone();
    for t in table_cols.keys() {
        if !own_set.contains(t) {
            entity_tables.insert(t.clone());
        }
    }
    let key_col = |name: &str| -> String {
        let m = refm.get(name).map(|s| s.as_str()).unwrap_or("id");
        format!("{}_{}", ddl_sql_name(name), ddl_sql_name(m))
    };
    // ddl._entity_columns: the ordered absorbed columns of an entity table,
    // (ft, col, kind 0=unary/1=value/2=ref, other), deduped by base with the
    // position suffix from 2 — one naming pass, so DDL and rows never disagree
    let entity_columns = |table: &str| -> Vec<(String, String, u8, Option<String>)> {
        let mut out = Vec::new();
        let mut seen: HashMap<String, usize> = HashMap::new();
        if let Some(fts) = table_cols.get(table) {
            for (_c, ft) in fts {
                let rs = roles.get(ft).map(|v| v.as_slice()).unwrap_or(&[]);
                let (base, kind, other): (String, u8, Option<String>) = if rs.len() == 1 {
                    let b = match ft.strip_prefix(table) {
                        Some(rest) => ddl_sql_name(rest),
                        None => ddl_sql_name(ft),
                    };
                    (b, 0, None)
                } else {
                    let other = rs.iter().map(|(_p, t)| t.clone()).find(|t| t != table);
                    match &other {
                        Some(o) if entities.contains(o) && entity_tables.contains(o) => {
                            (key_col(o), 2, other.clone())
                        }
                        Some(o) => (ddl_sql_name(o), 1, other.clone()),
                        None => (ddl_sql_name(ft), 1, None),
                    }
                };
                let n = seen.entry(base.clone()).or_insert(0);
                *n += 1;
                let col = if *n == 1 { base } else { format!("{}_{}", base, *n) };
                out.push((ft.clone(), col, kind, other));
            }
        }
        out
    };

    let mut out_tables: Vec<ProjTable> = Vec::new();
    // the drv.project report {table: rowcount}, in python's insertion order
    // (entity tables sorted, then own fact types sorted)
    let mut counts: Vec<(String, String)> = Vec::new();

    // ---- entity tables (ddl.generate + ddl.project, the absorbed branch) ----
    let mut sorted_entity: Vec<String> = entity_tables.iter().cloned().collect();
    sorted_entity.sort();
    for table in &sorted_entity {
        let ecols = entity_columns(table);
        let kc = key_col(table);
        let mut lines: Vec<String> = vec![format!("    {} TEXT PRIMARY KEY", sql_q(&kc))];
        let mut parents: Vec<String> = Vec::new();
        for (ft, col, kind, other) in &ecols {
            let _ = ft;
            if *kind == 0 {
                lines.push(format!("    {} BOOLEAN", sql_q(col))); // absorbed unary
                continue;
            }
            let refs = if *kind == 2 {
                let o = other.as_ref().expect("ref kind carries its player");
                let p = ddl_sql_name(o);
                if !parents.contains(&p) {
                    parents.push(p.clone());
                }
                format!(" REFERENCES {}({})", sql_q(&p), sql_q(&key_col(o)))
            } else {
                String::new()
            };
            lines.push(format!("    {} TEXT{}", sql_q(col), refs));
        }
        let create = format!(
            "CREATE TABLE IF NOT EXISTS {} (\n{}\n);",
            sql_q(&ddl_sql_name(table)),
            lines.join(",\n")
        );
        // the derived entity population: every id the entity's roles mention,
        // plus its own cell — set semantics over python == (set_key coalesces
        // 5 and 5.0, keeps "5" distinct), first-seen representative kept
        let mut ids: Vec<(String, V)> = Vec::new();
        let mut id_seen: HashSet<String> = HashSet::new();
        for ft in &role_ft_order {
            for (p, player) in &roles[ft] {
                if player != table {
                    continue;
                }
                for row in pop(ft) {
                    let it = row_items(row);
                    if *p >= 1 && it.len() >= *p as usize {
                        let v = it[*p as usize - 1].clone();
                        let k = key_of(&v);
                        if id_seen.insert(k.clone()) {
                            ids.push((k, v));
                        }
                    }
                }
            }
        }
        for row in pop(table) {
            let it = row_items(row);
            if !it.is_empty() {
                let v = it[0].clone();
                let k = key_of(&v);
                if id_seen.insert(k.clone()) {
                    ids.push((k, v));
                }
            }
        }
        ids.sort_by(|a, b| str_form(&a.1).cmp(&str_form(&b.1)).then_with(|| a.0.cmp(&b.0)));
        // per-column value views: unary membership set / functional last-wins map
        enum ColVals {
            Unary(HashSet<String>),
            Val(HashMap<String, V>),
        }
        let mut colvals: Vec<ColVals> = Vec::new();
        for (ft, _col, kind, _o) in &ecols {
            if *kind == 0 {
                let mut m = HashSet::new();
                for row in pop(ft) {
                    let it = row_items(row);
                    if !it.is_empty() {
                        m.insert(key_of(&it[0]));
                    }
                }
                colvals.push(ColVals::Unary(m));
            } else {
                let mut m: HashMap<String, V> = HashMap::new();
                for row in pop(ft) {
                    let it = row_items(row);
                    if it.len() >= 2 {
                        m.insert(key_of(&it[0]), it[1].clone()); // dict build: last wins
                    }
                }
                colvals.push(ColVals::Val(m));
            }
        }
        let mut rows: Vec<Vec<V>> = Vec::new();
        for (ik, iv) in &ids {
            let mut row = vec![iv.clone()];
            for cv in &colvals {
                row.push(match cv {
                    ColVals::Unary(m) => atom(Leaf::I(if m.contains(ik) { 1 } else { 0 })),
                    ColVals::Val(m) => m.get(ik).cloned().unwrap_or_else(bot), // ⊥ = NULL
                });
            }
            rows.push(row);
        }
        let mut cols: Vec<String> = vec![kc];
        cols.extend(ecols.iter().map(|(_ft, c, _k, _o)| c.clone()));
        counts.push((table.clone(), ids.len().to_string()));
        out_tables.push(ProjTable {
            key: table.clone(),
            sql: ddl_sql_name(table),
            create,
            cols,
            rows,
            parents,
        });
    }

    // ---- own-table fact types (spanning or no UC: row per fact) ----
    for ft in &own {
        let rs = roles.get(ft).map(|v| v.as_slice()).unwrap_or(&[]);
        if rs.is_empty() {
            // no roles, no relational shape: reported None, never malformed SQL
            counts.push((ft.clone(), "null".to_string()));
            continue;
        }
        let mut lines: Vec<String> = Vec::new();
        let mut key: Vec<String> = Vec::new();
        let mut seen: HashMap<String, usize> = HashMap::new();
        let mut parents: Vec<String> = Vec::new();
        for (_p, player) in rs {
            let base = if entities.contains(player) {
                key_col(player)
            } else {
                ddl_sql_name(player)
            };
            let n = seen.entry(base.clone()).or_insert(0);
            *n += 1;
            let col = if *n == 1 { base } else { format!("{}_{}", base, *n) };
            let refs = if entities.contains(player) && entity_tables.contains(player) {
                let p = ddl_sql_name(player);
                if !parents.contains(&p) {
                    parents.push(p.clone());
                }
                format!(" REFERENCES {}({})", sql_q(&p), sql_q(&key_col(player)))
            } else {
                String::new()
            };
            lines.push(format!("    {} TEXT{}", sql_q(&col), refs)); // NOT NULL soft-stripped
            key.push(col);
        }
        let create = format!(
            "CREATE TABLE IF NOT EXISTS {} (\n{},\n    PRIMARY KEY ({})\n);",
            sql_q(&ddl_sql_name(ft)),
            lines.join(",\n"),
            key.iter().map(|c| sql_q(c)).collect::<Vec<_>>().join(", ")
        );
        let all = pop(ft);
        let mut rows: Vec<Vec<V>> = Vec::new();
        let mut narrow = 0usize;
        for row in all {
            let it = row_items(row);
            if it.len() < rs.len() {
                narrow += 1; // a row narrower than its role count cannot bind
                continue;
            }
            rows.push(it[..rs.len()].to_vec());
        }
        let count = if all.is_empty() {
            "0".to_string()
        } else if narrow == 0 {
            all.len().to_string()
        } else {
            format!("{{\"projected\":{},\"narrow\":{}}}", all.len() - narrow, narrow)
        };
        counts.push((ft.clone(), count));
        out_tables.push(ProjTable {
            key: ft.clone(),
            sql: ddl_sql_name(ft),
            create,
            cols: key,
            rows,
            parents,
        });
    }

    // ---- Kahn parents-first (the old engine's Phase 3, rmap.rs:1676-1700):
    // ready = every referenced parent already placed, self-references pass,
    // externals pass; ready sorted by name; a cycle appends the rest sorted ----
    let names: HashSet<String> = out_tables.iter().map(|t| t.sql.clone()).collect();
    let mut placed: HashSet<String> = HashSet::new();
    let mut order: Vec<usize> = Vec::new();
    let mut remaining: Vec<usize> = (0..out_tables.len()).collect();
    while !remaining.is_empty() {
        let (mut ready, rest): (Vec<usize>, Vec<usize>) = remaining.iter().partition(|&&i| {
            out_tables[i]
                .parents
                .iter()
                .all(|p| p == &out_tables[i].sql || placed.contains(p) || !names.contains(p))
        });
        if ready.is_empty() {
            let mut rest_sorted = rest;
            rest_sorted.sort_by(|&a, &b| out_tables[a].sql.cmp(&out_tables[b].sql));
            order.extend(rest_sorted);
            break;
        }
        ready.sort_by(|&a, &b| out_tables[a].sql.cmp(&out_tables[b].sql));
        for &i in &ready {
            placed.insert(out_tables[i].sql.clone());
        }
        order.extend(ready);
        remaining = rest;
    }

    // ---- the answer: DDL + rows in Kahn order, plus the project report ----
    let mut r = String::from("{\"tables\":[");
    for (i, &ti) in order.iter().enumerate() {
        let t = &out_tables[ti];
        if i > 0 {
            r.push(',');
        }
        r.push_str("{\"name\":");
        esc(&t.sql, &mut r);
        r.push_str(",\"source\":");
        esc(&t.key, &mut r);
        r.push_str(",\"create_sql\":");
        esc(&t.create, &mut r);
        r.push_str(",\"columns\":[");
        for (k, c) in t.cols.iter().enumerate() {
            if k > 0 {
                r.push(',');
            }
            esc(c, &mut r);
        }
        r.push_str("],\"rows\":[");
        for (k, row) in t.rows.iter().enumerate() {
            if k > 0 {
                r.push(',');
            }
            r.push('[');
            for (m, v) in row.iter().enumerate() {
                if m > 0 {
                    r.push(',');
                }
                write_v(v, &mut r); // ⊥ prints null — exactly the NULL cell
            }
            r.push(']');
        }
        r.push_str("]}");
    }
    r.push_str("],\"counts\":{");
    for (i, (k, v)) in counts.iter().enumerate() {
        if i > 0 {
            r.push(',');
        }
        esc(k, &mut r);
        r.push(':');
        r.push_str(v);
    }
    r.push_str("}}");
    Ok(r)
}

fn op_ok(op: &str, result: &str) -> String {
    let mut s = String::from("{\"op\":");
    esc(op, &mut s);
    s.push_str(",\"result\":");
    s.push_str(result);
    s.push('}');
    s
}

fn op_err(op: &str, msg: &str) -> String {
    let mut s = String::from("{\"op\":");
    esc(op, &mut s);
    s.push_str(",\"error\":");
    esc(msg, &mut s);
    s.push('}');
    s
}

fn handle_op(op: &str, j: &J, srv: &mut Srv) -> String {
    match op_answer(op, j, srv) {
        Ok(r) => op_ok(op, &r),
        Err(m) => op_err(op, &m),
    }
}

// op_answer computes a verb's bare answer from the request object; the serve
// loop wraps it in the {"op", "result"|"error"} envelope (handle_op) and the
// MCP binding wraps it in the tools/call content envelope, so both surfaces
// serve the ONE resident op table. The store is mutable because run_rules
// REPLACES the retained store with its fixpoint; the read ops leave it alone.
fn op_answer(op: &str, j: &J, srv: &mut Srv) -> Result<String, String> {
    let fuel = match jget(j, "fuel") {
        Some(J::I(n)) if *n > 0 => Some(*n),
        _ => None,
    };
    match op {
        "verbs" => {
            let list = |xs: &[&str], out: &mut String| {
                out.push('[');
                for (i, x) in xs.iter().enumerate() {
                    if i > 0 { out.push(','); }
                    esc(x, out);
                }
                out.push(']');
            };
            // the flat list mirrors protocol.verbs(): sorted session + sorted app
            let mut all: Vec<&str> = Vec::new();
            all.extend_from_slice(&SESSION_VERBS);
            all.extend_from_slice(&APP_VERBS);
            let mut r = String::from("{\"verbs\":");
            list(&all, &mut r);
            r.push_str(",\"session\":");
            list(&SESSION_VERBS, &mut r);
            r.push_str(",\"app\":");
            list(&APP_VERBS, &mut r);
            r.push_str(",\"resident\":");
            list(&RESIDENT_OPS, &mut r);
            r.push('}');
            Ok(r)
        }
        "query" => {
            // FetchPop of the named cell over the resident store:
            // (ast:FetchPop : name) : D — an absent cell answers PHI, never ⊥
            let ft = match jget(j, "fact_type").and_then(scalar_atom) {
                Some(a) => a,
                None => return Err("query needs a scalar fact_type".to_string()),
            };
            let f = mkapp(atom(Leaf::S("ast:FetchPop".into())), ft.clone());
            let res = reduce_over(srv, f, srv.d.clone(), fuel);
            let mut r = String::from("{\"fact_type\":");
            write_v(&ft, &mut r);
            r.push_str(",\"rows\":");
            write_v(&res, &mut r);
            r.push('}');
            Ok(r)
        }
        "cells" => {
            // the store surface: addressable cell names (first match wins, as
            // FetchPop sees them) with row counts, optional substring pattern
            let pat = match jget(j, "pattern") {
                Some(J::S(p)) => Some(p.to_lowercase()),
                _ => None,
            };
            let mut entries: Vec<(String, String)> = Vec::new();
            for (k, contents) in &srv.cells {
                let name = match k {
                    Leaf::S(s) => s.clone(),
                    Leaf::I(i) => i.to_string(),
                    Leaf::F(f) => format!("{}", f),
                    Leaf::AppTag => continue,
                };
                if let Some(p) = &pat {
                    if !name.to_lowercase().contains(p.as_str()) {
                        continue;
                    }
                }
                let mut e = String::from("{\"name\":");
                match k {
                    Leaf::S(s) => esc(s, &mut e),
                    _ => e.push_str(&name),                   // numeric names stay numbers
                }
                e.push_str(",\"rows\":");
                match shape(contents) {
                    Shape::Seq(l) => e.push_str(&items(&l).len().to_string()),
                    _ => e.push_str("null"),                  // an atom cell has no rows
                }
                e.push('}');
                entries.push((name, e));
            }
            entries.sort();
            let mut r = String::from("{\"cells\":[");
            for (i, (_n, e)) in entries.iter().enumerate() {
                if i > 0 { r.push(','); }
                r.push_str(e);
            }
            r.push_str("]}");
            Ok(r)
        }
        "synthesize_pairs" => {
            // synthesize's engine half over the resident store: the canonical
            // (system:verbalize : id) : D — the entity's facts paired with
            // their fact types' reading templates; wording stays the caller's.
            // NATIVE BY DEFAULT (the ~40x plumb closed 2026-07-08):
            // verbalize rides the carrier exactly like the rules do — the
            // native view built FRESH from d (srv.nd may be stale,
            // op_run_rules' own idiom; trusting the mirror was the whole
            // "priced lever" mystery). Byte-parity pinned by the serve-op
            // suite; AREST_SYNTH_SCOTT=1 is the escape hatch to the old
            // Scott reduction.
            let id = match jget(j, "id").and_then(scalar_atom) {
                Some(a) => a,
                None => return Err("synthesize_pairs needs a scalar id".to_string()),
            };
            let res = if std::env::var_os("AREST_SYNTH_SCOTT").is_some() {
                let f = mkapp(atom(Leaf::S("system:verbalize".into())),
                              id.clone());
                reduce_over(srv, f, srv.d.clone(), fuel)
            } else {
                native_verbalize(srv, &id)
            };
            let mut r = String::from("{\"id\":");
            write_v(&id, &mut r);
            r.push_str(",\"pairs\":");
            write_v(&res, &mut r);
            r.push('}');
            Ok(r)
        }
        "run_rules" => {
            // the derivation fixpoint over the retained store (the
            // semi-naive positive-rule closure, with the optional "changed"
            // frontier bounding round one); the answer names the rounds and
            // the head cells that gained rows, and the retained store is
            // REPLACED by the derived result
            op_run_rules(j, srv)
        }
        "compile_model" => {
            // the Rust-native compile DRIVER (#20 skeleton): split → modality
            // → batch-classify through op_run_rules over the resident grammar
            // → dispatch the Classification_has_Translator table through the
            // reducer. The resident store is used as scratch and restored
            // whole; the answer reports {total, unclassified, prose} plus the
            // skeleton's missing/blocked diagnostics. The host "compile" verb
            // (the python delegation) is untouched — this op is the native
            // primary being grown beside it (the lex twin pattern).
            op_compile_model(j, srv)
        }
        "base_seed" => {
            // the native model-D seed (#20 seed slice): thaw base.store.json
            // when its embedded key matches, else recompute through the
            // already-certified compile_lines_native and persist
            // tmp-then-rename. Either way the resident ends up holding the
            // current base, ready for an app compile's own
            // context_from:"resident" (the identity/tasks --based flow).
            op_base_seed(j, srv)
        }
        "sql_project" => {
            // the RMAP 3NF projection over the resident store (transplant #2):
            // CREATE TABLE statements + insertion rows in Kahn parents-first
            // order, plus the {table: rowcount} report — python ddl.project's
            // answer computed natively, no sqlite; the consumer materializes
            op_sql_project(j, srv)
        }
        "neval" => {
            // DEBUG-ONLY (gated on AREST_NEVAL_TRACE): evaluate f : x over
            // the retained NATIVE carrier — the bisection probe the
            // silent-semantic gap requires (ledger 2026-07-08). Not in the
            // MCP tool table; the cases mechanism rides Scott, so this is
            // its native counterpart for A/B sub-term probes.
            if std::env::var_os("AREST_NEVAL_TRACE").is_none() {
                return Err("neval is a debug op: set AREST_NEVAL_TRACE".to_string());
            }
            let f = match jget(j, "f") {
                Some(v) => j_to_n(v),
                None => return Err("neval needs f".to_string()),
            };
            let x = match jget(j, "x") {
                Some(v) => j_to_n(v),
                None => return Err("neval needs x".to_string()),
            };
            // the retained mirror — coherent since the write-site audit
            // (2026-07-08); the probe now sees exactly what the native
            // reads see, which is the point of a bisection probe
            let ev = NEval {
                cells: srv.ncells.clone(),
                process: srv.nprocess.clone(),
                defs_n: srv.nd.clone(),
                fuel: std::cell::Cell::new(-1),
            };
            let res = n_to_v(&ev.mu(napp(f, x)));
            let mut r = String::from("{\"result\":");
            write_v(&res, &mut r);
            r.push('}');
            Ok(r)
        }
        _ => {
            if SESSION_VERBS.contains(&op) || APP_VERBS.contains(&op) {
                Err("verb needs the apps registry (host-side); resident ops: \
                     cells, query, run_rules, synthesize_pairs, verbs".to_string())
            } else {
                Err("unknown op; resident ops: cells, query, run_rules, \
                     synthesize_pairs, verbs".to_string())
            }
        }
    }
}


// ============================ the MCP binding =================================
// The daily-driver surface rides the Model Context Protocol's stdio transport
// under --mcp --apps-dir <path>: newline-delimited JSON-RPC 2.0, one object
// per line each way, mirroring python/protocol.py's mcp_server. This binding
// adds the host side the serve loop lacks. An apps registry scans the apps
// directory, where every app subdirectory <name>/ carries <name>.store.json,
// one serve-protocol set_store payload persisted at each snapshot site
// (Registry._sidecar writes it; tests/test_store_sidecar.py pins the
// contract). apps_use boots an app by feeding that file through handle, the
// SAME ingestion path a --serve stdin line takes, so no ingestion logic lives
// here. The native read verbs route through op_answer over the retained
// store. The write verbs, apps_compile, and the read long tail (get, schema,
// sql, explain, validate, verify, actions) need the compiler host, so the
// binding spawns the repository's one-shot CLI (cli.py at the repository
// root, found by walking up from the running executable, with --py-cli
// naming it directly and --python naming the interpreter) and answers the
// one JSON value the CLI prints; after a write, the app's sidecar re-ingests
// through the same path apps_use takes, so the retained store stays the
// sidecar's equal, while the delegated reads change nothing and reload
// nothing.

// The tool table is static JSON; the descriptor shapes and argument names
// mirror python/protocol.py's TOOLS for the verbs this binding serves.
const MCP_TOOLS: &str = concat!(
    r#"[{"name":"orient","#,
    r#""description":"Answers the apps inventory and the active app in one envelope.","#,
    r#""inputSchema":{"type":"object","properties":{}}},"#,
    r#"{"name":"apps_list","#,
    r#""description":"Answers every app under the apps directory; an app is a subdirectory carrying its <name>.store.json sidecar.","#,
    r#""inputSchema":{"type":"object","properties":{}}},"#,
    r#"{"name":"apps_current","#,
    r#""description":"Answers the retained app name, or null before apps_use.","#,
    r#""inputSchema":{"type":"object","properties":{}}},"#,
    r#"{"name":"apps_use","#,
    r#""description":"Loads an app's store sidecar through the serve ingestion path and retains it as the resident store.","#,
    r#""inputSchema":{"type":"object","properties":{"name":{"type":"string"}},"required":["name"]}},"#,
    r#"{"name":"query","#,
    r#""description":"Answers a fact type's population from the retained store.","#,
    r#""inputSchema":{"type":"object","properties":{"fact_type":{"type":"string"}},"required":["fact_type"]}},"#,
    r#"{"name":"cells","#,
    r#""description":"Answers cell names with row counts over the retained store; pattern filters by substring.","#,
    r#""inputSchema":{"type":"object","properties":{"pattern":{"type":"string"}}}},"#,
    r#"{"name":"synthesize","#,
    r#""description":"Answers an entity's facts paired with their fact types' reading templates; the caller words the prose.","#,
    r#""inputSchema":{"type":"object","properties":{"id":{"type":"string"}},"required":["id"]}},"#,
    r#"{"name":"derive","#,
    r#""description":"Runs the derivation rules over the retained store to the least fixed point and answers the rounds and the changed head cells; changed optionally bounds round one to the rules reading those cells.","#,
    r#""inputSchema":{"type":"object","properties":{"changed":{"type":"array","items":{"type":"string"}}}}},"#,
    r#"{"name":"apply","#,
    r#""description":"Commits one fact row (eq. create) and answers its receipt, NATIVELY in the resident for own-table and absorbed fact types alike (the stored create:<ft> handler computes its cell from the fact; the machine leg advances the status column; the bounded derive and the sidecar ride the same step); a refused write answers committed false with the violations.","#,
    r#""inputSchema":{"type":"object","properties":{"app":{"type":"string"},"fact_type":{"type":"string"},"fact":{"type":"array","items":{"type":"string"}}},"required":["app","fact_type","fact"]}},"#,
    r#"{"name":"retract","#,
    r#""description":"Delegates one fact-row retraction to the Python compiler host and answers its receipt; a refused retraction answers committed false with the violations.","#,
    r#""inputSchema":{"type":"object","properties":{"app":{"type":"string"},"fact_type":{"type":"string"},"fact":{"type":"array","items":{"type":"string"}}},"required":["app","fact_type","fact"]}},"#,
    r#"{"name":"apps_compile","#,
    r#""description":"Delegates a readings compile to the Python compiler host, rebuilding the app's database and store sidecar, and answers the compile report.","#,
    r#""inputSchema":{"type":"object","properties":{"app":{"type":"string"}},"required":["app"]}},"#,
    r#"{"name":"get","#,
    r#""description":"Answers the per-entity view of the retained app through the Python compiler host: the key, the absorbed values, and the facts the id participates in.","#,
    r#""inputSchema":{"type":"object","properties":{"noun":{"type":"string"},"id":{"type":"string"}},"required":["noun","id"]}},"#,
    r#"{"name":"schema","#,
    r#""description":"Answers the retained app's model surface through the Python compiler host: object types, fact types with readings and roles, and constraints.","#,
    r#""inputSchema":{"type":"object","properties":{}}},"#,
    r#"{"name":"sql","#,
    r#""description":"Runs read-only SQL over the retained app's snapshot database through the Python compiler host and answers the rows.","#,
    r#""inputSchema":{"type":"object","properties":{"statement":{"type":"string"}},"required":["statement"]}},"#,
    r#"{"name":"explain","#,
    r#""description":"Answers an id's provenance in the retained app through the Python compiler host: the derivation chains and the audit trail of its facts.","#,
    r#""inputSchema":{"type":"object","properties":{"id":{"type":"string"}},"required":["id"]}},"#,
    r#"{"name":"validate","#,
    r#""description":"Answers the retained app's alethic violations through the Python compiler host; an empty list means the population satisfies the schema.","#,
    r#""inputSchema":{"type":"object","properties":{}}},"#,
    r#"{"name":"verify","#,
    r#""description":"Runs the retained app's verification checks through the Python compiler host and answers them.","#,
    r#""inputSchema":{"type":"object","properties":{}}},"#,
    r#"{"name":"actions","#,
    r#""description":"Answers a noun id's state machine and available actions in the retained app through the Python compiler host.","#,
    r#""inputSchema":{"type":"object","properties":{"noun":{"type":"string"},"id":{"type":"string"}},"required":["noun","id"]}},"#,
    r#"{"name":"apps_status","#,
    r#""description":"One app's posture without activating it: exists, readings count, compiled, stale. Filesystem-derived, native in the resident.","#,
    r#""inputSchema":{"type":"object","properties":{"name":{"type":"string"}},"required":["name"]}},"#,
    r#"{"name":"apps_check","#,
    r#""description":"Sweep EVERY app: per-app health (ready / stale / library / not_found) plus the rolled-up summary. Native in the resident.","#,
    r#""inputSchema":{"type":"object","properties":{"include_ready":{"type":"boolean"}}}},"#,
    r#"{"name":"apps_register","#,
    r#""description":"Registration is directory-derived: re-scan the apps directory and answer the roster; nothing is written.","#,
    r#""inputSchema":{"type":"object","properties":{}}},"#,
    r#"{"name":"apps_create","#,
    r#""description":"A new app skeleton: <name>/readings/core.md. Refuses on an existing app. Native in the resident.","#,
    r#""inputSchema":{"type":"object","properties":{"name":{"type":"string"},"text":{"type":"string"}},"required":["name"]}},"#,
    r#"{"name":"engine_version","#,
    r#""description":"The engine and its version.","#,
    r#""inputSchema":{"type":"object","properties":{}}},"#,
    r#"{"name":"compile","#,
    r#""description":"The live ADDITIVE compile: the text joins the app's readings/ (the source of truth, so a rebuild keeps it) and the app recompiles. Rides the compiler host; the retained sidecar reloads.","#,
    r#""inputSchema":{"type":"object","properties":{"app":{"type":"string"},"text":{"type":"string"}},"required":["text"]}},"#,
    r#"{"name":"propose","#,
    r#""description":"The authoring dry-run: compile the candidate readings ATOP the app's model on a throwaway store - would-be declarations, classification, diagnostics - persisting nothing.","#,
    r#""inputSchema":{"type":"object","properties":{"app":{"type":"string"},"text":{"type":"string"}},"required":["text"]}},"#,
    r#"{"name":"induce","#,
    r#""description":"Hypothesis-Candidate search over a fact type - the abduction primitive. Enumerate bindings for the hidden fact, gate through alethic constraints (baseline delta) and forward-chain coverage of to_explain, score by the app's Scoring Rules, answer ranked candidates. Nothing persists.","#,
    r#""inputSchema":{"type":"object","properties":{"app":{"type":"string"},"ft_id":{"type":"string"},"to_explain":{"type":"array"},"bound":{"type":"object"}},"required":["ft_id"]}},"#,
    r#"{"name":"context","#,
    r#""description":"The resident's last mutation receipt, or a note when none this session.","#,
    r#""inputSchema":{"type":"object","properties":{}}},"#,
    r#"{"name":"ask","#,
    r#""description":"Read-only Q&A, no LLM in the engine: pass a plan {fact_type, filter} to execute the projection query; without one the verb answers needs_plan + the model surface for the CALLER's sampler to complete.","#,
    r#""inputSchema":{"type":"object","properties":{"app":{"type":"string"},"question":{"type":"string"},"plan":{"type":"object"}},"required":["question"]}},"#,
    r#"{"name":"select_component","#,
    r#""description":"Select a UI Component by intent and constraints from the Component registry app (binding doctrine: the registry is facts; toolkit implementations register in DEFS). Answers ranked {component, role, toolkit, symbol, score} records.","#,
    r#""inputSchema":{"type":"object","properties":{"intent":{"type":"string"},"traits":{"type":"array","items":{"type":"string"}},"toolkit":{"type":"string"},"limit":{"type":"number"},"app":{"type":"string"}},"required":["intent"]}},"#,
    r#"{"name":"tutor_list","#,
    r#""description":"List the tutor lessons (tracks easy/medium/hard) with titles and goals; the tutor rides the _tutor sandbox app.","#,
    r#""inputSchema":{"type":"object","properties":{}}},"#,
    r#"{"name":"tutor_get","#,
    r#""description":"One lesson, parsed: title, goal, the runnable fences (each is one first-class verb call), and the expect predicate.","#,
    r#""inputSchema":{"type":"object","properties":{"lesson":{"type":"string"}},"required":["lesson"]}},"#,
    r#"{"name":"tutor_check","#,
    r#""description":"Evaluate the lesson's expect predicate against the sandbox app; passed flips when the learner's work satisfies it.","#,
    r#""inputSchema":{"type":"object","properties":{"lesson":{"type":"string"}},"required":["lesson"]}},"#,
    r#"{"name":"tutor_reset","#,
    r#""description":"Rebootstrap the sandbox: wipe learner state, copy tutor/domains readings into the _tutor app, recompile.","#,
    r#""inputSchema":{"type":"object","properties":{}}},"#,
    r#"{"name":"tutor_apply","#,
    r#""description":"apply, scoped to the tutor sandbox app.","#,
    r#""inputSchema":{"type":"object","properties":{"fact_type":{"type":"string"},"fact":{"type":"array","items":{}}},"required":["fact_type","fact"]}},"#,
    r#"{"name":"tutor_query","#,
    r#""description":"query, scoped to the tutor sandbox app.","#,
    r#""inputSchema":{"type":"object","properties":{"fact_type":{"type":"string"}},"required":["fact_type"]}},"#,
    r#"{"name":"tutor_compile","#,
    r#""description":"compile readings text into the tutor sandbox app.","#,
    r#""inputSchema":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"]}},"#,
    r#"{"name":"tutor_propose","#,
    r#""description":"propose, scoped to the tutor sandbox app.","#,
    r#""inputSchema":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"]}},"#,
    r#"{"name":"tutor_actions","#,
    r#""description":"actions (HATEOAS transitions), scoped to the tutor sandbox app.","#,
    r#""inputSchema":{"type":"object","properties":{"noun":{"type":"string"},"id":{"type":"string"}},"required":["noun","id"]}},"#,
    r#"{"name":"tutor_authoring","#,
    r#""description":"The authoring workflow joined from the sandbox's Authoring Step facts: ordered steps with situation, guidance, status, and recommended tools.","#,
    r#""inputSchema":{"type":"object","properties":{"status":{"type":"string"}}}}]"#
);

#[cfg(feature = "host")]
struct Apps {
    dir: std::path::PathBuf,
    current: Option<String>,
    // The CLI delegate: the interpreter and the one-shot script the binding
    // spawns for the write verbs, apps_compile, and the read long tail. The
    // script path comes from the startup walk-up unless --py-cli names it;
    // None means the walk missed, and only the delegated verbs mind.
    python: String,
    cli: Option<std::path::PathBuf>,
}

#[cfg(feature = "host")]
impl Apps {
    fn sidecar(&self, name: &str) -> std::path::PathBuf {
        self.dir.join(name).join(format!("{}.store.json", name))
    }
    fn list(&self) -> Vec<String> {
        // An app is any subdirectory whose sidecar exists; nothing else counts.
        let mut names: Vec<String> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.dir) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if self.sidecar(&name).is_file() {
                    names.push(name);
                }
            }
        }
        names.sort();
        names
    }
}

#[cfg(feature = "host")]
fn esc_names(names: &[String], out: &mut String) {
    out.push('[');
    for (i, n) in names.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        esc(n, out);
    }
    out.push(']');
}

fn parse_json(text: &str) -> Option<J> {
    // P trusts protocol lines and panics on malformed bytes; the MCP transport
    // reads the wild, so a failed parse unwinds here and answers None instead
    // of killing the loop.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        P { b: text.as_bytes(), i: 0 }.parse()
    }))
    .ok()
}

// find_cli walks up from the running executable toward the filesystem root
// and answers the first ancestor directory's cli.py; the exe lives at
// <root>/rust/target/<profile>/arest.exe, so the repository root is the
// first hit. The --py-cli flag overrides the walk entirely.
#[cfg(feature = "host")]
fn find_cli() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    for dir in exe.ancestors().skip(1) {
        let cand = dir.join("cli.py");
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

// tail_text answers the last few hundred characters of a child's stderr,
// enough to name a failure inside one protocol line without flooding it.
#[cfg(feature = "host")]
fn tail_text(s: &str) -> String {
    let t = s.trim();
    let n = t.chars().count();
    if n <= 300 {
        t.to_string()
    } else {
        t.chars().skip(n - 300).collect()
    }
}

// load_sidecar feeds an app's persisted store through handle, the SAME
// ingestion path a --serve stdin line takes; apps_use rides it to boot an
// app, and the delegated write verbs ride it to reload one. It never touches
// the current-app marker, so a reload cannot switch apps.
#[cfg(feature = "host")]
fn load_sidecar(apps: &Apps, name: &str, srv: &mut Srv) -> Result<(), String> {
    let path = apps.sidecar(name);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => {
            return Err(format!("no app {:?} under {} (no store sidecar)",
                               name, apps.dir.display()))
        }
    };
    let payload = match parse_json(&text) {
        Some(p) if jget(&p, "d").is_some() => p,
        _ => return Err(format!("unparseable store sidecar for {:?}", name)),
    };
    handle(&payload, srv, true);
    Ok(())
}

// run_cli spawns the repository's one-shot Python CLI:
//   <python> <cli.py> <verb> --apps-dir <dir> <app> [tail...]
// and answers the exit code beside the ONE JSON value the CLI prints on
// stdout. An exit code outside ok answers -32603 carrying the tail of
// stderr, as do a spawn failure, a missing cli.py, and unparseable stdout.
// The parse both proves an answer arrived and lets the caller re-serialize
// it compactly through write_j.
#[cfg(feature = "host")]
fn run_cli(apps: &Apps, verb: &str, app: &str, tail: &[String], ok: &[i32])
    -> Result<(i32, J), (i64, String)>
{
    let cli = match &apps.cli {
        Some(p) => p.clone(),
        None => {
            return Err((-32603,
                "no cli.py found walking up from the executable; pass --py-cli <path>"
                    .to_string()))
        }
    };
    let mut cmd = std::process::Command::new(&apps.python);
    cmd.arg(&cli).arg(verb).arg("--apps-dir").arg(&apps.dir).arg(app);
    for t in tail {
        cmd.arg(t);
    }
    let out = match cmd.output() {
        Ok(o) => o,
        Err(e) => {
            return Err((-32603,
                format!("could not spawn {} for {}: {}", apps.python, verb, e)))
        }
    };
    let code = out.status.code().unwrap_or(-1);
    let err_tail = tail_text(&String::from_utf8_lossy(&out.stderr));
    if !ok.contains(&code) {
        let mut msg = format!("cli.py {} exited {}", verb, code);
        if !err_tail.is_empty() {
            msg.push_str(": ");
            msg.push_str(&err_tail);
        }
        return Err((-32603, msg));
    }
    let stdout_text = String::from_utf8_lossy(&out.stdout);
    match parse_json(stdout_text.trim()) {
        Some(v) => Ok((code, v)),
        None => {
            let mut msg = format!("cli.py {} answered no parseable receipt", verb);
            if !err_tail.is_empty() {
                msg.push_str(": ");
                msg.push_str(&err_tail);
            }
            Err((-32603, msg))
        }
    }
}

// The write verbs and apps_compile delegate through run_cli. The CLI exits 0
// on a committed write or a clean compile and 1 on a refusal; both answer
// the receipt as the tool result, because a refusal is an answer the caller
// reads, not a protocol failure. A write receipt is always an object, so a
// value of any other shape is a contract break and answers -32603.
// One app's posture from the filesystem alone (the registry is
// directory-derived): exists, readings count, compiled (<name>.db present),
// stale (any reading newer than the .db; an uncompiled app with readings is
// stale by definition). Mirrors protocol.Registry.status key for key.
#[cfg(feature = "host")]
fn app_status_json(apps: &Apps, name: &str) -> String {
    let d = std::path::Path::new(&apps.dir).join(name);
    let rd = d.join("readings");
    let mut readings = 0usize;
    let mut newest: Option<std::time::SystemTime> = None;
    if let Ok(entries) = std::fs::read_dir(&rd) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("md") {
                readings += 1;
                if let Ok(md) = e.metadata() {
                    if let Ok(t) = md.modified() {
                        newest = Some(match newest {
                            Some(n) if n > t => n,
                            _ => t,
                        });
                    }
                }
            }
        }
    }
    let db = d.join(format!("{}.db", name));
    let compiled = db.exists();
    let db_m = std::fs::metadata(&db).ok().and_then(|m| m.modified().ok());
    let stale = if compiled {
        matches!((newest, db_m), (Some(n), Some(dm)) if n > dm)
    } else {
        readings > 0
    };
    let mut r = String::from("{\"name\":");
    esc(name, &mut r);
    r.push_str(&format!(
        ",\"exists\":{},\"readings\":{},\"compiled\":{},\"stale\":{}}}",
        d.is_dir(), readings, compiled, stale));
    r
}

// The generic delegation: any first-class verb the resident does not compute
// natively rides cli.py's `call` form — ONE dispatch table on the Python side
// (protocol.SESSION_VERBS / APP_VERBS), never a second verb registry here.
// The retained app is injected when the caller names none, so the one-shot
// process needs no marker state; `reload` re-ingests the retained sidecar
// after a mutating verb (the live additive compile).
#[cfg(feature = "host")]
fn delegate_call(verb: &str, args: &J, apps: &Apps, srv: &mut Srv, reload: bool)
    -> Result<String, (i64, String)>
{
    let mut obj = match args {
        J::O(kv) => kv.clone(),
        _ => Vec::new(),
    };
    if !obj.iter().any(|(k, _)| k == "app") {
        if let Some(cur) = &apps.current {
            obj.push(("app".to_string(), J::S(cur.clone())));
        }
    }
    let mut body = String::new();
    write_j(&J::O(obj), &mut body);
    let (_code, receipt) = run_cli(apps, "call", verb, &[body], &[0])?;
    if reload {
        if let Some(cur) = apps.current.clone() {
            let _ = load_sidecar(apps, &cur, srv);
        }
    }
    let mut r = String::new();
    write_j(&receipt, &mut r);
    Ok(r)
}

#[cfg(feature = "host")]
fn delegate_verb(tool: &str, args: &J, apps: &Apps, srv: &mut Srv)
    -> Result<String, (i64, String)>
{
    let app = match jget(args, "app") {
        Some(J::S(a)) => a.clone(),
        _ => return Err((-32602, format!("{} needs a string app", tool))),
    };
    // The registry-facing tool name carries the apps_ prefix; the CLI names
    // the verb bare.
    let verb = if tool == "apps_compile" { "compile" } else { tool };
    let mut tail: Vec<String> = Vec::new();
    if verb != "compile" {
        match jget(args, "fact_type") {
            Some(J::S(f)) => tail.push(f.clone()),
            _ => return Err((-32602, format!("{} needs a string fact_type", tool))),
        }
        match jget(args, "fact") {
            Some(a @ J::A(_)) => {
                let mut s = String::new();
                write_j(a, &mut s);
                tail.push(s);
            }
            _ => return Err((-32602, format!("{} needs an array fact", tool))),
        }
    }
    let (code, receipt) = run_cli(apps, verb, &app, &tail, &[0, 1])?;
    if !matches!(receipt, J::O(_)) {
        return Err((-32603, format!("cli.py {} answered no object receipt", verb)));
    }
    // A committed verb rewrote the sidecar; re-ingesting keeps the retained
    // store its equal. A refused apply or retract reloads the unchanged file
    // for the same consistency, a compile reloads only on success, and no
    // reload runs unless the delegated app IS the retained one, so a write
    // to another app never switches or loads anything. A reload miss is
    // skipped rather than masking the receipt the caller must read.
    if (code == 0 || verb != "compile") && apps.current.as_deref() == Some(app.as_str()) {
        let _ = load_sidecar(apps, &app, srv);
    }
    let mut r = String::new();
    write_j(&receipt, &mut r);
    Ok(r)
}

// The read long tail rides the same delegation: get, schema, sql, explain,
// validate, verify, and actions need the compiler host's Registry but write
// nothing, so no sidecar reload follows. Each one scopes to the RETAINED
// app, so the caller names no app. A read never refuses, so only exit 0
// passes, and the answer is whatever one JSON value the CLI prints; sql
// answers an array of arrays, so no object envelope is assumed. synthesize
// stays native over the retained store and never routes here.
#[cfg(feature = "host")]
fn delegate_read(tool: &str, args: &J, apps: &Apps) -> Result<String, (i64, String)> {
    let app = match &apps.current {
        Some(n) => n.clone(),
        None => {
            return Err((-32602, format!("no app loaded; call apps_use before {}", tool)))
        }
    };
    let keys: &[&str] = match tool {
        "get" | "actions" => &["noun", "id"],
        "sql" => &["statement"],
        "explain" | "synthesize" => &["id"],
        _ => &[],
    };
    let mut tail: Vec<String> = Vec::new();
    for key in keys {
        match jget(args, key) {
            Some(J::S(v)) => tail.push(v.clone()),
            _ => return Err((-32602, format!("{} needs a string {}", tool, key))),
        }
    }
    let (_code, value) = run_cli(apps, tool, &app, &tail, &[0])?;
    let mut r = String::new();
    write_j(&value, &mut r);
    Ok(r)
}

// append_event writes one committed step to the app's event log in
// FileEventSink's format: a single line {"ft": <fact type>, "fact": [<row>..]}
// at <app_dir>/<app>.events.jsonl, byte for byte the json.dumps(entry) + "\n"
// the Python sink appends (the default separators carry the ", " and ": "
// spaces). The log is the durable source of truth a recompile replays through
// the same create, so a native write lands in the same stream the delegated
// write does, and a mixed history replays clean.
#[cfg(feature = "host")]
fn append_event(apps: &Apps, app: &str, ft: &str, fact: &[J]) -> std::io::Result<()> {
    use std::io::Write;
    let path = apps.dir.join(app).join(format!("{}.events.jsonl", app));
    let mut line = String::from("{\"ft\": ");
    esc(ft, &mut line);
    line.push_str(", \"fact\": [");
    for (i, e) in fact.iter().enumerate() {
        if i > 0 {
            line.push_str(", ");
        }
        write_j(e, &mut line);
    }
    line.push_str("]}\n");
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
    f.write_all(line.as_bytes())
}

// write_sidecar refreshes the app's store sidecar with the retained store,
// Registry._sidecar's payload: {"d": <store>, "process": [[name, obj]..],
// "overrides": 1, "cases": []}. The d field is the whole retained store; the
// process field is the resident's compiled defs, unchanged across a write, so
// it re-serializes what apps_use loaded (write_n is j_to_n's inverse, so a
// fresh boot reconstructs the same process table). The write is
// tmp-then-rename, so a concurrent reader never sees a torn file, exactly as
// _sidecar's os.replace. A fresh resident boots this file through the same
// ingestion apps_use takes, so the native write is durable to the resident.
#[cfg(feature = "host")]
// write_v_spaced / write_n_spaced / sidecar_payload (#20, the final pipeline
// slice): protocol.py's Registry._sidecar writes the sidecar through plain
// `json.dump(payload, f, ensure_ascii=False)` -- NO `separators=` override --
// so python's OWN default applies: item/key separators (", ", ": ") when
// indent is None (verified empirically: json.dumps({"a":[1,2]}) ==
// '{"a": [1, 2]}', confirmed independently against the checked-in
// tests/fixtures/apps/flow/flow.store.json, which is spaced throughout).
// This is DIFFERENT from write_v/write_n, the COMPACT serializers every
// differential dump (dump_store's "store" field, rp-pydump.py's hand-mirror
// write_v) has used since the first slice -- that pairing is a DELIBERATE,
// mutually-agreed TEST convention on both hosts, unrelated to this
// PRODUCTION file format. write_sidecar (below) previously used the compact
// write_v/write_n here too -- a real, silent divergence from python's actual
// on-disk bytes that this slice's own sidecar byte-compare acceptance
// caught (see the task report): every native write (native_apply's own
// already-shipped path, unchanged in behavior otherwise) was quietly
// rewriting a python-produced spaced sidecar into a compact one. Fixed once,
// here, for every writer.
fn write_v_spaced(v: &V, out: &mut String) {
    match shape(v) {
        Shape::Bot => out.push_str("null"),
        Shape::Atom(l) => match &*l {
            Leaf::S(s) => esc(s, out),
            Leaf::I(i) => out.push_str(&i.to_string()),
            Leaf::F(f) => {
                if f.fract() == 0.0 && f.is_finite() {
                    out.push_str(&format!("{:.1}", f));
                } else {
                    out.push_str(&format!("{}", f));
                }
            }
            Leaf::AppTag => out.push_str("\"#APP#\""),
        },
        Shape::Seq(l) => {
            out.push('[');
            let xs = items(&l);
            for (i, x) in xs.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_v_spaced(x, out);
            }
            out.push(']');
        }
    }
}

fn write_n_spaced(n: &N, out: &mut String) {
    match n {
        N::Bot => out.push_str("null"),
        N::A(l) => match &**l {
            Leaf::S(s) => esc(s, out),
            Leaf::I(i) => out.push_str(&i.to_string()),
            Leaf::F(f) => {
                if f.fract() == 0.0 && f.is_finite() {
                    out.push_str(&format!("{:.1}", f));
                } else {
                    out.push_str(&format!("{}", f));
                }
            }
            Leaf::AppTag => out.push_str("\"#APP#\""),
        },
        N::S(v) => {
            out.push('[');
            for (i, x) in v.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_n_spaced(x, out);
            }
            out.push(']');
        }
    }
}

// sidecar_payload is the ONE core the resident's every sidecar writer
// shares: native_apply's live-write path (write_sidecar, below) and
// op_compile_model's new "save" surface (#20) both serialize a <d, process>
// pair through it, so the two writers can never quietly disagree on format.
fn sidecar_payload(d: &V, process: &[(String, N)]) -> String {
    let mut payload = String::from("{\"d\": ");
    write_v_spaced(d, &mut payload);
    payload.push_str(", \"process\": [");
    for (i, (name, obj)) in process.iter().enumerate() {
        if i > 0 {
            payload.push_str(", ");
        }
        payload.push('[');
        esc(name, &mut payload);
        payload.push_str(", ");
        write_n_spaced(obj, &mut payload);
        payload.push(']');
    }
    payload.push_str("], \"overrides\": 1, \"cases\": []}");
    payload
}

fn write_sidecar(apps: &Apps, app: &str, srv: &Srv) -> std::io::Result<()> {
    let payload = sidecar_payload(&srv.d, &srv.nprocess);
    let path = apps.sidecar(app);
    let mut tmp = path.clone().into_os_string();
    // the tmp name carries the pid, matching _sidecar: the Python CLI writes
    // this sidecar too, and two writers sharing one tmp path tear the file
    tmp.push(format!(".{}.tmp", std::process::id()));
    let tmp = std::path::PathBuf::from(tmp);
    std::fs::write(&tmp, payload.as_bytes())?;
    std::fs::rename(&tmp, &path)
}

// native_apply is the resident's own write path for an OWN-TABLE fact type: the
// fact type carries a create:<ft> handler cell (engine.py create_handlers
// stores one per own-table fact type; an absorbed fact type has none, phase
// two), so the resident computes the create in process instead of delegating
// to the Python CLI. It reduces that handler over the pair of the fact and the
// retained store through the resident's own evaluator — the same
// apply(handler, ⟨fact, D⟩) under D's DEFS that engine.py's ast.run reduces —
// extracts the receipt exactly as protocol.py Registry.apply does (committed
// iff the representation is not the ERROR atom and D' differs from D, the
// violation set read from the representation's second part), and on a commit
// retains D', runs the native derive bounded to the written fact type, and
// persists natively (the event log line and the refreshed store sidecar). It
// answers None to fall back to CLI delegation when the fact type is absorbed
// (no handler cell), when the target app is not the retained one, or when the
// reduction does not yield a proper ⟨representation, D'⟩ pair — the bare-ERROR
// refusal, where Python re-runs validate for the offenders, stays the
// delegate's job.
// The HOSTLESS write core: the create:<ft> handler evaluation, the
// bounded derive, and the receipt — persistence is the CALLER'S
// (native_apply appends the fs event log + sidecar; the wasm Worker
// returns the committed event for the Durable Object to append — the
// same single-writer stream Def. iso demands, two storages). None =
// this write needs the compiler host (no handler / absorbed shapes
// the delegate serves).
fn apply_core(args: &J, app: &str, srv: &mut Srv)
    -> Option<Result<(String, Option<(String, Vec<J>)>), (i64, String)>> {
    let app = app.to_string();
    let ft = match jget(args, "fact_type") {
        Some(J::S(f)) => f.clone(),
        _ => return None,
    };
    let fact = match jget(args, "fact") {
        Some(J::A(xs)) => xs.clone(),
        _ => return None,
    };
    // THE ID-SENTINEL GUARD (the phi phantom, 2026-07-08): a key
    // position carrying the phi atom or the empty string is a leak,
    // never modeling intent — refuse before evaluation, mirroring
    // Registry.apply's receipt (replay ungated; retract stays open)
    if let Some(J::S(k)) = fact.first() {
        if k.is_empty() || k == "\u{03c6}" {
            let mut r = String::from("{\"app\":");
            esc(&app, &mut r);
            r.push_str(",\"fact_type\":");
            esc(&ft, &mut r);
            r.push_str(",\"fact\":[");
            for (i, f) in fact.iter().enumerate() {
                if i > 0 { r.push(','); }
                match f {
                    J::S(s) => esc(s, &mut r),
                    J::I(n) => r.push_str(&n.to_string()),
                    _ => r.push_str("null"),
                }
            }
            r.push_str("],\"committed\":false,\"violations\":[[\"id-sentinel\",\"a key must not be empty or the phi atom\"]]}");
            return Some(Ok((r, None)));
        }
    }
    // a create:<ft> handler cell serves BOTH shapes now (phase two done):
    // an own-table handler carries its fixed cell name; an absorbed handler
    // computes cellkey(table, key) from the fact at reduce time. Absence
    // (a stale pre-0.9.0 sidecar) delegates
    let cell_name = Leaf::S(format!("create:{}", ft));
    let handler = match srv.cells.iter().find(|(k, _)| k.nateq(&cell_name)) {
        Some((_, c)) => c.clone(),
        None => return None,
    };
    // the operand ⟨fact_as_v, D⟩: the fact a sequence of atoms paired with the
    // retained store, exactly the ⟨input_fact, D⟩ ast.run reduces
    let fact_v = seq(from_vec(fact.iter().map(to_v).collect()));
    // THE WRITE FLIP (2026-07-08): the handler evaluates on the NATIVE
    // carrier exactly like the rules do — the Scott reduction of
    // apply(handler, ⟨fact, D⟩) with the whole store as a Scott value
    // measured 65 s at tasks scale (the read family's 30,000x lesson,
    // write edition). NEval rides the coherent mirror; the answered D'
    // stays native for the commit path (no re-conversion).
    // AREST_APPLY_SCOTT restores the old reduction.
    let mut d2_native: Option<N> = None;
    let res = if std::env::var_os("AREST_APPLY_SCOTT").is_some() {
        let operand = seq(from_vec(vec![fact_v.clone(), srv.d.clone()]));
        reduce_over(srv, handler, operand, None)
    } else {
        let ev = NEval {
            cells: srv.ncells.clone(),
            process: srv.nprocess.clone(),
            defs_n: srv.nd.clone(),
            fuel: std::cell::Cell::new(-1),
        };
        // rp_reduce_apply (#20, the replay slice) factors out exactly this
        // reduction -- apply(handler, <fact, D>) on the native carrier -- so
        // replay's trigger arm can ride the SAME create internals against a
        // handler it builds fresh (no create:<ft> cell exists yet at replay
        // time; see rp_create_from_spec's own comment). Behavior here is
        // unchanged: same call, same inputs, same result.
        let rn = rp_reduce_apply(&ev, &v_to_n(&handler), &v_to_n(&fact_v), &srv.nd);
        if let N::S(parts) = &rn {
            if parts.len() == 2 {
                d2_native = Some(parts[1].clone());
            }
        }
        n_to_v(&rn)
    };
    let it = items(&list_of(&res));
    if it.len() != 2 {
        return None;
    }
    let o = it[0].clone();
    let d2 = it[1].clone();
    // the bare ERROR atom (an alethic refusal answering the atom, or an
    // authorization refusal) has no ⟨P'', V⟩ to read; Python re-runs validate
    // for the receipt's offenders, so this path delegates
    if matches!(aval(&o), Some(l) if matches!(&*l, Leaf::S(s) if s == "ERROR")) {
        return None;
    }
    // committed iff the store changed (a refused own-table step, e.g. a
    // duplicate the population already holds, answers D' == D)
    let refused = eqobj(&d2, &srv.d);
    // the violation set is the representation's second part (o = ⟨P'', V⟩); a
    // committed step carries the empty V (φ), read as []
    let mut violations: Vec<V> = Vec::new();
    if let Shape::Seq(_) = shape(&o) {
        let oi = items(&list_of(&o));
        if oi.len() >= 2 {
            if let Shape::Seq(_) = shape(&oi[1]) {
                violations = items(&list_of(&oi[1]));
            }
        }
    }
    let mut committed_event: Option<(String, Vec<J>)> = None;
    if !refused {
        // retain D' as the resident store, then run the native derive bounded
        // to the written fact type (Registry.apply's run_rules(D2,
        // changed=[ft])), which replaces the retained store with the fixpoint
        srv.d = d2;
        srv.cells = cells_of(&srv.d);
        // mirror coherence: the native view refreshes with the store —
        // from the handler's own native answer when the flip produced
        // one (no 5 MB re-conversion), else the conversion
        srv.nd = match d2_native.take() {
            Some(n) => n,
            None => v_to_n(&srv.d),
        };
        srv.ncells = n_cells_of(&srv.nd);
        let derive_req = J::O(vec![("changed".to_string(), J::A(vec![J::S(ft.clone())]))]);
        let _ = op_run_rules(&derive_req, srv);
        // persist natively: the committed step to the event log, then the
        // store sidecar refreshed so a fresh resident boots the written store
committed_event = Some((ft.clone(), fact.clone()));
    }
    // the receipt, protocol.py Registry.apply's shape and key order, compact
    // like the delegated path (run_cli re-serializes the CLI receipt the same
    // way through write_j)
    let mut r = String::from("{\"app\":");
    esc(&app, &mut r);
    r.push_str(",\"fact_type\":");
    esc(&ft, &mut r);
    r.push_str(",\"fact\":[");
    for (i, e) in fact.iter().enumerate() {
        if i > 0 {
            r.push(',');
        }
        write_j(e, &mut r);
    }
    r.push_str("],\"committed\":");
    r.push_str(if refused { "false" } else { "true" });
    r.push_str(",\"violations\":[");
    for (i, v) in violations.iter().enumerate() {
        if i > 0 {
            r.push(',');
        }
        write_v(v, &mut r);
    }
    r.push_str("]}");
    Some(Ok((r, committed_event)))
}

// A tool call answers its bare JSON, or a JSON-RPC error pair: -32601 names
// an unknown tool and -32602 names invalid or unusable parameters.
thread_local! {
    // the resident's last mutation receipt, the context verb's answer
    // (protocol.Registry.last_receipt's analog)
    static LAST_RECEIPT: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(feature = "host")]
fn native_apply(args: &J, apps: &Apps, srv: &mut Srv) -> Option<Result<String, (i64, String)>> {
    // the target app must be the retained store; a write to any other app
    // delegates, exactly as the delegated path only reloads the current app
    let app = match jget(args, "app") {
        Some(J::S(a)) => a.clone(),
        _ => return None,
    };
    if apps.current.as_deref() != Some(app.as_str()) {
        return None;
    }
    match apply_core(args, &app, srv)? {
        Ok((receipt, committed)) => {
            if let Some((ft, fact)) = committed {
                // persist natively: the committed step to the event log,
                // then the store sidecar refreshed so a fresh resident
                // boots the written store
                if let Err(e) = append_event(apps, &app, &ft, &fact) {
                    return Some(Err((-32603,
                        format!("committed but the event log write failed: {}", e))));
                }
                if let Err(e) = write_sidecar(apps, &app, srv) {
                    return Some(Err((-32603,
                        format!("committed but the sidecar write failed: {}", e))));
                }
            }
            Some(Ok(receipt))
        }
        Err(e) => Some(Err(e)),
    }
}

#[cfg(feature = "host")]
fn mcp_call(tool: &str, args: &J, apps: &mut Apps, srv: &mut Srv) -> Result<String, (i64, String)> {
    let out = mcp_call_inner(tool, args, apps, srv);
    if matches!(tool, "apply" | "retract") {
        if let Ok(r) = &out {
            LAST_RECEIPT.with(|c| *c.borrow_mut() = Some(r.clone()));
        }
    }
    out
}

// ONE dispatch, two bindings: the STORE-ONLY verb table — no Apps,
// no delegates, srv only (the wasm Worker serves exactly this; the
// MCP host wraps it with the app-loaded guard and the delegate
// escape). The app name is envelope text, passed in. None = not a
// store-only verb, the caller's dispatch continues.
#[allow(clippy::needless_return)]
// The native carrier rendered as JSON (hoisted from the get arm so the
// view surface shares it): atoms escape as scalars, seqs as arrays, ⊥
// as null.
fn n_json(n: &N, out: &mut String) {
    match n {
        N::A(l) => match &**l {
            Leaf::S(s) => esc(s, out),
            Leaf::I(i) => out.push_str(&i.to_string()),
            Leaf::F(f) => out.push_str(&f.to_string()),
            _ => out.push_str("null"),
        },
        N::S(v) => {
            out.push('[');
            for (i, e) in v.iter().enumerate() {
                if i > 0 { out.push(','); }
                n_json(e, out);
            }
            out.push(']');
        }
        N::Bot => out.push_str("null"),
    }
}

// THE VIEW TREES over the wire (the abstract UI, 2026-07-08): the same
// trees the desktop containers render — system:view_detail over the
// entity's field pairs (entity_view's fields leg) and system:view_menu
// over ⟨status, sm-triples⟩ — evaluated ON THE CARRIER and answered as
// JSON. A client is a TRANSDUCER (kind dispatch over the tree), never a
// meaning site; its buttons POST the menu's event fact types back (the
// >>= over HTTP).
fn view_trees_json(noun: &str, id: &str, srv: &Srv) -> Result<String, (i64, String)> {
    let ev = NEval {
        cells: srv.ncells.clone(),
        process: srv.nprocess.clone(),
        defs_n: srv.nd.clone(),
        fuel: std::cell::Cell::new(-1),
    };
    let na = |s: &str| N::A(Rc::new(Leaf::S(s.to_string())));
    let view = ev.mu(napp(
        na("system:entity_view"),
        nseq(vec![na(noun), na(id), srv.nd.clone()]),
    ));
    let fields = match &view {
        N::S(v) if v.len() == 3 => v[1].clone(),
        _ => return Err((-32603,
            "entity_view answered an unexpected shape".to_string())),
    };
    // the boundary transduction the get arm also performs: the hole
    // sentinel is internal representation; absence renders as the
    // empty value, never as '#'
    let fields = match &fields {
        N::S(pairs) => nseq(pairs.iter().map(|p| match p {
            N::S(kv) if kv.len() >= 2 => {
                let hole = matches!(&kv[1],
                    N::A(l) if matches!(&**l, Leaf::S(s) if s == "#"));
                if hole {
                    nseq(vec![kv[0].clone(),
                              N::A(Rc::new(Leaf::S(String::new())))])
                } else {
                    p.clone()
                }
            }
            _ => p.clone(),
        }).collect()),
        _ => fields,
    };
    let detail = ev.mu(napp(na("system:view_detail"), fields));
    let leaf = |s: &str| Leaf::S(s.to_string());
    let sv = |l: &Leaf| match l {
        Leaf::S(s) => Some(s.clone()),
        Leaf::I(i) => Some(i.to_string()),
        _ => None,
    };
    let two = |r: &V| {
        let it = items(&list_of(r));
        if it.len() >= 2 {
            match (aval(&it[0]).and_then(|l| sv(&l)),
                   aval(&it[1]).and_then(|l| sv(&l))) {
                (Some(a), Some(b)) => Some((a, b)),
                _ => None,
            }
        } else {
            None
        }
    };
    let bound: Vec<(String, String)> = pop_rows(&srv.cells, &leaf("smDef"))
        .iter().filter_map(two).map(|(sm, n)| (n, sm)).collect();
    let subs: Vec<(String, String)> = pop_rows(&srv.cells, &leaf("subtype"))
        .iter().filter_map(two).collect();
    let mut n = noun.to_string();
    let mut seen: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    let smd = loop {
        if let Some((_n, sm)) = bound.iter().find(|(bn, _)| *bn == n) {
            break Some(sm.clone());
        }
        if !seen.insert(n.clone()) {
            break None;
        }
        match subs.iter().find(|(s, _)| *s == n) {
            Some((_s, sup)) => n = sup.clone(),
            None => break None,
        }
    };
    let mut out = String::from("{\"views\":[");
    n_json(&detail, &mut out);
    if let Some(smd) = smd {
        let gov = bound.iter().find(|(_n, sm)| *sm == smd)
            .map(|(n, _)| n.clone()).unwrap_or_else(|| noun.to_string());
        let status_ft = pop_rows(&srv.cells, &leaf("smStatusFt")).iter()
            .filter_map(two).find(|(bn, _)| *bn == gov)
            .map(|(_, ft)| ft);
        let mut status: Option<String> = None;
        if let Some(ft) = status_ft {
            let rows = ev.mu(napp(
                na("system:vb_fetch"),
                nseq(vec![na(&ft), srv.nd.clone()]),
            ));
            if let N::S(rs) = &rows {
                for r in rs.iter() {
                    if let N::S(cols) = r {
                        if cols.len() >= 2 {
                            if let (N::A(a), N::A(b)) = (&cols[0], &cols[1]) {
                                if sv(a).as_deref() == Some(id) {
                                    status = sv(b);
                                }
                            }
                        }
                    }
                }
            }
        }
        if let Some(st) = status {
            let pops: Vec<N> = ["smFrom", "smTrigger", "smTo"].iter()
                .map(|c| {
                    let rows = pop_rows(&srv.cells, &leaf(c));
                    v_to_n(&seq(from_vec(rows)))
                })
                .collect();
            let triples = ev.mu(napp(na("system:sm_join"), nseq(pops)));
            let menu = ev.mu(napp(
                na("system:view_menu"),
                nseq(vec![na(&st), triples]),
            ));
            out.push(',');
            n_json(&menu, &mut out);
        }
    }
    out.push_str("]}");
    Ok(out)
}

// THE ENTRY TREE over the wire: system:view_entry over the canon's
// classification (system:ev_cols), evaluated on the carrier — the
// per-noun create form; each input's SubmitKey IS its fact type, and
// the client POSTs one {fact_type, fact} per filled input.
fn entry_tree_json(noun: &str, srv: &Srv) -> Result<String, (i64, String)> {
    // native build of system:view_entry's shape (the certified-override
    // pattern): one input node per classified column, the fact type the
    // node's SubmitKey — ev_cols_native is the same classifier the get
    // arm trusts
    let spine: Vec<(String, N)> = match &srv.nd {
        N::S(cells) => cells
            .iter()
            .filter_map(|c| {
                if let N::S(it) = c {
                    if it.len() == 3 {
                        if let (N::A(l0), N::A(k)) = (&it[0], &it[1]) {
                            if matches!(&**l0, Leaf::S(s) if s == "CELL") {
                                return leaf_str(k)
                                    .map(|key| (key, it[2].clone()));
                            }
                        }
                    }
                }
                None
            })
            .collect(),
        _ => return Err((-32603, "no store".to_string())),
    };
    let na = |s: &str| N::A(Rc::new(Leaf::S(s.to_string())));
    let inputs: Vec<N> = ev_cols_native(&spine, noun)
        .into_iter()
        .map(|(ft, kind, _other, col)| {
            nseq(vec![na("input"), na(&ft), na(&col), na(&kind)])
        })
        .collect();
    let entry = nseq(vec![na("entry"), nseq(inputs)]);
    let mut out = String::from("{\"views\":[");
    n_json(&entry, &mut out);
    out.push_str("]}");
    Ok(out)
}

fn store_call(tool: &str, args: &J, app: &str, srv: &mut Srv)
    -> Option<Result<String, (i64, String)>> {
    let _ = app;
    Some(match tool {
        "nouns" => {
            // the store's noun inventory: the DISTINCT rmapColumns
            // tables (a table's name IS its top noun) — one pass, the
            // menu surface every host shares
            let mut nouns: Vec<String> = Vec::new();
            if let Some((_, N::S(rows))) = srv
                .ncells
                .iter()
                .find(|(k, _)| matches!(k, Leaf::S(s) if s == "rmapColumns"))
            {
                for r in rows.iter() {
                    if let N::S(cc) = r {
                        if !cc.is_empty() {
                            if let N::A(l) = &cc[0] {
                                if let Some(t) = leaf_str(l) {
                                    if !nouns.contains(&t) {
                                        nouns.push(t);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            nouns.sort();
            let mut out = String::from("{\"nouns\":[");
            for (i, n) in nouns.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                esc(n, &mut out);
            }
            out.push_str("]}");
            Ok(out)
        }
        "list" => {
            // NATIVE noun listing: the population's ids off the table
            // INDEX cell, the table resolved through the noun's role-1
            // fts (subtypes list their top supertype's table — the
            // same resolution the entity view uses). One spine pass;
            // the interpretive query never belongs on a serving path.
            let noun = match jget(args, "noun") {
                Some(J::S(s)) => s.clone(),
                _ => return Some(Err((-32602,
                    "list needs a string noun".to_string()))),
            };
            let spine: Vec<(String, N)> = srv
                .ncells
                .iter()
                .filter_map(|(k, v)| match k {
                    Leaf::S(s) => Some((s.clone(), v.clone())),
                    _ => None,
                })
                .collect();
            let cols = ev_cols_native(&spine, &noun);
            // the table is where the noun's first classified ft lives;
            // a noun with no absorbed fts reads its own-name cell
            let table = cols
                .first()
                .and_then(|(ft, _, _, _)| {
                    spine.iter().find_map(|(k, v)| {
                        if k != "rmapColumns" {
                            return None;
                        }
                        if let N::S(rows) = v {
                            for r in rows.iter() {
                                if let N::S(cc) = r {
                                    if cc.len() >= 3 {
                                        if let (N::A(t), N::A(f)) =
                                            (&cc[0], &cc[2])
                                        {
                                            if matches!(&**f, Leaf::S(s) if s == ft) {
                                                return leaf_str(t);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        None
                    })
                })
                .unwrap_or_else(|| noun.clone());
            let mut ids: Vec<String> = Vec::new();
            if let Some((_, N::S(rows))) =
                srv.ncells.iter().find(|(k, _)| matches!(k, Leaf::S(s) if *s == table))
            {
                for r in rows.iter() {
                    let key = match r {
                        N::S(cc) if !cc.is_empty() => match &cc[0] {
                            N::A(l) => leaf_str(l),
                            _ => None,
                        },
                        N::A(l) => leaf_str(l),
                        _ => None,
                    };
                    if let Some(k) = key {
                        if !k.is_empty() && k != "#" && k != "φ" {
                            ids.push(k);
                        }
                    }
                }
            }
            ids.sort();
            ids.dedup();
            let mut out = String::from("{\"noun\":");
            esc(&noun, &mut out);
            out.push_str(",\"ids\":[");
            for (i, id) in ids.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                esc(id, &mut out);
            }
            out.push_str("]}");
            Ok(out)
        }
        "get" => {
            // NATIVE: the 3NF per-entity view — system:entity_view resolves
            // to its canon-named prim (the vb_fetch treatment: one spine
            // pass; the interpretive evaluation measured minutes at tasks
            // scale), ev_cols rides along so the render knows a unary T
            // from a binary value "T", rendered to Registry.get's exact
            // JSON shape. AREST_DELEGATE_READS is the escape hatch.
            let app = app.to_string();
            let noun = match jget(args, "noun") {
                Some(J::S(s)) => s.clone(),
                _ => return Some(Err((-32602, "get needs a string noun".to_string()))),
            };
            let id = match jget(args, "id") {
                Some(J::S(s)) => s.clone(),
                _ => return Some(Err((-32602, "get needs a string id".to_string()))),
            };
            let ev = NEval {
                cells: srv.ncells.clone(),
                process: srv.nprocess.clone(),
                defs_n: srv.nd.clone(),
                fuel: std::cell::Cell::new(-1),
            };
            let na = |s: &str| N::A(Rc::new(Leaf::S(s.to_string())));
            let view = ev.mu(napp(
                na("system:entity_view"),
                nseq(vec![na(&noun), na(&id), srv.nd.clone()]),
            ));
            let cols = ev.mu(napp(
                na("system:ev_cols"),
                nseq(vec![na(&noun), srv.nd.clone()]),
            ));
            let (exists, fields, facts) = match &view {
                N::S(v) if v.len() == 3 => (&v[0], &v[1], &v[2]),
                _ => return Some(Err((-32603,
                    "entity_view answered an unexpected shape".to_string()))),
            };
            let kinds: Vec<String> = match &cols {
                N::S(cs) => cs.iter().map(|c| match c {
                    N::S(cc) if cc.len() >= 2 => match &cc[1] {
                        N::A(l) => leaf_str(l).unwrap_or_default(),
                        _ => String::new(),
                    },
                    _ => String::new(),
                }).collect(),
                _ => Vec::new(),
            };
            let njson = n_json;
            let mut out = String::from("{\"app\":");
            esc(&app, &mut out);
            out.push_str(",\"noun\":");
            esc(&noun, &mut out);
            out.push_str(",\"id\":");
            esc(&id, &mut out);
            out.push_str(",\"exists\":");
            out.push_str(match exists {
                N::A(l) if matches!(&**l, Leaf::S(s) if s == "T") => "true",
                _ => "false",
            });
            out.push_str(",\"fields\":{");
            if let N::S(fs) = fields {
                let mut first = true;
                for (i, f) in fs.iter().enumerate() {
                    if let N::S(kv) = f {
                        if kv.len() == 2 {
                            if !first { out.push(','); }
                            first = false;
                            if let N::A(l) = &kv[0] {
                                esc(&leaf_str(l).unwrap_or_default(), &mut out);
                            } else {
                                out.push_str("\"\"");
                            }
                            out.push(':');
                            let unary = kinds.get(i).map(|k| k == "unary")
                                .unwrap_or(false);
                            match &kv[1] {
                                N::A(l) if unary
                                    && matches!(&**l, Leaf::S(s) if s == "T") =>
                                    out.push_str("true"),
                                N::A(l) if unary
                                    && matches!(&**l, Leaf::S(s) if s == "F") =>
                                    out.push_str("false"),
                                N::A(l) if matches!(&**l, Leaf::S(s) if s == "#") =>
                                    out.push_str("null"),
                                other => njson(other, &mut out),
                            }
                        }
                    }
                }
            }
            out.push_str("},\"facts\":[");
            if let N::S(fx) = facts {
                for (i, f) in fx.iter().enumerate() {
                    if i > 0 { out.push(','); }
                    if let N::S(fr) = f {
                        if fr.len() == 2 {
                            out.push_str("{\"fact_type\":");
                            if let N::A(l) = &fr[0] {
                                esc(&leaf_str(l).unwrap_or_default(), &mut out);
                            } else {
                                out.push_str("\"\"");
                            }
                            out.push_str(",\"row\":");
                            njson(&fr[1], &mut out);
                            out.push('}');
                        }
                    }
                }
            }
            out.push_str("]}");
            Ok(out)
        }
        "actions" => {
            // NATIVE (the store-only family): Theorem 4 off the retained
            // store — the machine walk (smDef + subtype chain), the status
            // via the CANON's vb_fetch (the RMAP-aware ft_view, evaluated
            // on the carrier — no rust reimplementation of the column
            // dispatch), the triples via the CANON's sm_join, filtered
            // from == status. Registry.actions' exact shape and key order.
            let app = app.to_string();
            let noun = match jget(args, "noun") {
                Some(J::S(s)) => s.clone(),
                _ => return Some(Err((-32602, "actions needs a string noun".to_string()))),
            };
            let id = match jget(args, "id") {
                Some(J::S(s)) => s.clone(),
                Some(J::I(i)) => i.to_string(),
                _ => return Some(Err((-32602, "actions needs a scalar id".to_string()))),
            };
            let leaf = |s: &str| Leaf::S(s.to_string());
            let sv = |l: &Leaf| match l {
                Leaf::S(s) => Some(s.clone()),
                Leaf::I(i) => Some(i.to_string()),
                _ => None,
            };
            let two = |r: &V| {
                let it = items(&list_of(r));
                if it.len() >= 2 {
                    match (aval(&it[0]).and_then(|l| sv(&l)),
                           aval(&it[1]).and_then(|l| sv(&l))) {
                        (Some(a), Some(b)) => Some((a, b)),
                        _ => None,
                    }
                } else {
                    None
                }
            };
            let bound: Vec<(String, String)> = pop_rows(&srv.cells, &leaf("smDef"))
                .iter().filter_map(two).map(|(sm, n)| (n, sm)).collect();
            let subs: Vec<(String, String)> = pop_rows(&srv.cells, &leaf("subtype"))
                .iter().filter_map(two).collect();
            let mut n = noun.clone();
            let mut seen: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            let smd = loop {
                if let Some((_n, sm)) = bound.iter().find(|(bn, _)| *bn == n) {
                    break Some(sm.clone());
                }
                if !seen.insert(n.clone()) {
                    break None;
                }
                match subs.iter().find(|(s, _)| *s == n) {
                    Some((_s, sup)) => n = sup.clone(),
                    None => break None,
                }
            };
            let mut out = String::from("{\"app\":");
            esc(&app, &mut out);
            out.push_str(",\"noun\":");
            esc(&noun, &mut out);
            out.push_str(",\"id\":");
            esc(&id, &mut out);
            let smd = match smd {
                None => {
                    out.push_str(",\"machine\":null,\"actions\":[]}");
                    return Some(Ok(out));
                }
                Some(s) => s,
            };
            let gov = bound.iter().find(|(_n, sm)| *sm == smd)
                .map(|(n, _)| n.clone()).unwrap_or_else(|| noun.clone());
            let status_ft = pop_rows(&srv.cells, &leaf("smStatusFt")).iter()
                .filter_map(two).find(|(bn, _)| *bn == gov)
                .map(|(_, ft)| ft);
            // reads trust the mirror (the coherence audit, 2026-07-08):
            // the 247 s per-call v_to_n rebuild at tasks scale retires
            let ev = NEval {
                cells: srv.ncells.clone(),
                process: srv.nprocess.clone(),
                defs_n: srv.nd.clone(),
                fuel: std::cell::Cell::new(-1),
            };
            let mut status: Option<String> = None;
            if let Some(ft) = status_ft {
                let rows = ev.mu(napp(
                    N::A(Rc::new(Leaf::S("system:vb_fetch".into()))),
                    nseq(vec![N::A(Rc::new(Leaf::S(ft))), srv.nd.clone()]),
                ));
                if let N::S(rs) = &rows {
                    for r in rs.iter() {
                        if let N::S(cols) = r {
                            if cols.len() >= 2 {
                                if let (N::A(a), N::A(b)) = (&cols[0], &cols[1]) {
                                    if sv(a).as_deref() == Some(id.as_str()) {
                                        status = sv(b);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            let pops: Vec<N> = ["smFrom", "smTrigger", "smTo"].iter()
                .map(|c| {
                    let rows = pop_rows(&srv.cells, &leaf(c));
                    v_to_n(&seq(from_vec(rows)))
                })
                .collect();
            let triples = ev.mu(napp(
                N::A(Rc::new(Leaf::S("system:sm_join".into()))),
                nseq(pops),
            ));
            out.push_str(",\"machine\":");
            esc(&smd, &mut out);
            out.push_str(",\"status\":");
            match &status {
                Some(s) => esc(s, &mut out),
                None => out.push_str("null"),
            }
            out.push_str(",\"actions\":[");
            let mut first = true;
            if let (Some(st), N::S(ts)) = (&status, &triples) {
                for t in ts.iter() {
                    if let N::S(cols) = t {
                        if cols.len() >= 3 {
                            if let (N::A(f), N::A(e), N::A(to)) =
                                (&cols[0], &cols[1], &cols[2])
                            {
                                if sv(f).as_deref() == Some(st.as_str()) {
                                    if !first { out.push(','); }
                                    first = false;
                                    out.push_str("{\"event\":");
                                    esc(&sv(e).unwrap_or_default(), &mut out);
                                    out.push_str(",\"to\":");
                                    esc(&sv(to).unwrap_or_default(), &mut out);
                                    out.push('}');
                                }
                            }
                        }
                    }
                }
            }
            out.push_str("]}");
            Ok(out)
        }
        "schema" => {
            // NATIVE (the store-only read family, 2026-07-08): the model
            // surface straight off the retained cells, mirroring
            // Registry.schema's shape and ordering. AREST_DELEGATE_READS=1
            // is the family's escape hatch back to the python delegate.
            let app = app.to_string();
            let leaf = |s: &str| Leaf::S(s.to_string());
            let sv = |l: &Leaf| match l {
                Leaf::S(s) => Some(s.clone()),
                Leaf::I(i) => Some(i.to_string()),
                _ => None,
            };
            let mut nouns: Vec<(String, String)> = Vec::new();
            for r in pop_rows(&srv.cells, &leaf("instanceOf")) {
                let it = items(&list_of(&r));
                if it.len() >= 2 {
                    if let (Some(n), Some(k)) =
                        (aval(&it[0]).and_then(|l| sv(&l)),
                         aval(&it[1]).and_then(|l| sv(&l)))
                    {
                        if k == "ObjectType" || k == "ValueType" {
                            nouns.push((n, k));
                        }
                    }
                }
            }
            nouns.sort();
            let mut roles: std::collections::HashMap<String, Vec<(i64, String)>> =
                std::collections::HashMap::new();
            for r in pop_rows(&srv.cells, &leaf("role")) {
                let it = items(&list_of(&r));
                if it.len() >= 4 {
                    if let (Some(ft), Some(Leaf::I(i)), Some(p)) = (
                        aval(&it[1]).and_then(|l| sv(&l)),
                        aval(&it[2]).as_deref().cloned().into(),
                        aval(&it[3]).and_then(|l| sv(&l)),
                    ) {
                        roles.entry(ft).or_default().push((i, p));
                    }
                }
            }
            let mut fts: Vec<(String, String)> = Vec::new();
            for f in pop_rows(&srv.cells, &leaf("factType")) {
                let it = items(&list_of(&f));
                if it.len() >= 2 {
                    if let (Some(id), Some(rd)) =
                        (aval(&it[0]).and_then(|l| sv(&l)),
                         aval(&it[1]).and_then(|l| sv(&l)))
                    {
                        fts.push((id, rd));
                    }
                }
            }
            fts.sort();
            let mut out = String::from("{\"app\":");
            esc(&app, &mut out);
            out.push_str(",\"object_types\":[");
            for (i, (n, k)) in nouns.iter().enumerate() {
                if i > 0 { out.push(','); }
                out.push_str("{\"name\":");
                esc(n, &mut out);
                out.push_str(",\"kind\":");
                esc(k, &mut out);
                out.push('}');
            }
            out.push_str("],\"fact_types\":[");
            for (i, (id, rd)) in fts.iter().enumerate() {
                if i > 0 { out.push(','); }
                out.push_str("{\"id\":");
                esc(id, &mut out);
                out.push_str(",\"reading\":");
                esc(rd, &mut out);
                out.push_str(",\"roles\":[");
                let mut rs = roles.get(id).cloned().unwrap_or_default();
                rs.sort();
                for (j, (_i, p)) in rs.iter().enumerate() {
                    if j > 0 { out.push(','); }
                    esc(p, &mut out);
                }
                out.push_str("]}");
            }
            out.push_str("],\"constraints\":[");
            let mut first = true;
            for c in pop_rows(&srv.cells, &leaf("constraint")) {
                let it = items(&list_of(&c));
                if it.len() >= 2 {
                    if let (Some(id), Some(k)) =
                        (aval(&it[0]).and_then(|l| sv(&l)),
                         aval(&it[1]).and_then(|l| sv(&l)))
                    {
                        if !first { out.push(','); }
                        first = false;
                        out.push_str("{\"id\":");
                        esc(&id, &mut out);
                        out.push_str(",\"kind\":");
                        esc(&k, &mut out);
                        out.push_str(",\"fact_type\":");
                        match it.get(2).and_then(|x| aval(x)).and_then(|l| sv(&l)) {
                            Some(ft) => esc(&ft, &mut out),
                            None => out.push_str("null"),
                        }
                        out.push('}');
                    }
                }
            }
            out.push_str("]}");
            Ok(out)
        }
        "synthesize" => {
            // NATIVE BY DEFAULT (the ~40x plumb closed 2026-07-08): the MCP
            // synthesize evaluates verbalize on the carrier and RENDERS the
            // Registry's exact shape ({app, id, facts:[{reading, row,
            // text}]}) — the delegated python answered this and the pins
            // hold it; the python spawn it replaces measured 35+ MINUTES
            // at tasks scale. AREST_SYNTH_SCOTT=1 restores the delegate.
            let app = app.to_string();
            let id = match jget(args, "id") {
                Some(J::S(s)) => atom(Leaf::S(s.clone())),
                Some(J::I(i)) => atom(Leaf::I(*i)),
                _ => return Some(Err((-32602,
                    "synthesize needs a scalar id".to_string()))),
            };
            let pairs = native_verbalize(srv, &id);
            let mut r = String::from("{\"app\":");
            esc(&app, &mut r);
            r.push_str(",\"id\":");
            write_v(&id, &mut r);
            r.push_str(",\"facts\":[");
            let mut first = true;
            for p in items(&list_of(&pairs)) {
                let it = items(&list_of(&p));
                if it.len() != 2 {
                    continue;
                }
                let reading = match aval(&it[0]).as_deref() {
                    Some(Leaf::S(s)) => s.clone(),
                    _ => continue,
                };
                let row: Vec<String> = items(&list_of(&it[1]))
                    .iter()
                    .filter_map(|x| aval(x).map(|l| match &*l {
                        Leaf::S(s) => s.clone(),
                        Leaf::I(i) => i.to_string(),
                        Leaf::F(fl) => fl.to_string(),
                        _ => String::new(),
                    }))
                    .collect();
                let mut text = reading.clone();
                for (i, v) in row.iter().enumerate() {
                    text = text.replace(&format!("{{{}}}", i), v);
                }
                if !first {
                    r.push(',');
                }
                first = false;
                r.push_str("{\"reading\":");
                esc(&reading, &mut r);
                r.push_str(",\"row\":");
                write_v(&it[1], &mut r);
                r.push_str(",\"text\":");
                esc(&text, &mut r);
                r.push('}');
            }
            r.push_str("]}");
            Ok(r)
        }
        "query" | "cells" => {
            // The MCP arguments object IS the op request body.
            op_answer(tool, args, srv).map_err(|m| (-32602, m))
        }
        "derive" => {
            // derive is the run_rules op under the daily driver's tool
            // name: the naive fixpoint runs natively over the retained
            // store and replaces it in place.
            op_answer("run_rules", args, srv).map_err(|m| (-32602, m))
        }
        _ => return None,
    })
}

#[cfg(feature = "host")]
fn mcp_call_inner(tool: &str, args: &J, apps: &mut Apps, srv: &mut Srv) -> Result<String, (i64, String)> {
    // ONE dispatch, two bindings (the doctrine): the STORE-ONLY verbs
    // route through the hostless store_call the wasm Worker shares.
    // The host keeps the app-loaded guard and the delegate escape.
    if matches!(tool, "get" | "actions" | "schema" | "synthesize"
                    | "query" | "cells" | "derive") {
        if apps.current.is_none() {
            return Err((-32602,
                format!("no app loaded; call apps_use before {}", tool)));
        }
        if (std::env::var_os("AREST_DELEGATE_READS").is_some()
            && matches!(tool, "get" | "actions" | "schema" | "synthesize"))
            || (tool == "synthesize"
                && std::env::var_os("AREST_SYNTH_SCOTT").is_some())
        {
            return delegate_read(tool, args, apps);
        }
        let app = apps.current.clone().unwrap_or_default();
        if let Some(r) = store_call(tool, args, &app, srv) {
            return r;
        }
    }

    match tool {
        "context" => Ok(LAST_RECEIPT.with(|c| c.borrow().clone())
            .unwrap_or_else(|| "{\"note\":\"no mutation this session\"}".to_string())),
        "orient" => {
            let mut r = String::from("{\"apps\":");
            esc_names(&apps.list(), &mut r);
            r.push_str(",\"current\":");
            match &apps.current {
                Some(n) => esc(n, &mut r),
                None => r.push_str("null"),
            }
            r.push('}');
            Ok(r)
        }
        "apps_list" => {
            let mut r = String::new();
            esc_names(&apps.list(), &mut r);
            Ok(r)
        }
        "apps_current" => {
            let mut r = String::from("{\"current\":");
            match &apps.current {
                Some(n) => esc(n, &mut r),
                None => r.push_str("null"),
            }
            r.push('}');
            Ok(r)
        }
        "apps_use" => {
            let name = match jget(args, "name") {
                Some(J::S(n)) => n.clone(),
                _ => return Err((-32602, "apps_use needs a string name".to_string())),
            };
            load_sidecar(apps, &name, srv).map_err(|m| (-32602, m))?;
            apps.current = Some(name.clone());
            let mut r = String::from("{\"app\":");
            esc(&name, &mut r);
            r.push_str(",\"ok\":true}");
            Ok(r)
        }
        // apply flips native for OWN-TABLE: a fact type carrying a create:<ft>
        // handler cell computes and persists in process (native_apply); an
        // absorbed fact type, a non-retained target app, or a bare-ERROR
        // refusal answers None and falls through to the CLI delegate. retract
        // and apps_compile still delegate whole.
        "apply" => match native_apply(args, apps, srv) {
            Some(res) => res,
            None => delegate_verb(tool, args, apps, srv),
        },
        "retract" | "apps_compile" => delegate_verb(tool, args, apps, srv),
        // synthesize delegates for now: the canonical verbalize over the
        // daily driver's store reduces in minutes on this path where the
        // Python host's native twins answer in seconds (measured 2026-07-05
        // on the claude scratch: 264 s canonical Rust against 10.9 s
        // delegated). Plumbing the native carrier into op_answer is the
        // priced lever that brings it home.

        "sql" | "explain" | "validate" | "verify" => {
            delegate_read(tool, args, apps)
        }





        "engine_version" => Ok("{\"engine\":\"arest\",\"version\":\"0.9.0\"}".to_string()),
        "apps_status" => {
            let name = match jget(args, "name") {
                Some(J::S(n)) => n.clone(),
                _ => return Err((-32602, "apps_status needs a string name".to_string())),
            };
            Ok(app_status_json(apps, &name))
        }
        "apps_check" => {
            // the registry-wide sweep, filesystem-derived like the registry
            let include_ready = !matches!(jget(args, "include_ready"), Some(J::B(false)));
            let (mut ready, mut stale, mut library, mut not_found) = (0u32, 0u32, 0u32, 0u32);
            let mut rows = String::from("[");
            let mut first = true;
            for name in apps.list() {
                let st = app_status_json(apps, &name);
                let health = if st.contains("\"exists\":false") {
                    not_found += 1; "not_found"
                } else if st.contains("\"readings\":0") {
                    library += 1; "library"
                } else if st.contains("\"stale\":true") {
                    stale += 1; "stale"
                } else {
                    ready += 1; "ready"
                };
                if health == "ready" && !include_ready {
                    continue;
                }
                if !first { rows.push(','); }
                first = false;
                rows.push_str(&st[..st.len() - 1]);
                rows.push_str(&format!(",\"health\":\"{}\"}}", health));
            }
            rows.push(']');
            Ok(format!(
                "{{\"summary\":{{\"ready\":{},\"stale\":{},\"library\":{},\"not_found\":{}}},\"apps\":{}}}",
                ready, stale, library, not_found, rows))
        }
        "apps_register" => {
            let mut r = String::from("{\"registered\":");
            esc_names(&apps.list(), &mut r);
            r.push_str(",\"note\":\"directory-derived; nothing written\"}");
            Ok(r)
        }
        "apps_create" => {
            let name = match jget(args, "name") {
                Some(J::S(n)) => n.clone(),
                _ => return Err((-32602, "apps_create needs a string name".to_string())),
            };
            let d = std::path::Path::new(&apps.dir).join(&name);
            if d.is_dir() {
                return Err((-32602, format!("app {:?} already exists", name)));
            }
            let rd = d.join("readings");
            if let Err(e) = std::fs::create_dir_all(&rd) {
                return Err((-32603, format!("could not create {}: {}", rd.display(), e)));
            }
            let text = match jget(args, "text") {
                Some(J::S(t)) => t.clone(),
                _ => format!("# {}
", name),
            };
            if let Err(e) = std::fs::write(rd.join("core.md"), text) {
                return Err((-32603, format!("could not write core.md: {}", e)));
            }
            let mut r = String::from("{\"created\":");
            esc(&name, &mut r);
            r.push_str(",\"readings\":1}");
            Ok(r)
        }
        // the verbs needing the compiler host ride the generic call form —
        // one Python dispatch table, never a second registry here. The live
        // additive compile mutates the app, so the retained sidecar reloads.
        "compile" => delegate_call("compile", args, apps, srv, true),
        // the tutor surface + select_component (the 2026-07-08 ports): the
        // generic call form, one Python dispatch table — never a second
        // registry here. The tutor write verbs reload like compile; the
        // reads reload nothing.
        "select_component" | "tutor_list" | "tutor_get" | "tutor_check"
        | "tutor_query" | "tutor_actions" | "tutor_authoring" => {
            delegate_call(tool, args, apps, srv, false)
        }
        "tutor_apply" | "tutor_compile" | "tutor_propose" | "tutor_reset" => {
            delegate_call(tool, args, apps, srv, true)
        }
        "propose" | "induce" | "ask" => delegate_call(tool, args, apps, srv, false),
        _ => Err((-32601, format!("unknown tool {:?}", tool))),
    }
}

#[cfg(feature = "host")]
fn run_mcp() {
    use std::io::{BufRead, Write};
    let (dir, python, py_cli) = {
        let mut args = std::env::args();
        let mut dir = None;
        let mut python = None;
        let mut py_cli = None;
        while let Some(a) = args.next() {
            if a == "--apps-dir" {
                dir = args.next();
            } else if a == "--python" {
                python = args.next();
            } else if a == "--py-cli" {
                py_cli = args.next();
            }
        }
        match dir {
            Some(d) => (d, python, py_cli),
            None => {
                eprintln!("--mcp needs --apps-dir <path>");
                std::process::exit(2);
            }
        }
    };
    let mut apps = Apps {
        dir: std::path::PathBuf::from(dir),
        current: None,
        python: python.unwrap_or_else(|| "python".to_string()),
        cli: py_cli.map(std::path::PathBuf::from).or_else(find_cli),
    };
    let mut srv = Srv { d: phi(), cells: Vec::new(), mu: make_mu(),
                        nd: N::Bot, ncells: Vec::new(), nprocess: Vec::new() };
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        let j = match parse_json(&line) {
            Some(j) => j,
            None => continue, // malformed with no recoverable id: skip the line
        };
        // A notification carries no id (a null id counts as none); it still
        // computes, but it never answers, mirroring the Python binding.
        let id = match jget(&j, "id") {
            None | Some(J::Null) => None,
            Some(other) => Some(other),
        };
        let method = match jget(&j, "method") {
            Some(J::S(m)) => m.as_str(),
            _ => "",
        };
        let answer: Option<Result<String, (i64, String)>> = match method {
            "initialize" => {
                let mut r = String::from("{\"protocolVersion\":");
                match jget(&j, "params").and_then(|p| jget(p, "protocolVersion")) {
                    Some(J::S(v)) => esc(v, &mut r),
                    _ => r.push_str("\"2024-11-05\""),
                }
                r.push_str(",\"capabilities\":{\"tools\":{}},\
                            \"serverInfo\":{\"name\":\"arest\",\"version\":\"0.1.0\"}}");
                Some(Ok(r))
            }
            "tools/list" => Some(Ok(format!("{{\"tools\":{}}}", MCP_TOOLS))),
            "tools/call" => {
                let none = J::O(Vec::new());
                let p = jget(&j, "params").unwrap_or(&none);
                let args = jget(p, "arguments").unwrap_or(&none);
                match jget(p, "name") {
                    Some(J::S(t)) => Some(mcp_call(t, args, &mut apps, &mut srv).map(|a| {
                        // the MCP content envelope: the tool's JSON answer rides as text
                        let mut r = String::from("{\"content\":[{\"type\":\"text\",\"text\":");
                        esc(&a, &mut r);
                        r.push_str("}]}");
                        r
                    })),
                    _ => Some(Err((-32602, "tools/call needs a tool name".to_string()))),
                }
            }
            _ => {
                if id.is_none() {
                    None // any other notification is consumed silently
                } else {
                    Some(Err((-32601, format!("unknown method {:?}", method))))
                }
            }
        };
        if let (Some(ans), Some(idj)) = (answer, id) {
            let mut out = String::from("{\"jsonrpc\":\"2.0\",\"id\":");
            write_j(idj, &mut out);
            match ans {
                Ok(result) => {
                    out.push_str(",\"result\":");
                    out.push_str(&result);
                }
                Err((code, msg)) => {
                    out.push_str(",\"error\":{\"code\":");
                    out.push_str(&code.to_string());
                    out.push_str(",\"message\":");
                    esc(&msg, &mut out);
                    out.push('}');
                }
            }
            out.push('}');
            let mut so = std::io::stdout();
            so.write_all(out.as_bytes()).ok();
            so.write_all(b"\n").ok();
            so.flush().ok();
        }
    }
}

#[cfg(feature = "host")]
fn show_case(v: &V, out: &mut String) {
    match shape(v) {
        Shape::Bot => out.push('\u{22a5}'),
        Shape::Atom(l) => match &*l {
            Leaf::S(s) => { out.push('\''); out.push_str(s); out.push('\''); }
            Leaf::I(i) => out.push_str(&i.to_string()),
            Leaf::F(f) => {
                if f.fract() == 0.0 && f.is_finite() {
                    out.push_str(&format!("{:.1}", f));
                } else {
                    out.push_str(&format!("{}", f));
                }
            }
            Leaf::AppTag => out.push_str("#APP#"),
        },
        Shape::Seq(l) => {
            out.push('(');
            for (i, x) in items(&l).iter().enumerate() {
                if i > 0 { out.push_str(", "); }
                show_case(x, out);
            }
            out.push(')');
        }
    }
}

#[cfg(feature = "host")]
fn run() {
    // the intersection source loads ON THE WORKER (CANON is a thread_local; the
    // reduction thread must hold it)
    CANON.with(|c| {
        *c.borrow_mut() = canon_defs();
    });
    // the native mirror of the canon, converted once: the native carrier NEval
    // resolves a canon def through it when a partial process list does not carry
    // it (a hand-built store), exactly where the Scott mu resolves through CANON.
    // canon_defs builds only data terms (atoms and sequences, never closures),
    // so v_to_n converts each faithfully.
    NCANON.with(|nc| {
        *nc.borrow_mut() = CANON.with(|c| {
            c.borrow().iter().map(|(n, v)| (n.clone(), v_to_n(v))).collect()
        });
    });
    register_base();
    register_overrides();                                     // twins on by default
    if std::env::args().any(|a| a == "--cases") {
        // the cross-host case table: reduce each pair and print name=result
        // in the convention every host shares
        let mu = make_mu();
        for (name, pair) in scenario_defs() {
            let expr = nth(&pair, 0);
            let operand = nth(&pair, 1);
            let v = mu.app(mkapp(expr, operand));
            let mut line = String::new();
            show_case(&v, &mut line);
            println!("{}={}", name, line);
        }
        return;
    }
    if std::env::args().any(|a| a == "--mcp") {
        // the MCP stdio binding over an apps directory of persisted stores
        run_mcp();
        return;
    }
    let serve = std::env::args().any(|a| a == "--serve");
    let mut srv = Srv { d: phi(), cells: Vec::new(), mu: make_mu(),
                        nd: N::Bot, ncells: Vec::new(), nprocess: Vec::new() };
    if serve {
        use std::io::{BufRead, Write};
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };
            if line.trim().is_empty() {
                continue;
            }
            let j = P { b: line.as_bytes(), i: 0 }.parse();
            let out = handle(&j, &mut srv, true);
            let mut so = std::io::stdout();
            so.write_all(out.as_bytes()).ok();
            so.write_all(b"
").ok();
            so.flush().ok();
        }
    } else {
        let mut input = String::new();
        std::io::stdin().read_to_string(&mut input).unwrap();
        let j = P { b: input.as_bytes(), i: 0 }.parse();
        let out = handle(&j, &mut srv, false);
        if !out.is_empty() {
            println!("{}", out);
        }
    }
}

fn seqv(xs: Vec<V>) -> V {
    let mut l = nil();
    for x in xs.into_iter().rev() {
        l = cons(x, l);
    }
    seq(l)
}

// host_bootstrap_defs (#20, the final pipeline slice's sidecar completion):
// the 8 python-only compiled defs engine.py binds directly at MODULE level
// -- NOT via canon.load() -- confirmed by tracing every kernel.define() call
// during a real package import: these are the LAST 8 entries in
// kernel.latest, right after the 325 theta/constraints/ast/system.canon
// names canon.load_all() installs (NCANON's own content, below), and they
// appear in NEITHER any of the four shared .canon files NOR any other
// canon.load() call site -- engine.py:805-809 ("the default (minimal)
// stages") and :3586/3608/3615 (machine_run/rmap/csdp, the self-host
// bootstrap's own RMAP/machine-run/CSDP values) bind them as literal,
// hand-built Scott trees over the SAME _S(...)/A(...) vocabulary the canon
// files use. Transcribed here line-for-line from that source (verified
// against a python probe of _conv(from_lam(obj)) for each of the 8 names) so
// the sidecar's "process" field reaches python's own 333-entry content
// byte-for-byte, not merely NCANON's 325 -- these 8 have no OTHER purpose in
// this binary (op_run_rules/scheduler_cells/create_handlers etc. never
// dispatch through them; they are dead weight for every op except this
// diagnostic completeness).
fn host_bootstrap_defs() -> Vec<(String, N)> {
    let a1 = || N::A(Rc::new(Leaf::I(1)));
    let a2 = || N::A(Rc::new(Leaf::I(2)));
    let a3 = || N::A(Rc::new(Leaf::I(3)));
    let phi_n = || nseq(vec![]); // PHI = seq(nil()) -- the V-level phi()'s own N analog

    // resolve/derive/validate/emit/create (engine.py:799-809)
    let resolve_def = mf_na("apndl");
    let derive_def = mf_na("id");
    let validate_def = nseq(vec![mf_na("CONS"), mf_na("id"), nseq(vec![mf_na("CONST"), phi_n()])]);
    let emit_def = a1();
    let create_def = nseq(vec![
        mf_na("COMP"),
        mf_na("emit"),
        mf_na("validate"),
        mf_na("derive"),
        mf_na("resolve"),
    ]);

    // run: machine_run (engine.py:3575-3586), foldl(t, acc0, inputs) as one
    // WHILE loop over the state triple <t, acc, remaining>
    let input_ = nseq(vec![mf_na("COMP"), a1(), a3()]);
    let new_acc = nseq(vec![
        mf_na("COMP"),
        mf_na("apply"),
        nseq(vec![mf_na("CONS"), a1(), nseq(vec![mf_na("CONS"), a2(), input_])]),
    ]);
    let new_rem = nseq(vec![mf_na("COMP"), mf_na("tl"), a3()]);
    let step = nseq(vec![mf_na("CONS"), a1(), new_acc, new_rem]);
    let hasmore = nseq(vec![mf_na("COMP"), mf_na("not"), mf_na("null"), a3()]);
    let loop_ = nseq(vec![mf_na("WHILE"), hasmore, step]);
    let init_ = nseq(vec![
        mf_na("CONS"),
        a1(),
        nseq(vec![mf_na("COMP"), a1(), a2()]),
        nseq(vec![mf_na("COMP"), a2(), a2()]),
    ]);
    let run_def = nseq(vec![mf_na("COMP"), a2(), loop_, init_]);

    // rmap (engine.py:3595-3608): the RMAP table-key assignment as a value
    let kind_ = nseq(vec![mf_na("COMP"), a3(), a2()]);
    let ot_ = nseq(vec![mf_na("COMP"), a2(), a2()]);
    let ft_ = nseq(vec![mf_na("COMP"), a1(), a2()]);
    let is_functional = nseq(vec![
        mf_na("COMP"),
        mf_na("eq"),
        nseq(vec![mf_na("CONS"), kind_, nseq(vec![mf_na("CONST"), mf_na("functional")])]),
    ]);
    let table_key = nseq(vec![mf_na("COND"), is_functional, ot_, ft_.clone()]);
    let entry = nseq(vec![mf_na("CONS"), table_key, ft_]);
    let rmap_def =
        nseq(vec![mf_na("COMP"), mf_na("apndr"), nseq(vec![mf_na("CONS"), a1(), entry])]);

    // csdp (engine.py:3611-3615): the CSDP populate step, literally apndr
    let csdp_def = mf_na("apndr");

    vec![
        ("resolve".to_string(), resolve_def),
        ("derive".to_string(), derive_def),
        ("validate".to_string(), validate_def),
        ("emit".to_string(), emit_def),
        ("create".to_string(), create_def),
        ("run".to_string(), run_def),
        ("rmap".to_string(), rmap_def),
        ("csdp".to_string(), csdp_def),
    ]
}

// ============================ intersection source =============================
// shared/*.py are INTERSECTION SOURCE: normal Python modules AND, include!d here,
// normal Rust. One file, two hosts, verbatim; the vocabulary bound below (DEF, A,
// N, PHI, S2..S9) is this platform's lambda, so the lambda used determines the
// implementation. No JSON shim, no parser: rustc tokenizes the same bytes CPython
// executes. The file is ONE tuple literal (include! takes a single expression);
// elements evaluate left to right in both languages. Constraints the file honors:
// double-quoted strings, no imports, no assignments; PHI is nullary (PHI()) so a
// file may use it any number of times.
#[allow(non_snake_case, unused, path_statements)]
fn canon_defs() -> Vec<(String, V)> {
    let out: RefCell<Vec<(String, V)>> = RefCell::new(Vec::new());
    {
        let DEF = |n: &str, o: V| out.borrow_mut().push((n.to_string(), o));
        let A = |s: &str| atom(Leaf::S(s.to_string()));
        let N = |i: i64| atom(Leaf::I(i));
        let K = |x: V| seqv(vec![atom(Leaf::S("CONST".to_string())), x]);
        let PHI = || phi();
        let S1 = |a: V| seqv(vec![a]);
        let S2 = |a: V, b: V| seqv(vec![a, b]);
        let S3 = |a: V, b: V, c: V| seqv(vec![a, b, c]);
        let S4 = |a: V, b: V, c: V, d: V| seqv(vec![a, b, c, d]);
        let S5 = |a: V, b: V, c: V, d: V, e: V| seqv(vec![a, b, c, d, e]);
        let S6 = |a: V, b: V, c: V, d: V, e: V, f: V| seqv(vec![a, b, c, d, e, f]);
        let S7 = |a: V, b: V, c: V, d: V, e: V, f: V, g: V| seqv(vec![a, b, c, d, e, f, g]);
        let S8 = |a: V, b: V, c: V, d: V, e: V, f: V, g: V, h: V| seqv(vec![a, b, c, d, e, f, g, h]);
        let S9 = |a: V, b: V, c: V, d: V, e: V, f: V, g: V, h: V, i: V| seqv(vec![a, b, c, d, e, f, g, h, i]);
        include!("../../shared/theta.canon");
        include!("../../shared/constraints.canon");
        include!("../../shared/ast.canon");
        include!("../../shared/system.canon");
    }
    out.into_inner()
}

// The cross-host case table (shared/scenarios.canon), the same bytes the Python,
// C#, and Java hosts consume: each DEF is ⟨expr, operand⟩, reduced by --cases.
#[allow(non_snake_case, unused, path_statements)]
fn scenario_defs() -> Vec<(String, V)> {
    let out: RefCell<Vec<(String, V)>> = RefCell::new(Vec::new());
    {
        let DEF = |n: &str, o: V| out.borrow_mut().push((n.to_string(), o));
        let A = |s: &str| atom(Leaf::S(s.to_string()));
        let N = |i: i64| atom(Leaf::I(i));
        let K = |x: V| seqv(vec![atom(Leaf::S("CONST".to_string())), x]);
        let PHI = || phi();
        let S1 = |a: V| seqv(vec![a]);
        let S2 = |a: V, b: V| seqv(vec![a, b]);
        let S3 = |a: V, b: V, c: V| seqv(vec![a, b, c]);
        let S4 = |a: V, b: V, c: V, d: V| seqv(vec![a, b, c, d]);
        let S5 = |a: V, b: V, c: V, d: V, e: V| seqv(vec![a, b, c, d, e]);
        let S6 = |a: V, b: V, c: V, d: V, e: V, f: V| seqv(vec![a, b, c, d, e, f]);
        let S7 = |a: V, b: V, c: V, d: V, e: V, f: V, g: V| seqv(vec![a, b, c, d, e, f, g]);
        let S8 = |a: V, b: V, c: V, d: V, e: V, f: V, g: V, h: V| seqv(vec![a, b, c, d, e, f, g, h]);
        let S9 = |a: V, b: V, c: V, d: V, e: V, f: V, g: V, h: V, i: V| seqv(vec![a, b, c, d, e, f, g, h, i]);
        include!("../../shared/scenarios.canon");
    }
    out.into_inner()
}

#[cfg(feature = "host")]
fn main() {
    // mu recurses deeply through closures; give it room
    std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(run)
        .unwrap()
        .join()
        .unwrap();
}

// a bin target must carry SOME main; the no-host build (wasm32 core)
// gets a stub — the Worker consumes the LIBRARY target instead
#[cfg(not(feature = "host"))]
fn main() {}

// ---- the Worker surface (feature = "worker", wasm-bindgen) ----
// Step 2 of the wasm arc: prove the toolchain with the CORE exported —
// the JS engine evaluates canon. arest_eval takes f and x as the
// serve-op JSON term encoding (j_to_n), evaluates on the native
// carrier with the compiled-in NCANON, and answers the result the
// same way. The verb-table dispatch (init store + get/actions/...)
// is step 3, the mcp_call_inner hostless refactor.
#[cfg(feature = "worker")]
// pub: wasm-bindgen exports the symbols regardless, but RUST consumers
// of the rlib (engine/os hosts this same verb surface on bare UEFI —
// wasm_bindgen compiles to plain attributes off-wasm) need the module
// reachable as arest_core::worker::*.
pub mod worker {
    use super::*;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    pub fn arest_version() -> String {
        // derived, never hard-coded: the crate version from Cargo, the
        // host from the target (this same verb surface serves wasm
        // Workers AND bare-UEFI engine/os)
        let host = if cfg!(target_arch = "wasm32") { "wasm" }
                   else if cfg!(target_os = "uefi") { "uefi" }
                   else { "native" };
        format!("arest {} core ({})", env!("CARGO_PKG_VERSION"), host)
    }

    use std::cell::RefCell;

    // the base + override registries install once per isolate — the
    // native binaries do this in main(); the wasm entries must do it
    // themselves or every handler body evaluates against empty
    // registers and reads as refused (the local-smoke lesson).
    fn ensure_base() {
        thread_local! {
            static INIT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        }
        INIT.with(|i| {
            if !i.get() {
                register_base();
                register_overrides();
                i.set(true);
            }
        });
    }

    thread_local! {
        // wasm is single-threaded: the retained stores KEY BY APP — a
        // tenant is a cell whose contents is an entire store (AREST.tex
        // sec:cells, tenancy one level up), so the map's slots are the
        // tenant cells and isolation is unaddressability (Prop. tenant:
        // every verb serves only the ACTIVE slot arest_use selected).
        // Callers that never select (the MCP host, engine/os, the
        // pre-tenancy worker) ride the "" slot — the old single store.
        static WSTORES: RefCell<HashMap<String, Srv>> =
            RefCell::new(HashMap::new());
        static WAPP: RefCell<String> = RefCell::new(String::new());
    }

    fn fresh_srv() -> Srv {
        Srv {
            d: phi(),
            cells: Vec::new(),
            mu: make_mu(),
            nd: N::Bot,
            ncells: Vec::new(),
            nprocess: Vec::new(),
        }
    }

    fn with_active<R>(f: impl FnOnce(&mut Srv) -> R) -> R {
        // TAKE the store out, run f with NO thread-local borrow held,
        // put it back: a reentrant wasm touch (a registered override,
        // a host callback) must never observe WSTORES borrowed — the
        // 2026-07-09 production 1101s were core::cell::
        // panic_already_borrowed from running the verb body inside
        // the map borrow. (A panicking f loses the store; a wasm
        // panic aborts the isolate anyway.)
        let app = WAPP.with(|a| a.borrow().clone());
        let mut srv = WSTORES
            .with(|m| m.borrow_mut().remove(&app))
            .unwrap_or_else(fresh_srv);
        let r = f(&mut srv);
        WSTORES.with(|m| m.borrow_mut().insert(app, srv));
        r
    }

    #[wasm_bindgen]
    pub fn arest_use(app: &str) -> String {
        // select the tenant cell; every later verb serves THIS store.
        // Cheap and synchronous by design: the JS side re-selects after
        // every await (isolate single-threadedness makes use+call
        // atomic within a microtask).
        ensure_base();
        WAPP.with(|a| *a.borrow_mut() = app.to_string());
        let resident = WSTORES.with(|m| {
            m.borrow().get(app).map(|s| !s.cells.is_empty()).unwrap_or(false)
        });
        let mut out = String::from("{\"app\":");
        esc(app, &mut out);
        out.push_str(",\"resident\":");
        out.push_str(if resident { "true" } else { "false" });
        out.push('}');
        out
    }

    #[wasm_bindgen]
    pub fn arest_load(store_json: &str) -> String {
        ensure_base();
        match parse_json(store_json) {
            Some(payload) if jget(&payload, "d").is_some() => {
                with_active(|srv| handle(&payload, srv, true))
            }
            _ => "{\"error\":\"the payload needs a d\"}".to_string(),
        }
    }

    #[wasm_bindgen]
    pub fn arest_call(tool: &str, args_json: &str) -> String {
        ensure_base();
        let args = match parse_json(args_json) {
            Some(a) => a,
            None => return "{\"error\":\"unparseable args\"}".to_string(),
        };
        with_active(|srv| {
            match store_call(tool, &args, "worker", srv) {
                Some(Ok(r)) => r,
                Some(Err((code, msg))) => {
                    let mut out = String::from("{\"error\":");
                    esc(&msg, &mut out);
                    out.push_str(",\"code\":");
                    out.push_str(&code.to_string());
                    out.push('}');
                    out
                }
                None => "{\"error\":\"not a store-only verb\"}".to_string(),
            }
        })
    }

    #[wasm_bindgen]
    pub fn arest_view(noun: &str, id: &str) -> String {
        // the abstract UI's trees over the wire: the client transduces
        // (kind dispatch), the menu's buttons POST the event fact
        // types back — the >>= over HTTP
        ensure_base();
        with_active(|srv| match view_trees_json(noun, id, srv) {
            Ok(r) => r,
            Err((code, msg)) => {
                let mut out = String::from("{\"error\":");
                esc(&msg, &mut out);
                out.push_str(",\"code\":");
                out.push_str(&code.to_string());
                out.push('}');
                out
            }
        })
    }

    #[wasm_bindgen]
    pub fn arest_entry(noun: &str) -> String {
        // the create form's tree; submits POST one fact each
        ensure_base();
        with_active(|srv| match entry_tree_json(noun, srv) {
            Ok(r) => r,
            Err((code, msg)) => {
                let mut out = String::from("{\"error\":");
                esc(&msg, &mut out);
                out.push_str(",\"code\":");
                out.push_str(&code.to_string());
                out.push('}');
                out
            }
        })
    }

    #[wasm_bindgen]
    pub fn arest_ingest(entries_json: &str) -> String {
        // the bulk federated ingest: [{ft, facts:[[id, val], ...]}, ...]
        // — one pass over the V-side store, absorbed routing per
        // rmapColumns, pops unioned, mirror refreshed ONCE (coherence)
        ensure_base();
        let entries = match parse_json(entries_json) {
            Some(J::A(es)) => es,
            _ => return "{\"error\":\"entries must be an array\"}".to_string(),
        };
        with_active(|srv| {
            let mut cells: Vec<(String, V)> = cells_of(&srv.d)
                .into_iter()
                .filter_map(|(k, v)| leaf_str(&std::rc::Rc::new(k))
                    .map(|n| (n, v)))
                .collect();
            let find = |cells: &Vec<(String, V)>, name: &str| -> Option<usize> {
                cells.iter().position(|(k, _)| k == name)
            };
            // rmapColumns: (table, pos, ft) — the absorbed routing map
            let mut route: std::collections::HashMap<String, (String, i64)> =
                std::collections::HashMap::new();
            let mut widths: std::collections::HashMap<String, i64> =
                std::collections::HashMap::new();
            if let Some(i) = find(&cells, "rmapColumns") {
                for r in items(&list_of(&cells[i].1)) {
                    let it = items(&list_of(&r));
                    if it.len() >= 3 {
                        if let (Some(t), Some(pv), Some(f)) =
                            (aval(&it[0]), aval(&it[1]), aval(&it[2]))
                        {
                            if let (Leaf::S(ts), Leaf::I(pi), Leaf::S(fs)) =
                                (&*t, &*pv, &*f)
                            {
                                route.insert(fs.clone(), (ts.clone(), *pi));
                                let w = widths.entry(ts.clone()).or_insert(0);
                                if *pi > *w {
                                    *w = *pi;
                                }
                            }
                        }
                    }
                }
            }
            let sat = |s: &str| atom(Leaf::S(s.to_string()));
            let mut ingested = 0usize;
            for e in &entries {
                let ft = match jget(e, "ft") {
                    Some(J::S(f)) => f.clone(),
                    _ => continue,
                };
                let facts = match jget(e, "facts") {
                    Some(J::A(fs)) => fs,
                    _ => continue,
                };
                // union into the ft pop
                let mut pop: Vec<V> = match find(&cells, &ft) {
                    Some(i) => items(&list_of(&cells[i].1)),
                    None => Vec::new(),
                };
                let vkey = |v: &V| -> String {
                    let mut s = String::new();
                    write_v(v, &mut s);
                    s
                };
                let mut seen: std::collections::HashSet<String> =
                    pop.iter().map(|p| vkey(p)).collect();
                for f in facts {
                    let row: Vec<V> = match f {
                        J::A(cols) => cols
                            .iter()
                            .map(|c| match c {
                                J::S(s) => sat(s),
                                J::I(i) => atom(Leaf::I(*i)),
                                _ => sat(""),
                            })
                            .collect(),
                        _ => continue,
                    };
                    if row.is_empty() {
                        continue;
                    }
                    let rv = seq(from_vec(row.clone()));
                    let mut wrote = false;
                    let rk = vkey(&rv);
                    if !seen.contains(&rk) {
                        seen.insert(rk);
                        pop.push(rv);
                        wrote = true;
                    }
                    // absorbed routing: land on the table row + index
                    if let Some((table, pos)) = route.get(&ft) {
                        let width = *widths.get(table).unwrap_or(pos);
                        let key = match aval(&row[0]) {
                            Some(l) => match &*l {
                                Leaf::S(s) => s.clone(),
                                Leaf::I(i) => i.to_string(),
                                _ => continue,
                            },
                            None => continue,
                        };
                        let val = if row.len() >= 2 {
                            row[1].clone()
                        } else {
                            sat("T")
                        };
                        let rc = format!("{}:{}", table, key);
                        let mut rowv: Vec<V> = match find(&cells, &rc) {
                            Some(i) => items(&list_of(&cells[i].1)),
                            None => Vec::new(),
                        };
                        if rowv.is_empty() {
                            rowv.push(sat(&key));
                            while (rowv.len() as i64) < width {
                                rowv.push(sat("#"));
                            }
                        }
                        while (rowv.len() as i64) < width {
                            rowv.push(sat("#"));
                        }
                        let idx = (*pos as usize).saturating_sub(1);
                        if idx < rowv.len() {
                            rowv[idx] = val;
                        }
                        let rvv = seq(from_vec(rowv));
                        match find(&cells, &rc) {
                            Some(i) => cells[i].1 = rvv,
                            None => cells.push((rc, rvv)),
                        }
                        // the table index
                        let mut tbl: Vec<V> = match find(&cells, table) {
                            Some(i) => items(&list_of(&cells[i].1)),
                            None => Vec::new(),
                        };
                        let krow = seq(from_vec(vec![sat(&key)]));
                        if !tbl.iter().any(|t| {
                            let ti = items(&list_of(t));
                            !ti.is_empty()
                                && aval(&ti[0]).map(|l| matches!(&*l,
                                    Leaf::S(s) if *s == key))
                                    .unwrap_or(false)
                        }) {
                            tbl.push(krow);
                            let tv = seq(from_vec(tbl));
                            match find(&cells, table.as_str()) {
                                Some(i) => cells[i].1 = tv,
                                None => cells.push((table.clone(), tv)),
                            }
                        }
                    }
                    if wrote {
                        ingested += 1;
                    }
                }
                let pv = seq(from_vec(pop));
                match find(&cells, &ft) {
                    Some(i) => cells[i].1 = pv,
                    None => cells.push((ft, pv)),
                }
            }
            // rebuild D + the native mirror ONCE (the coherence pattern)
            let triples: Vec<V> = cells
                .into_iter()
                .map(|(k, v)| seq(from_vec(vec![
                    atom(Leaf::S("CELL".to_string())), atom(Leaf::S(k)), v,
                ])))
                .collect();
            srv.d = seq(from_vec(triples));
            srv.cells = cells_of(&srv.d);
            srv.nd = v_to_n(&srv.d);
            srv.ncells = n_cells_of(&srv.nd);
            format!("{{\"ingested\":{}}}", ingested)
        })
    }

    #[wasm_bindgen]
    pub fn arest_apply(args_json: &str) -> String {
        ensure_base();
        // the write path: apply_core evaluates and commits to the
        // RETAINED store; the COMMITTED EVENT rides the envelope for
        // the caller to append to its Durable Object stream — the
        // single-writer cell (Def. iso), worker storage edition.
        let args = match parse_json(args_json) {
            Some(a) => a,
            None => return "{\"error\":\"unparseable args\"}".to_string(),
        };
        with_active(|srv| {
            match apply_core(&args, "worker", srv) {
                Some(Ok((receipt, committed))) => {
                    let mut out = String::from("{\"receipt\":");
                    out.push_str(&receipt);
                    out.push_str(",\"event\":");
                    match committed {
                        Some((ft, fact)) => {
                            out.push_str("{\"ft\":");
                            esc(&ft, &mut out);
                            out.push_str(",\"fact\":[");
                            for (i, f) in fact.iter().enumerate() {
                                if i > 0 { out.push(','); }
                                match f {
                                    J::S(v) => esc(v, &mut out),
                                    J::I(n) => out.push_str(&n.to_string()),
                                    _ => out.push_str("null"),
                                }
                            }
                            out.push_str("]}");
                        }
                        None => out.push_str("null"),
                    }
                    out.push('}');
                    out
                }
                Some(Err((code, msg))) => {
                    let mut out = String::from("{\"error\":");
                    esc(&msg, &mut out);
                    out.push_str(",\"code\":");
                    out.push_str(&code.to_string());
                    out.push('}');
                    out
                }
                None => "{\"error\":\"this write needs the compiler host\"}"
                    .to_string(),
            }
        })
    }

    #[wasm_bindgen]
    pub fn arest_eval(f_json: &str, x_json: &str) -> String {
        ensure_base();
        let f = match parse_json(f_json) {
            Some(v) => j_to_n(&v),
            None => return "{\"error\":\"unparseable f\"}".to_string(),
        };
        let x = match parse_json(x_json) {
            Some(v) => j_to_n(&v),
            None => return "{\"error\":\"unparseable x\"}".to_string(),
        };
        let ev = NEval {
            cells: Vec::new(),
            process: Vec::new(),
            defs_n: N::Bot,
            fuel: std::cell::Cell::new(-1),
        };
        let res = n_to_v(&ev.mu(napp(f, x)));
        let mut out = String::from("{\"result\":");
        write_v(&res, &mut out);
        out.push('}');
        out
    }
}

// ============================ FastStore property tests (#35) ==================
// Stage A's own certification item 1 (docs/2026-07-11-store-twin-spec.md): random
// Store/setcell sequences must byte-equal the EXISTING store_into/setcell_into
// primitives' result over the identical sequence. Twin equivalence, not just
// unit behavior -- both codepaths run side by side over the same operations and
// the dumps are compared, so the test fails the instant the twin's observable
// contract drifts from the primitives it must remain indistinguishable from.
#[cfg(test)]
mod faststore_tests {
    use super::*;

    fn row(vals: &[&str]) -> V {
        seq(from_vec(vals.iter().map(|s| atom(Leaf::S((*s).to_string()))).collect()))
    }

    fn rows_of(rows: &[Vec<String>]) -> V {
        seq(from_vec(
            rows.iter()
                .map(|r| row(&r.iter().map(|s| s.as_str()).collect::<Vec<_>>()))
                .collect(),
        ))
    }

    fn dump(v: &V) -> String {
        let mut s = String::new();
        write_v(v, &mut s);
        s
    }

    fn seed_srv_from_d(d: V) -> Srv {
        let nd = v_to_n(&d);
        let cells = cells_of(&d);
        let ncells = n_cells_of(&nd);
        Srv { d, cells, mu: bot(), nd, ncells, nprocess: Vec::new() }
    }

    fn seed_srv(cells: Vec<(Leaf, V)>) -> Srv {
        seed_srv_from_d(cells_to_d(&cells))
    }

    fn cell_triple(name: &str, contents: V) -> V {
        seq(from_vec(vec![atom(Leaf::S("CELL".to_string())), atom(Leaf::S(name.to_string())), contents]))
    }

    // ---- directed: the four primitive behaviors the spec names verbatim ----

    #[test]
    fn store_on_absent_cell_is_a_fresh_prepend() {
        let srv = seed_srv(vec![(Leaf::S("Other".into()), rows_of(&[vec!["x".into()]]))]);
        let mut store = FastStore::from_srv(&srv);
        store.store(&Leaf::S("Fresh".into()), rows_of(&[vec!["a".into()], vec!["b".into()]]));
        assert_eq!(store.pop_rows(&Leaf::S("Fresh".into())).len(), 2);
        // re-topped: Fresh must be the FIRST physical cell now
        let all = store.to_all_cells();
        assert!(matches!(&all[0].0, Leaf::S(s) if s == "Fresh"));
    }

    #[test]
    fn store_on_present_cell_pops_the_top_and_prepends() {
        let srv = seed_srv(vec![
            (Leaf::S("A".into()), rows_of(&[vec!["old".into()]])),
            (Leaf::S("B".into()), rows_of(&[vec!["b".into()]])),
        ]);
        let mut store = FastStore::from_srv(&srv);
        store.store(&Leaf::S("A".into()), rows_of(&[vec!["new".into()]]));
        let got = store.pop_rows(&Leaf::S("A".into()));
        assert_eq!(got.len(), 1);
        assert_eq!(dump(&got[0]), dump(&row(&["new"])));
        let all = store.to_all_cells();
        assert!(matches!(&all[0].0, Leaf::S(s) if s == "A")); // re-topped to front
        assert_eq!(all.len(), 2); // old A content replaced, not appended as a 3rd cell
    }

    #[test]
    fn setcell_on_present_cell_replaces_in_place() {
        let srv = seed_srv(vec![
            (Leaf::S("A".into()), rows_of(&[vec!["old".into()]])),
            (Leaf::S("B".into()), rows_of(&[vec!["b".into()]])),
        ]);
        let mut store = FastStore::from_srv(&srv);
        store.setcell(&Leaf::S("A".into()), rows_of(&[vec!["new".into()]]));
        let all = store.to_all_cells();
        // position UNCHANGED (still first) -- setcell never re-tops
        assert!(matches!(&all[0].0, Leaf::S(s) if s == "A"));
        assert_eq!(dump(&all[0].1), dump(&rows_of(&[vec!["new".into()]])));
    }

    #[test]
    fn setcell_on_absent_cell_appends_at_the_end() {
        let srv = seed_srv(vec![(Leaf::S("A".into()), rows_of(&[vec!["a".into()]]))]);
        let mut store = FastStore::from_srv(&srv);
        store.setcell(&Leaf::S("Z".into()), rows_of(&[vec!["z".into()]]));
        let all = store.to_all_cells();
        assert_eq!(all.len(), 2);
        assert!(matches!(&all[1].0, Leaf::S(s) if s == "Z")); // appended LAST
    }

    // ---- directed: a pre-existing shadowed same-named cell survives, inert,
    // exactly as raw_cells_of's own comment describes and store_into's actual
    // pop-only-the-first-match behavior produces ----

    #[test]
    fn shadowed_same_named_cell_round_trips_untouched() {
        let d = seq(from_vec(vec![
            cell_triple("Foo", rows_of(&[vec!["top".into()]])),
            cell_triple("Foo", rows_of(&[vec!["shadow".into()]])),
            cell_triple("Bar", rows_of(&[vec!["only".into()]])),
        ]));
        let srv = seed_srv_from_d(d.clone());
        let store = FastStore::from_srv(&srv);
        // reads see only the top
        let got = store.pop_rows(&Leaf::S("Foo".into()));
        assert_eq!(got.len(), 1);
        assert_eq!(dump(&got[0]), dump(&row(&["top"])));
        // a pure entry+exit round trip must be byte-identical to the input --
        // nothing observed the shadow, but it must still be there on the way out
        let out_d = cells_to_d(&store.to_all_cells());
        assert_eq!(dump(&out_d), dump(&d));
    }

    #[test]
    fn shadowed_cell_survives_a_store_to_the_active_occurrence() {
        let d = seq(from_vec(vec![
            cell_triple("Foo", rows_of(&[vec!["top".into()]])),
            cell_triple("Foo", rows_of(&[vec!["shadow".into()]])),
            cell_triple("Bar", rows_of(&[vec!["only".into()]])),
        ]));
        // the existing primitive, run on the identical input/operation, is the
        // oracle for this test (not just an inspection of the twin alone)
        let mut d_prim = d.clone();
        let mut cells_prim = cells_of(&d_prim);
        let mut nd_prim = v_to_n(&d_prim);
        let mut ncells_prim = n_cells_of(&nd_prim);
        store_into(&mut d_prim, &mut cells_prim, &mut nd_prim, &mut ncells_prim,
                   &Leaf::S("Foo".into()), rows_of(&[vec!["new".into()]]));

        let srv = seed_srv_from_d(d);
        let mut store = FastStore::from_srv(&srv);
        store.store(&Leaf::S("Foo".into()), rows_of(&[vec!["new".into()]]));
        let out_d = cells_to_d(&store.to_all_cells());

        assert_eq!(dump(&out_d), dump(&d_prim));
        // and the shadow ("shadow") must appear exactly once in the dump text,
        // proving it rode through rather than being silently dropped
        assert_eq!(dump(&out_d).matches("shadow").count(), 1);
    }

    // ---- the random differential: many Store/setcell sequences, twin vs the
    // existing primitives, final dump compared byte for byte ----

    struct Rng(u64);
    impl Rng {
        fn next_u64(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next_u64() % n
        }
    }

    #[derive(Clone, Debug)]
    enum Op {
        Store(String, Vec<Vec<String>>),
        SetCell(String, Vec<Vec<String>>),
    }

    fn random_ops(rng: &mut Rng, names: &[&str], n: usize) -> Vec<Op> {
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let name = names[rng.below(names.len() as u64) as usize].to_string();
            let nrows = rng.below(4) as usize;
            let rows: Vec<Vec<String>> =
                (0..nrows).map(|_| vec![format!("v{}", rng.below(25))]).collect();
            if rng.below(2) == 0 {
                out.push(Op::Store(name, rows));
            } else {
                out.push(Op::SetCell(name, rows));
            }
        }
        out
    }

    fn run_primitives(mut srv: Srv, ops: &[Op]) -> Srv {
        for op in ops {
            match op {
                Op::Store(name, rows) => store_into(
                    &mut srv.d, &mut srv.cells, &mut srv.nd, &mut srv.ncells,
                    &Leaf::S(name.clone()), rows_of(rows),
                ),
                Op::SetCell(name, rows) => setcell_into(
                    &mut srv.d, &mut srv.cells, &mut srv.nd, &mut srv.ncells,
                    &Leaf::S(name.clone()), rows_of(rows),
                ),
            }
        }
        srv
    }

    fn run_faststore(srv: &Srv, ops: &[Op]) -> (V, Vec<(Leaf, V)>, Vec<(Leaf, N)>) {
        let mut store = FastStore::from_srv(srv);
        for op in ops {
            match op {
                Op::Store(name, rows) => store.store(&Leaf::S(name.clone()), rows_of(rows)),
                Op::SetCell(name, rows) => store.setcell(&Leaf::S(name.clone()), rows_of(rows)),
            }
        }
        let d = cells_to_d(&store.to_all_cells());
        let cells = store.to_active_cells();
        let ncells = (*store.ncells_native()).clone();
        (d, cells, ncells)
    }

    fn ncells_dump(ncells: &[(Leaf, N)]) -> String {
        // a stable text rendering for comparison: name then write_n(content)
        let mut out = String::new();
        for (name, n) in ncells {
            out.push_str(&leaf_text(name));
            out.push(':');
            write_n(n, &mut out);
            out.push(';');
        }
        out
    }

    #[test]
    fn random_store_setcell_sequences_match_primitives() {
        let names = ["Alpha", "Beta", "Gamma", "Delta", "Epsilon", "Zeta"];
        for seed in 0..80u64 {
            let mut rng = Rng((seed.wrapping_mul(2654435761)).wrapping_add(0x9e3779b97f4a7c15) | 1);
            let ops = random_ops(&mut rng, &names, 50);
            let seed_cells = initial_d_cells();

            let srv_prim = seed_srv(seed_cells.clone());
            let after_prim = run_primitives(srv_prim, &ops);
            let prim_dump = dump(&after_prim.d);
            let prim_cells_dump = dump(&cells_to_d(&after_prim.cells));
            let prim_ncells_dump = ncells_dump(&after_prim.ncells);

            let srv_twin = seed_srv(seed_cells);
            let (twin_d, twin_cells, twin_ncells) = run_faststore(&srv_twin, &ops);
            let twin_dump = dump(&twin_d);
            let twin_cells_dump = dump(&cells_to_d(&twin_cells));
            let twin_ncells_dump = ncells_dump(&twin_ncells);

            assert_eq!(prim_dump, twin_dump, "seed {}: d diverged over {:?}", seed, ops);
            assert_eq!(
                prim_cells_dump, twin_cells_dump,
                "seed {}: cells diverged over {:?}", seed, ops
            );
            assert_eq!(
                prim_ncells_dump, twin_ncells_dump,
                "seed {}: ncells diverged over {:?}", seed, ops
            );
        }
    }
}
