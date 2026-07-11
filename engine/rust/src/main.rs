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
        let post = match tok.find('-') {
            Some(p) => tok[p + 1..].to_string(),
            None => String::new(),
        };
        rows.push((tok, nopunct, base, subscript, lower, qtext, title, post, k > 0, k as i64));
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
const RESIDENT_OPS: [&str; 6] =
    ["cells", "compile_model", "query", "run_rules", "synthesize_pairs", "verbs"];

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
    let mut d = srv.d.clone();
    let mut cells = srv.cells.clone();
    // The native view of the current store, seeded from the resident mirror
    // (coherent by the write-site audit, 2026-07-08 — every store replacement
    // refreshes it, so the fresh v_to_n rebuild this seed used to pay is gone).
    // Every rule body evaluates through the native carrier NEval over this view
    // instead of the Scott mu, the measured fast path, and store_into keeps it
    // in lockstep with d/cells so a head is visible to later rules the instant
    // it is stored. The resident process defs carry the compiled canon; a
    // hand-built store without them falls through to NCANON in NEval.
    let mut nd = srv.nd.clone();
    let mut ncells = srv.ncells.clone();
    let nprocess = srv.nprocess.clone();
    let mut changed: BTreeSet<String> = BTreeSet::new();
    let leaf = |s: &str| Leaf::S(s.to_string());
    // reads: rule id key to the set of cell keys its body reads (ruleReads
    // rows are ⟨rule id, read cell⟩). The mirror blocks quantify over all
    // rules through it, round one intersects it with the frontier, and the
    // later rounds' full fallback intersects it with the delta.
    let mut reads: HashMap<String, HashSet<String>> = HashMap::new();
    for r in pop_rows(&cells, &leaf("ruleReads")) {
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
        for r in pop_rows(&cells, &leaf("instanceOf")) {
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
        for r in pop_rows(&cells, &leaf("role")) {
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
                        ft_rows = Some(pop_rows(&cells, &name));
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
        if !out.is_empty() && pop_rows(&cells, &leaf(MIRROR)).is_empty() {
            sort_rows(&mut out);
            store_into(&mut d, &mut cells, &mut nd, &mut ncells, &leaf(MIRROR), seq(from_vec(out)));
            changed.insert(MIRROR.to_string());
        }
    }
    // THE ROLE MIRROR: Fact_Type_has_Role derives from the role M-facts the
    // same way (the role facts ARE the knowledge); only the empty cell fills.
    const FTR: &str = "Fact_Type_has_Role";
    if any_reads(FTR) {
        let mut out: Vec<V> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for r in pop_rows(&cells, &leaf("role")) {
            let it = items(&list_of(&r));
            if it.len() >= 2 {
                let pair = seq(from_vec(vec![it[1].clone(), it[0].clone()]));
                if seen.insert(key_of(&pair)) {
                    out.push(pair);
                }
            }
        }
        if !out.is_empty() && pop_rows(&cells, &leaf(FTR)).is_empty() {
            sort_rows(&mut out);
            store_into(&mut d, &mut cells, &mut nd, &mut ncells, &leaf(FTR), seq(from_vec(out)));
            changed.insert(FTR.to_string());
        }
    }
    // atomsof: rule id key to its body atoms as ⟨position text, atom cell
    // key⟩ in ruleAtom row order; the stored delta variant for an atom rides
    // the DEFS cell named "<rule id>~d<position>", exactly the name Python
    // formats
    let mut atomsof: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for r in pop_rows(&cells, &leaf("ruleAtom")) {
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
    for r in pop_rows(&cells, &leaf("ruleAgg")) {
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
    for r in pop_rows(&cells, &leaf("ruleDerives")) {
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
            let full = |nd: &N, ncells: &Vec<(Leaf, N)>| -> Option<Vec<V>> {
                let res = neval_rule(ncells, &nprocess, nd, &rr.rid, nd.clone());
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
                    match full(&nd, &ncells) {
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
                            // on bottom, so the rows use N::S, not nseq.
                            let drows_n: Vec<N> = drows.iter().map(v_to_n).collect();
                            let operand =
                                N::S(Rc::new(vec![N::S(Rc::new(drows_n)), nd.clone()]));
                            let res = neval_rule(&ncells, &nprocess, &nd, &vid, operand);
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
                        match full(&nd, &ncells) {
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
                        match full(&nd, &ncells) {
                            Some(c) => c,
                            None => continue,
                        }
                    } else {
                        continue;
                    }
                }
            };
            let old = pop_rows(&cells, &rr.head);
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
                store_into(&mut d, &mut cells, &mut nd, &mut ncells, &rr.head, seq(from_vec(merged)));
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
    for r in pop_rows(&cells, &leaf("passHeads")) {
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
    for r in pop_rows(&cells, &leaf("spans")) {
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
    for c in pop_rows(&cells, &leaf("constraint")) {
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
    for r in pop_rows(&cells, &leaf("passOrder")) {
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
    for r in pop_rows(&cells, &leaf("passBound")) {
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
            let res = neval_rule(&ncells, &nprocess, &nd, &rr.rid, nd.clone());
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
            let before = pop_rows(&cells, &rr.head);
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
                    let res = neval_rule(&ncells, &nprocess, &nd, rid, nd.clone());
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
                store_into(&mut d, &mut cells, &mut nd, &mut ncells, &rr.head, seq(from_vec(merged)));
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
            let outs = eval_rules(&ncells, &nprocess, &nd, &rids);
            let mut prod_keys: HashSet<String> = HashSet::new();
            for r in &outs {
                prod_keys.insert(keyed_key(r, key_pos));
            }
            let stored = pop_rows(&cells, hl);
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
                store_into(&mut d, &mut cells, &mut nd, &mut ncells, hl, seq(from_vec(merged)));
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
            let outs = eval_rules(&ncells, &nprocess, &nd, rids);
            let stored = pop_rows(&cells, hl);
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
                store_into(&mut d, &mut cells, &mut nd, &mut ncells, hl, seq(from_vec(m)));
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
            let stored = pop_rows(&cells, hl);
            let mut cur_keys: HashSet<String> = HashSet::new();
            for r in &stored {
                cur_keys.insert(key_of(r));
            }
            store_into(&mut d, &mut cells, &mut nd, &mut ncells, hl, seq(from_vec(Vec::new())));
            let rids: Vec<Leaf> = plain_of.get(hk).cloned().unwrap_or_default();
            let mut prev: Option<HashSet<String>> = None;
            let mut outs_keys: HashSet<String> = HashSet::new();
            loop {
                if prev.as_ref() == Some(&outs_keys) {
                    break;
                }
                prev = Some(outs_keys.clone());
                let outs = eval_rules(&ncells, &nprocess, &nd, &rids);
                outs_keys = outs.iter().map(|r| key_of(r)).collect();
                let mut m = outs;
                sort_rows(&mut m);
                store_into(&mut d, &mut cells, &mut nd, &mut ncells, hl, seq(from_vec(m)));
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
        for r in pop_rows(&cells, &leaf("rmapColumns")) {
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
        if !layout.is_empty() {
            // unary heads write "T"; the role rows carry each head's arity
            let mut maxpos: HashMap<String, i64> = HashMap::new();
            for r in pop_rows(&cells, &leaf("role")) {
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
            for ftname in changed.iter() {
                let (table, col) = match layout.get(ftname) {
                    Some((t, c)) => (t.clone(), *c),
                    None => continue,
                };
                let width = *widths.get(&table).unwrap_or(&1);
                let unary = maxpos.get(ftname).copied() == Some(1);
                // want: key text → ⟨key atom, the value the column must carry⟩
                let mut want: HashMap<String, (V, V)> = HashMap::new();
                for r in pop_rows(&cells, &leaf(ftname)) {
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
                let mut tbl = pop_rows(&cells, &tleaf);
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
                    let mut row = pop_rows(&cells, &rc);
                    if row.is_empty() {
                        row = vec![atom(Leaf::S(kt.clone()))];
                    }
                    while row.len() < width {
                        row.push(hole());
                    }
                    if !eqobj(&row[col - 1], &v) {
                        row[col - 1] = v;
                        store_into(&mut d, &mut cells, &mut nd, &mut ncells, &rc,
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
                    let mut row = pop_rows(&cells, &rc);
                    if row.is_empty() {
                        row = vec![ka.clone()];
                    }
                    while row.len() < width {
                        row.push(hole());
                    }
                    row[col - 1] = v;
                    store_into(&mut d, &mut cells, &mut nd, &mut ncells, &rc,
                               seq(from_vec(row)));
                    tbl.push(seq(from_vec(vec![ka])));
                }
                if grew {
                    store_into(&mut d, &mut cells, &mut nd, &mut ncells, &tleaf,
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
    srv.d = d;
    srv.cells = cells;
    srv.nd = nd;
    srv.ncells = n_cells_of(&srv.nd);
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

// The _COOK boundary (compiler.py:1949, the #18 doctrine at system.canon:5504):
// Stage-1 text→X resolution the HOST performs before a translator body sees its
// groups (reading→ft-id via _clause_ft, name/ref-mode splits, constraint
// reading→ft resolution, value-spec parsing, the subtype/fact/rule cooks).
// Those cooks are PYTHON today; the native port lands with #18's Stage-1/handler
// boundary refactor. Mirrors `kind in _COOK` exactly: Err names the gate, Ok
// means the kind's groups pass through raw (the already-canonized handlers).
fn native_cook(kind: &str) -> Result<(), String> {
    match kind {
        "sm_trigger" | "sm_guard" | "ring" | "frequency" | "value_constraint"
        | "uniqueness" | "mandatory" | "neg_uniqueness" | "neg_mandatory"
        | "for_each_mandatory" | "inverse_uc" | "spanning_uc" | "spanning_uc2"
        | "negation" | "subtype_of" | "fact_type_reading" | "derivation_rule"
        | "class_rule" => Err(format!("cook not ported: {}", kind)),
        _ => Ok(()),
    }
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

// _reading (compiler.py:593): a fact-type reading → (template, roles) — the
// certified-equal host override of system:reading_parse
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
        if tok.contains('-') && !tok.ends_with('-') {
            // forward hyphen binding: adj-Type -> role Type
            let post = &tok[tok.find('-').unwrap() + 1..];
            if known.contains(post) {
                roles.push(post.to_string());
                out.push(format!("{{{}}}", roles.len() - 1));
                i += 1;
                continue;
            }
        }
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
                out.push(tok.to_string());
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
) {
    let leaf = |s: &str| Leaf::S(s.to_string());
    let strv = |x: &V| aval(x).and_then(|l| leaf_str(&l));
    let mut names: std::collections::HashSet<String> = std::collections::HashSet::new();
    for r in pop_rows(cells, &leaf("instanceOf")) {
        let it = items(&list_of(&r));
        if it.len() >= 2 {
            if let (Some(a), Some(b)) = (strv(&it[0]), strv(&it[1])) {
                if b == "ObjectType" || b == "ValueType" {
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
    (names, edges, fts)
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
    missing.push("model D starts EMPTY (meta.initial_D process seed not ported)".to_string());
    missing.push(
        "translator bodies are host closures until #18 canonizes them; native dispatch answers only for canon DEFs"
            .to_string(),
    );

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
    let (b_names, b_edges, b_fts) = match jget(j, "context_from") {
        Some(J::S(s)) if s == "resident" => context_of(&srv.cells),
        _ => (HashSet::new(), Vec::new(), HashSet::new()),
    };
    let mut names = known_names(&stmts);
    for n in b_names {
        names.insert(n);
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
    // TODO(#20): meta.initial_D — the skeleton's model store starts EMPTY
    let mut model_d: V = seq(from_vec(Vec::new()));
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
    let ctx = seqc(vec![
        seq(from_vec(names_sorted.iter().map(|n| atom_s(n)).collect())),
        seq(from_vec(subs_pairs)),
        seq(from_vec(fts.iter().map(|f| atom_s(f)).collect())),
        seq(from_vec(plain.iter().map(|f| atom_s(f)).collect())),
    ]);
    let empty_cls: HashSet<String> = HashSet::new();
    let mut unclassified: Vec<String> = Vec::new();
    let mut prose: Vec<String> = Vec::new();
    let mut blocked: Vec<String> = Vec::new();
    let mut classified = 0usize;
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
        if cls.contains("Prose") && residual.is_empty() {
            if sm_suspect(stmt) {
                unclassified.push(stmt.clone());
            } else {
                prose.push(stmt.clone());
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
            continue;
        }
        // deontic carries its operator sign through the modality field
        let mfield = if *m == "deontic" {
            format!("{}:{}", m, sg)
        } else {
            (*m).to_string()
        };
        let mut accepted = false;
        for t in &translators {
            // rho: dispatch through DEFS via the reducer — the direct analog
            // of python D = _apply(_A(t), operand). When #18 lands a canon
            // translator DEF this arm runs it for free; today a host-only
            // name reduces to ⊥ (or stays a stuck app) and the _COOK gate
            // below explains what blocks the native path.
            let operand = seqc(vec![
                atom(Leaf::S(inner.clone())),
                atom(Leaf::S(mfield.clone())),
                ctx.clone(),
                model_d.clone(),
            ]);
            let res = reduce_over(srv, atom(Leaf::S(t.clone())), operand, fuel);
            if matches!(shape(&res), Shape::Seq(_)) && !isapp(&res) {
                model_d = res;
                accepted = true;
            } else {
                for k in translator_kinds(t) {
                    if let Err(reason) = native_cook(k) {
                        if !blocked.contains(&reason) {
                            blocked.push(reason);
                        }
                    }
                }
                let miss = format!("translator not native: {}", t);
                if !blocked.contains(&miss) {
                    blocked.push(miss);
                }
            }
        }
        if !accepted {
            // NO translator accepted: reported loudly — never a silent vanish
            unclassified.push(stmt.clone());
        }
    }
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
    r.push('}');
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
fn write_sidecar(apps: &Apps, app: &str, srv: &Srv) -> std::io::Result<()> {
    let mut payload = String::from("{\"d\":");
    write_v(&srv.d, &mut payload);
    payload.push_str(",\"process\":[");
    for (i, (name, obj)) in srv.nprocess.iter().enumerate() {
        if i > 0 {
            payload.push(',');
        }
        payload.push('[');
        esc(name, &mut payload);
        payload.push(',');
        write_n(obj, &mut payload);
        payload.push(']');
    }
    payload.push_str("],\"overrides\":1,\"cases\":[]}");
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
        let rn = ev.mu(napp(
            v_to_n(&handler),
            nseq(vec![v_to_n(&fact_v), srv.nd.clone()]),
        ));
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
