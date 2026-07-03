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
                (Some(a), Some(b)) => match (&*a, &*b) {
                    (Leaf::I(x), Leaf::I(y)) => atom(Leaf::I(f_i(*x, *y))),
                    _ => match (num(&a), num(&b)) {
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
                (Some(a), Some(b)) => match (&*a, &*b) {
                    (Leaf::S(x), Leaf::S(y)) => bool2a(rel_s(x, y)),
                    _ => match (num(&a), num(&b)) {
                        (Some(x), Some(y)) => bool2a(rel_n(x, y)),
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
                    return self.mu(r);                       // the native base
                }
                if let Some(k) = leaf_key(lc) {
                    if let Some((_, obj)) = self.process.iter().rev().find(|(n, _)| *n == k) {
                        return self.mu(napp(obj.clone(), x));
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
        let intv = |n: &N| -> Option<i64> {
            if let N::A(l) = n { if let Leaf::I(i) = &**l { Some(*i) } else { None } } else { None }
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
                Some((a, b)) => match (intv(&a), intv(&b)) {
                    (Some(p), Some(q)) => N::A(Rc::new(Leaf::I(match s { "+" => p + q, "-" => p - q, _ => p * q }))),
                    _ => match (numv(&a), numv(&b)) {
                        (Some(p), Some(q)) => N::A(Rc::new(Leaf::F(match s { "+" => p + q, "-" => p - q, _ => p * q }))),
                        _ => N::Bot,
                    },
                },
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
                Some((N::A(a), N::A(b))) => match (&*a, &*b) {
                    (Leaf::S(p), Leaf::S(q)) => nb(match s { "ge" => p >= q, "gt" => p > q, "le" => p <= q, _ => p < q }),
                    _ => match (num(&a), num(&b)) {
                        (Some(p), Some(q)) => nb(match s { "ge" => p >= q, "gt" => p > q, "le" => p <= q, _ => p < q }),
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
        J::O(_) => N::Bot,
    }
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

// ============================ JSON (hand-rolled, zero deps) ==================
#[derive(Debug, Clone)]
enum J {
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
        J::O(_) => bot(),
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

fn run() {
    // the intersection source loads ON THE WORKER (CANON is a thread_local; the
    // reduction thread must hold it)
    CANON.with(|c| {
        *c.borrow_mut() = canon_defs();
    });
    register_base();
    register_overrides();                                     // twins on by default
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
// double-quoted strings, no imports, no assignments, PHI referenced once per file
// (it is a moved local).
#[allow(non_snake_case, unused, path_statements)]
fn canon_defs() -> Vec<(String, V)> {
    let out: RefCell<Vec<(String, V)>> = RefCell::new(Vec::new());
    {
        let DEF = |n: &str, o: V| out.borrow_mut().push((n.to_string(), o));
        let A = |s: &str| atom(Leaf::S(s.to_string()));
        let N = |i: i64| atom(Leaf::I(i));
        let PHI = phi();
        let S2 = |a: V, b: V| seqv(vec![a, b]);
        let S3 = |a: V, b: V, c: V| seqv(vec![a, b, c]);
        let S4 = |a: V, b: V, c: V, d: V| seqv(vec![a, b, c, d]);
        let S5 = |a: V, b: V, c: V, d: V, e: V| seqv(vec![a, b, c, d, e]);
        let S6 = |a: V, b: V, c: V, d: V, e: V, f: V| seqv(vec![a, b, c, d, e, f]);
        let S7 = |a: V, b: V, c: V, d: V, e: V, f: V, g: V| seqv(vec![a, b, c, d, e, f, g]);
        let S8 = |a: V, b: V, c: V, d: V, e: V, f: V, g: V, h: V| seqv(vec![a, b, c, d, e, f, g, h]);
        let S9 = |a: V, b: V, c: V, d: V, e: V, f: V, g: V, h: V, i: V| seqv(vec![a, b, c, d, e, f, g, h, i]);
        include!("../../shared/theta.py");
    }
    out.into_inner()
}

fn main() {
    // mu recurses deeply through closures; give it room
    std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(run)
        .unwrap()
        .join()
        .unwrap();
}
