// crates/arest/src/wasm_lower.rs
//
// Lower Func trees to WASM functions (prototype).
//
// Don't reinvent the VM: we already run inside a WASM VM (V8 in
// Workers, wasmer/wasmtime on server). Compile each Func to a WASM
// function and dispatch via the host VM — no AREST-level interpreter
// on the hot path.
//
// The emitted module exports two things:
//   - `apply` : function of type (i64) -> (i32)
//   - `memory` : the linear memory holding allocated Objects
//
// The caller passes an i64 scalar input; the emitted body boxes it
// into an Atom, runs the lowered Func, and returns a pointer to the
// result Object. Callers dereference the pointer by reading bytes
// from `memory`.
//
// Supported variants:
//   Primitives                : Id
//   Literal                   : Constant(Object::Atom) (i64-parseable)
//   Combining forms (§11.2.4) : Compose, Condition, Construction,
//                               ApplyToAll, Filter, Insert, While
//   Structural (§11.2.3)      : Selector, Tail, Reverse, RotL, RotR
//                               ApndL, ApndR, Concat,
//                               DistL, DistR, Trans
//   Arithmetic (§11.2.3)      : Add, Sub, Mul, Div  (÷ protects
//                               against divide-by-zero → φ)
//   Comparison (§11.2.3)      : Eq, Gt, Lt, Ge, Le  (signed i64)
//   Logic (§11.2.3)           : And, Or, Not
//   Predicates (§11.2.3)      : AtomTest, NullTest, Length
//
// Intentionally NOT supported (out of scope for this PoC):
//   - Fetch / FetchOrPhi / Store : need access to D (runtime state).
//     The PoC's pure `(i64) → i32` contract has no way to plumb D
//     through; a production lowering would import a cell-access
//     host function.
//   - Def(name) / Platform(name) / Native : dispatch through DEFS.
//     Same issue — these resolve names in D at runtime.
//   - Contains / Lower                   : need string Atom layout
//     (length + UTF-8 bytes). Every other variant works on i64
//     atoms + Seqs of pointers, avoiding the string-width question.
//   - Constant(Object::Seq / Map)        : literal Seqs/Maps would
//     need data-section layout; currently only i64 atoms literally.
//
// Memory layout (absolute offsets):
//   0 .. HEAP_START : reserved (unused sentinels)
//   HEAP_START ..   : bump-allocated heap
//
// Object header (first 4 bytes at every object ptr):
//   tag = 0 → Atom : [u32 tag] [4B pad] [i64 value]              (16 B)
//   tag = 1 → Seq  : [u32 tag] [u32 length] [i32 elem ptr × n]   (8 + 4n B)
//   phi           → represented as pointer 0; no allocation.
//
// Calling convention inside the body: each `emit_body` case
// consumes one i32 Object pointer from the stack and leaves one
// i32 Object pointer on the stack. `lower_to_wasm` pushes the
// boxed input pointer once.
//
// Heap lifetime: `apply` resets the heap pointer at entry, so every
// invocation computes on a fresh bump allocator. The returned pointer
// is valid only until the next call. Callers snapshot or copy before
// re-invoking.

#![cfg(feature = "wasm-lower")]

use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, EntityType, ExportKind, ExportSection, Function,
    FunctionSection, GlobalSection, GlobalType, ImportSection, Instruction, MemArg,
    MemorySection, MemoryType, Module, TypeSection, ValType,
};

use crate::ast::{Func, Object};

// Heap starts 16 bytes in; the prefix is reserved for future sentinels.
const HEAP_START: i32 = 16;

// Object tags.
const TAG_ATOM: i32 = 0;        // i64 atom        : [tag=0, pad, i64 value]
const TAG_SEQ: i32 = 1;         // seq of Objects  : [tag=1, length, i32 elem ptr × n]
const TAG_STRING_ATOM: i32 = 2; // string atom     : [tag=2, byte_length, n bytes]

// Object sizes.
const ATOM_SIZE: i32 = 16;      // 4B tag + 4B pad + 8B value
const SEQ_HEADER_SIZE: i32 = 8; // 4B tag + 4B length
// StringAtom header: 4B tag + 4B byte length. Payload follows at offset 8.

// Host imports: the runtime functions the module pulls in from the
// embedder so that D-access primitives (Fetch, Store, Def, Platform)
// have somewhere to dispatch. Imports occupy the first N slots of
// the function index space; regular function indices shift past
// them. Embedders bind the imports via `install_host_imports`
// (below) or an equivalent linker call.
const IMPORT_CELL_FETCH:       u32 = 0; // (name_ptr)          → value_ptr
const IMPORT_CELL_FETCH_OR_PHI:u32 = 1; // (name_ptr)          → value_ptr | phi
const IMPORT_CELL_STORE:       u32 = 2; // (name_ptr, val_ptr) → new_state_ptr
const IMPORT_DEF_DISPATCH:     u32 = 3; // (name_ptr, x_ptr)   → result_ptr
const IMPORT_PLATFORM_DISPATCH:u32 = 4; // (name_ptr, x_ptr)   → result_ptr
const NUM_IMPORTS: u32 = 5;

// Emitted-function indices. `apply` comes first among the
// module-defined functions (so the export still resolves to its
// natural position) but sits AT offset NUM_IMPORTS in the function
// namespace because all imports are numbered before locally defined
// functions in WebAssembly.
const FN_APPLY:             u32 = NUM_IMPORTS;
const FN_ALLOC:             u32 = NUM_IMPORTS + 1;
const FN_ALLOC_ATOM:        u32 = NUM_IMPORTS + 2;
const FN_TRUTHY:            u32 = NUM_IMPORTS + 3;
const FN_ALLOC_STRING_ATOM: u32 = NUM_IMPORTS + 4;

// Global index: only one.
const G_HEAP_PTR: u32 = 0;

// MemArg helpers. All accesses are on memory 0 (the only memory we
// declare); alignments match natural sizes (2 = 4B, 3 = 8B).
const fn i32_at(offset: u64) -> MemArg {
    MemArg { offset, align: 2, memory_index: 0 }
}
const fn i64_at(offset: u64) -> MemArg {
    MemArg { offset, align: 3, memory_index: 0 }
}

/// Lower a Func tree to a valid WASM module.
///
/// Returns `Ok(bytes)` with the module on success, or `Err(msg)`
/// describing which variant is not yet supported. Callers wrap the
/// bytes in `WebAssembly.Module` (V8) or `wasmi::Module::new`
/// (interpreter) to instantiate and invoke.
pub fn lower_to_wasm(func: &Func) -> Result<Vec<u8>, String> {
    let mut module = Module::new();

    // ── Types ─────────────────────────────────────────────────────
    // 0: (i64) → (i32)           — apply, alloc_atom
    // 1: (i32) → (i32)           — alloc, truthy, alloc_string_atom,
    //                              cell_fetch, cell_fetch_or_phi
    // 2: (i32, i32) → (i32)       — cell_store, def_dispatch,
    //                              platform_dispatch
    let mut types = TypeSection::new();
    types.ty().function(vec![ValType::I64], vec![ValType::I32]);                    // 0
    types.ty().function(vec![ValType::I32], vec![ValType::I32]);                    // 1
    types.ty().function(vec![ValType::I32, ValType::I32], vec![ValType::I32]);      // 2
    module.section(&types);

    // ── Imports ───────────────────────────────────────────────────
    // Five host functions live in the "arest" namespace. Each reads
    // pointers to Objects in linear memory (name, argument),
    // dispatches on the host side, and returns a pointer (allocated
    // via the module's exported alloc helpers) to the result Object.
    //
    // cell_fetch / cell_fetch_or_phi: read-only access to D.
    //   cell_fetch returns φ (null ptr) when the cell is absent;
    //   cell_fetch_or_phi returns an empty-Seq ptr — the convention
    //   used by AREST's eval context to avoid ⊥-propagation through
    //   Construction on missing FT cells.
    //
    // cell_store: name + value → new D representation. Returns a
    //   placeholder; the host updates its externally-held D and the
    //   WASM program continues with its own snapshot.
    //
    // def_dispatch / platform_dispatch: higher-order — name a Def
    //   or Platform primitive and apply it to x. The host runs
    //   `ast::apply(Func::Def(name), x, &d)` (or Platform) and
    //   serialises the result back into WASM memory.
    let mut imports = ImportSection::new();
    imports.import("arest", "cell_fetch",        EntityType::Function(1));
    imports.import("arest", "cell_fetch_or_phi", EntityType::Function(1));
    imports.import("arest", "cell_store",        EntityType::Function(2));
    imports.import("arest", "def_dispatch",      EntityType::Function(2));
    imports.import("arest", "platform_dispatch", EntityType::Function(2));
    module.section(&imports);

    // ── Functions ─────────────────────────────────────────────────
    // Each `functions.function(N)` says "the next module-defined
    // function has type N". Their function indices in the final
    // namespace start at NUM_IMPORTS (= 5).
    let mut functions = FunctionSection::new();
    functions.function(0); // apply             (type 0)
    functions.function(1); // alloc             (type 1)
    functions.function(0); // alloc_atom        (type 0)
    functions.function(1); // truthy            (type 1)
    functions.function(1); // alloc_string_atom (type 1)
    module.section(&functions);

    // ── Memory ────────────────────────────────────────────────────
    // One page (64 KB) initial. A single `apply` invocation that
    // allocates more than 64 KB will trap — for the PoC that means
    // "don't build megabyte-scale Seqs in one call". Memory.grow
    // is a straightforward follow-up.
    let mut memory = MemorySection::new();
    memory.memory(MemoryType {
        minimum: 1,
        maximum: None,
        memory64: false,
        shared: false,
        page_size_log2: None,
    });
    module.section(&memory);

    // ── Globals ───────────────────────────────────────────────────
    // heap_ptr, mutable i32, initialized to HEAP_START.
    let mut globals = GlobalSection::new();
    globals.global(
        GlobalType { val_type: ValType::I32, mutable: true, shared: false },
        &ConstExpr::i32_const(HEAP_START),
    );
    module.section(&globals);

    // ── Exports ───────────────────────────────────────────────────
    let mut exports = ExportSection::new();
    exports.export("apply", ExportKind::Func, FN_APPLY);
    // The host calls these when encoding an Object back into the
    // module's linear memory (so imports like cell_fetch can return
    // a freshly-built value without the host reinventing the
    // allocator).
    exports.export("alloc", ExportKind::Func, FN_ALLOC);
    exports.export("alloc_atom", ExportKind::Func, FN_ALLOC_ATOM);
    exports.export("alloc_string_atom", ExportKind::Func, FN_ALLOC_STRING_ATOM);
    exports.export("memory", ExportKind::Memory, 0);
    module.section(&exports);

    // ── Code ──────────────────────────────────────────────────────
    let mut codes = CodeSection::new();

    // --- apply(i64) -> i32 ---
    //
    // Local layout:
    //   local 0     : i64 input (the parameter)
    //   local 1     : i32 input_ptr (boxed atom of input)
    //   local 2 ... : i32 scratches used by Condition/Construction
    //
    // Entry resets the heap so each invocation gets a fresh bump.
    // The returned pointer is therefore valid only until the next
    // call; callers snapshot before re-invoking.
    // Local layout (declared in declaration order):
    //   local 0            : i64 input (parameter)
    //   local 1            : i32 input_ptr
    //   locals 2..2+scratch: i32 scratches (Condition/Construction/…)
    //   local 2+scratch    : i64 scratch for Div's zero-check stash
    //                        (always declared; costs ~1 byte if unused)
    let scratch = scratch_needed(func);
    let div_i64_slot: u32 = 2 + scratch;
    let mut apply_locals: Vec<(u32, ValType)> = vec![(1, ValType::I32)];
    if scratch > 0 {
        apply_locals.push((scratch, ValType::I32));
    }
    apply_locals.push((1, ValType::I64));
    let mut apply_body = Function::new(apply_locals);
    // heap_ptr = HEAP_START
    apply_body.instruction(&Instruction::I32Const(HEAP_START));
    apply_body.instruction(&Instruction::GlobalSet(G_HEAP_PTR));
    // input_ptr = alloc_atom(input)
    apply_body.instruction(&Instruction::LocalGet(0));
    apply_body.instruction(&Instruction::Call(FN_ALLOC_ATOM));
    apply_body.instruction(&Instruction::LocalSet(1));
    // Seed the stack with input_ptr and emit the lowered body.
    apply_body.instruction(&Instruction::LocalGet(1));
    emit_body(func, &mut apply_body, 2, div_i64_slot)?;
    apply_body.instruction(&Instruction::End);
    codes.function(&apply_body);

    // --- alloc(size: i32) -> i32 ---
    //
    // Bump allocator: return old heap_ptr; advance heap_ptr by size.
    // No bounds check — a request larger than remaining memory traps
    // on the first write that goes out of range. PoC-level.
    let mut alloc_body = Function::new([]);
    alloc_body.instruction(&Instruction::GlobalGet(G_HEAP_PTR));  // stack: [old_ptr]
    alloc_body.instruction(&Instruction::GlobalGet(G_HEAP_PTR));
    alloc_body.instruction(&Instruction::LocalGet(0));
    alloc_body.instruction(&Instruction::I32Add);
    alloc_body.instruction(&Instruction::GlobalSet(G_HEAP_PTR));  // heap_ptr += size
    alloc_body.instruction(&Instruction::End);                     // returns old_ptr
    codes.function(&alloc_body);

    // --- alloc_atom(value: i64) -> i32 ---
    //
    // Reserve 16 bytes, write the Atom tag + value, return the ptr.
    let mut alloc_atom_body = Function::new([(1, ValType::I32)]);  // local 1: ptr
    alloc_atom_body.instruction(&Instruction::I32Const(ATOM_SIZE));
    alloc_atom_body.instruction(&Instruction::Call(FN_ALLOC));
    alloc_atom_body.instruction(&Instruction::LocalTee(1));
    alloc_atom_body.instruction(&Instruction::I32Const(TAG_ATOM));
    alloc_atom_body.instruction(&Instruction::I32Store(i32_at(0)));
    alloc_atom_body.instruction(&Instruction::LocalGet(1));
    alloc_atom_body.instruction(&Instruction::LocalGet(0));
    alloc_atom_body.instruction(&Instruction::I64Store(i64_at(8)));
    alloc_atom_body.instruction(&Instruction::LocalGet(1));
    alloc_atom_body.instruction(&Instruction::End);
    codes.function(&alloc_atom_body);

    // --- truthy(ptr: i32) -> i32 ---
    //
    // AREST Object truthiness:
    //   ptr == 0 (phi)  → 0
    //   Atom (tag 0)    → i64 value != 0
    //   Seq (tag 1)     → length != 0
    //   StringAtom      → byte_length != 0  (falls through the "else"
    //     (tag 2)         arm — the length field is at offset 4 for
    //                     both Seq and StringAtom, so a single
    //                     `i32.load offset=4` serves both)
    let mut truthy_body = Function::new([]);
    truthy_body.instruction(&Instruction::LocalGet(0));
    truthy_body.instruction(&Instruction::I32Eqz);
    truthy_body.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
    truthy_body.instruction(&Instruction::I32Const(0));
    truthy_body.instruction(&Instruction::Else);
    truthy_body.instruction(&Instruction::LocalGet(0));
    truthy_body.instruction(&Instruction::I32Load(i32_at(0)));
    truthy_body.instruction(&Instruction::I32Const(TAG_ATOM));
    truthy_body.instruction(&Instruction::I32Eq);
    truthy_body.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
    // Atom branch: load i64 value at offset 8, test != 0.
    truthy_body.instruction(&Instruction::LocalGet(0));
    truthy_body.instruction(&Instruction::I64Load(i64_at(8)));
    truthy_body.instruction(&Instruction::I64Const(0));
    truthy_body.instruction(&Instruction::I64Ne);
    truthy_body.instruction(&Instruction::Else);
    // Seq branch: load u32 length at offset 4, test != 0.
    truthy_body.instruction(&Instruction::LocalGet(0));
    truthy_body.instruction(&Instruction::I32Load(i32_at(4)));
    truthy_body.instruction(&Instruction::I32Const(0));
    truthy_body.instruction(&Instruction::I32Ne);
    truthy_body.instruction(&Instruction::End);  // inner if
    truthy_body.instruction(&Instruction::End);  // outer if
    truthy_body.instruction(&Instruction::End);  // function
    codes.function(&truthy_body);

    // --- alloc_string_atom(byte_length: i32) -> i32 ---
    //
    // Reserve space for a StringAtom: 4B tag + 4B length + n bytes,
    // rounded up to a 4-byte multiple so the next bump-alloc starts
    // at an aligned address (i32 loads from the heap assume 4-byte
    // alignment; wasmi tolerates unaligned but trapping runtimes
    // wouldn't). Writes tag=2 and length; the caller fills the n
    // payload bytes via i32.store8 at offset 8+i.
    let mut alloc_string_atom_body = Function::new([(1, ValType::I32)]);
    // aligned_size = (8 + length + 3) & -4
    alloc_string_atom_body.instruction(&Instruction::I32Const(8 + 3));
    alloc_string_atom_body.instruction(&Instruction::LocalGet(0));
    alloc_string_atom_body.instruction(&Instruction::I32Add);
    alloc_string_atom_body.instruction(&Instruction::I32Const(-4));
    alloc_string_atom_body.instruction(&Instruction::I32And);
    alloc_string_atom_body.instruction(&Instruction::Call(FN_ALLOC));
    alloc_string_atom_body.instruction(&Instruction::LocalTee(1));
    // Write tag at offset 0.
    alloc_string_atom_body.instruction(&Instruction::I32Const(TAG_STRING_ATOM));
    alloc_string_atom_body.instruction(&Instruction::I32Store(i32_at(0)));
    // Write byte length at offset 4.
    alloc_string_atom_body.instruction(&Instruction::LocalGet(1));
    alloc_string_atom_body.instruction(&Instruction::LocalGet(0));
    alloc_string_atom_body.instruction(&Instruction::I32Store(i32_at(4)));
    // Return ptr.
    alloc_string_atom_body.instruction(&Instruction::LocalGet(1));
    alloc_string_atom_body.instruction(&Instruction::End);
    codes.function(&alloc_string_atom_body);

    module.section(&codes);

    Ok(module.finish())
}

/// Count the max number of simultaneously-live scratch i32 locals the
/// body needs. Locals must be declared up front, so we do a pre-walk.
///
/// Sibling subterms (Compose's f/g; Condition's p/f/g; Construction's
/// children) run at disjoint times and share scratch slots — we take
/// the *max* of their needs, not the sum.
fn scratch_needed(func: &Func) -> u32 {
    match func {
        // Numeric literals take no scratch; string literals stash
        // the freshly-allocated StringAtom pointer while we write
        // bytes into its payload.
        Func::Constant(Object::Atom(s)) => {
            if s.parse::<i64>().is_ok() { 0 } else { 1 }
        }
        Func::Compose(f, g) => scratch_needed(f).max(scratch_needed(g)),
        Func::Condition(p, f, g) => {
            1 + scratch_needed(p)
                .max(scratch_needed(f))
                .max(scratch_needed(g))
        }
        Func::Construction(children) => {
            // Construction holds x in one slot and the in-progress
            // Seq ptr in another. Children evaluate one at a time,
            // each reusing the same "after-seq" slot.
            2 + children.iter().map(scratch_needed).max().unwrap_or(0)
        }
        Func::ApplyToAll(f) => {
            // ApplyToAll needs four slots simultaneously across its
            // loop: index i, length, input seq ptr, output seq ptr.
            // The child f sees next_scratch + 4 as its first free
            // slot.
            4 + scratch_needed(f)
        }
        Func::Filter(p) => {
            // Filter needs six slots: index, length, input seq ptr,
            // output seq ptr, kept count, and a stash for the current
            // element (since p consumes it but we also need it to
            // store into the output on the truthy branch).
            6 + scratch_needed(p)
        }
        Func::Insert(f) => {
            // Insert needs five slots: index, length, input seq ptr,
            // accumulator (the running fold result), and a temporary
            // pair ptr built fresh each iteration for the binary f
            // to consume.
            5 + scratch_needed(f)
        }
        // Binary arithmetic consumes a pair Seq and needs one i32
        // scratch for pair_ptr. Div additionally uses the
        // function-level i64 slot (allocated unconditionally by
        // lower_to_wasm) to stash the divisor across the zero-check
        // branch — it doesn't count toward the i32 scratch budget.
        Func::Add | Func::Sub | Func::Mul | Func::Div => 1,
        // Comparisons share the arithmetic unpack: one pair_slot,
        // both operands on the WASM stack, i64 compare, extend to
        // i64, alloc_atom.
        Func::Eq | Func::Gt | Func::Lt | Func::Ge | Func::Le => 1,
        // Logic: And/Or are pair-unary-truthy → i32.and/or;
        // Not is unary-truthy → i32.eqz. Only And/Or need pair_slot.
        Func::And | Func::Or => 1,
        Func::Not => 0,
        // Structural predicates need one scratch to stash the ptr
        // across the null-guard branch (dereferencing a null ptr
        // would load sentinel bytes from memory[0..3], silently
        // misclassifying φ as Atom).
        Func::AtomTest | Func::NullTest | Func::Length => 1,
        // Contains: pair → bool atom. Byte-level substring search
        // with nested loops. Nine slots: pair, haystack, needle,
        // haystack_len, needle_len, i (outer), j (inner), match,
        // and result.
        Func::Contains => 9,
        // Lower: unary string → new string. Five slots: src, src_len,
        // dst, i (loop index), b (per-byte scratch for the branchless
        // ASCII-case fold).
        Func::Lower => 5,
        // Fetch / FetchOrPhi: single host call consuming the name
        // atom on the stack and leaving the value atom. Zero scratch.
        Func::Fetch | Func::FetchOrPhi => 0,
        // Store: pair input → host call. One slot for the pair ptr.
        Func::Store => 1,
        // Def(name) / Platform(name): two slots (x stash + name
        // StringAtom ptr). The name string is emitted inline byte by
        // byte into its fresh atom.
        Func::Def(_) | Func::Platform(_) => 2,
        // Unary Seq transformers allocate a new Seq of derived
        // length and copy elements with an index mapping. Five slots:
        // i, in_len, in_seq, out_seq, out_len.
        Func::Tail | Func::Reverse | Func::RotL | Func::RotR => 5,
        // ApndL/ApndR: pair in, Seq out of length inner.length + 1.
        // Five slots: pair, inner, inner_len, out, i.
        Func::ApndL | Func::ApndR => 5,
        // Concat: Seq-of-Seqs flatten. Two passes — first to sum
        // total length, then to copy. Nine slots: outer, outer_len,
        // i, total_len, out, out_pos, inner, inner_len, j.
        Func::Concat => 9,
        // Distribution: pair in, Seq-of-pairs out. Each output
        // element is itself an allocated 2-elem Seq. Seven slots:
        // pair, inner, inner_len, out, i, pair_i (scratch pair),
        // scalar (stashed head for DistL or tail for DistR).
        Func::DistL | Func::DistR => 7,
        // Trans: Seq-of-Seqs transpose. Seven slots: outer, outer_len,
        // inner_len, out, i (output row), pair_i (scratch per-row),
        // j (input-row iterator).
        Func::Trans => 7,
        // While: two function-level slots (acc, counter) plus whatever
        // the pred and body subterms need. Pred and body alternate —
        // they can share the slot range, so only take the max.
        Func::While(p, f) => 2 + scratch_needed(p).max(scratch_needed(f)),
        _ => 0,
    }
}

/// Emit a binary i64 arithmetic op (Add, Sub, Mul). Consumes the
/// pair Seq ptr on the stack, loads both Atom i64 values onto the
/// operand stack in order (a then b), invokes `op`, and wraps the
/// result in a fresh Atom. No i64 local needed — operands stay on
/// the WASM operand stack between the two loads.
fn emit_binary_i64_arith(body: &mut Function, pair_slot: u32, op: Instruction<'static>) {
    body.instruction(&Instruction::LocalSet(pair_slot));
    // a = pair[0].value
    body.instruction(&Instruction::LocalGet(pair_slot));
    body.instruction(&Instruction::I32Load(i32_at(8)));
    body.instruction(&Instruction::I64Load(i64_at(8)));
    // b = pair[1].value
    body.instruction(&Instruction::LocalGet(pair_slot));
    body.instruction(&Instruction::I32Load(i32_at(12)));
    body.instruction(&Instruction::I64Load(i64_at(8)));
    // stack: [a, b] — apply op.
    body.instruction(&op);
    body.instruction(&Instruction::Call(FN_ALLOC_ATOM));
}

/// Emit a binary i64 comparison (Eq, Gt, Lt, Ge, Le). Consumes the
/// pair Seq, loads both operands, applies the i64 compare (which
/// returns i32 0/1), zero-extends to i64, and wraps as an Atom so
/// the result can flow into Compose/Condition/ApplyToAll like any
/// other Object.
fn emit_binary_i64_compare(body: &mut Function, pair_slot: u32, cmp: Instruction<'static>) {
    body.instruction(&Instruction::LocalSet(pair_slot));
    body.instruction(&Instruction::LocalGet(pair_slot));
    body.instruction(&Instruction::I32Load(i32_at(8)));
    body.instruction(&Instruction::I64Load(i64_at(8)));
    body.instruction(&Instruction::LocalGet(pair_slot));
    body.instruction(&Instruction::I32Load(i32_at(12)));
    body.instruction(&Instruction::I64Load(i64_at(8)));
    body.instruction(&cmp);
    body.instruction(&Instruction::I64ExtendI32U);
    body.instruction(&Instruction::Call(FN_ALLOC_ATOM));
}

/// Emit a unary Seq → Seq transformer whose output length is a
/// function of the input length and whose element i is a function
/// of (i, in_len). Used by Tail, Reverse, RotL, RotR — they share
/// 95 % of their emission, differing only in:
///
///   - `compute_out_len(body, in_len_slot)` : leaves the output
///     Seq's length on the operand stack.
///   - `compute_src_idx(body, i_slot, in_len_slot)` : leaves the
///     source-element index on the stack.
///
/// Null-pointer input is handled gracefully by returning an empty
/// Seq; this keeps the PoC usable with φ-producing subterms
/// (Insert on empty, Filter that keeps nothing) feeding into
/// subsequent Seq ops.
///
/// Scratch usage: 5 slots (i, in_len, in_seq, out_seq, out_len).
fn emit_unary_seq_map(
    body: &mut Function,
    next_scratch: u32,
    compute_out_len: impl Fn(&mut Function, u32),
    compute_src_idx: impl Fn(&mut Function, u32, u32),
) {
    let i_slot = next_scratch;
    let in_len_slot = next_scratch + 1;
    let in_slot = next_scratch + 2;
    let out_slot = next_scratch + 3;
    let out_len_slot = next_scratch + 4;

    // Stash input ptr, null-check.
    body.instruction(&Instruction::LocalTee(in_slot));
    body.instruction(&Instruction::I32Eqz);
    body.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
    // Null input: allocate empty Seq and return.
    body.instruction(&Instruction::I32Const(SEQ_HEADER_SIZE));
    body.instruction(&Instruction::Call(FN_ALLOC));
    body.instruction(&Instruction::LocalTee(out_slot));
    body.instruction(&Instruction::I32Const(TAG_SEQ));
    body.instruction(&Instruction::I32Store(i32_at(0)));
    body.instruction(&Instruction::LocalGet(out_slot));
    body.instruction(&Instruction::I32Const(0));
    body.instruction(&Instruction::I32Store(i32_at(4)));
    body.instruction(&Instruction::LocalGet(out_slot));
    body.instruction(&Instruction::Else);
    // Non-null: read in_len, derive out_len, alloc out_seq.
    body.instruction(&Instruction::LocalGet(in_slot));
    body.instruction(&Instruction::I32Load(i32_at(4)));
    body.instruction(&Instruction::LocalSet(in_len_slot));
    compute_out_len(body, in_len_slot);
    body.instruction(&Instruction::LocalSet(out_len_slot));
    // alloc = header + 4 * out_len
    body.instruction(&Instruction::I32Const(SEQ_HEADER_SIZE));
    body.instruction(&Instruction::LocalGet(out_len_slot));
    body.instruction(&Instruction::I32Const(4));
    body.instruction(&Instruction::I32Mul);
    body.instruction(&Instruction::I32Add);
    body.instruction(&Instruction::Call(FN_ALLOC));
    body.instruction(&Instruction::LocalTee(out_slot));
    body.instruction(&Instruction::I32Const(TAG_SEQ));
    body.instruction(&Instruction::I32Store(i32_at(0)));
    body.instruction(&Instruction::LocalGet(out_slot));
    body.instruction(&Instruction::LocalGet(out_len_slot));
    body.instruction(&Instruction::I32Store(i32_at(4)));
    // i = 0 ; loop body copies in[src_idx(i, in_len)] → out[i]
    body.instruction(&Instruction::I32Const(0));
    body.instruction(&Instruction::LocalSet(i_slot));
    body.instruction(&Instruction::Block(BlockType::Empty));
    body.instruction(&Instruction::Loop(BlockType::Empty));
    body.instruction(&Instruction::LocalGet(i_slot));
    body.instruction(&Instruction::LocalGet(out_len_slot));
    body.instruction(&Instruction::I32GeS);
    body.instruction(&Instruction::BrIf(1));
    // Store addr = out_seq + 4*i (the I32Store adds SEQ_HEADER_SIZE).
    body.instruction(&Instruction::LocalGet(out_slot));
    body.instruction(&Instruction::LocalGet(i_slot));
    body.instruction(&Instruction::I32Const(4));
    body.instruction(&Instruction::I32Mul);
    body.instruction(&Instruction::I32Add);
    // Load source element address = in_seq + 4 * src_idx.
    body.instruction(&Instruction::LocalGet(in_slot));
    compute_src_idx(body, i_slot, in_len_slot);
    body.instruction(&Instruction::I32Const(4));
    body.instruction(&Instruction::I32Mul);
    body.instruction(&Instruction::I32Add);
    body.instruction(&Instruction::I32Load(i32_at(SEQ_HEADER_SIZE as u64)));
    body.instruction(&Instruction::I32Store(i32_at(SEQ_HEADER_SIZE as u64)));
    // i += 1
    body.instruction(&Instruction::LocalGet(i_slot));
    body.instruction(&Instruction::I32Const(1));
    body.instruction(&Instruction::I32Add);
    body.instruction(&Instruction::LocalSet(i_slot));
    body.instruction(&Instruction::Br(0));
    body.instruction(&Instruction::End); // end loop
    body.instruction(&Instruction::End); // end block
    body.instruction(&Instruction::LocalGet(out_slot));
    body.instruction(&Instruction::End); // end else (null-guard)
}

/// Emit signed i64 division with Backus's ÷ semantics: b == 0 yields
/// φ (null ptr) rather than trapping. Uses the function-level i64
/// scratch `div_i64_slot` to stash b across the zero-check branch.
fn emit_binary_i64_div(body: &mut Function, pair_slot: u32, div_i64_slot: u32) {
    body.instruction(&Instruction::LocalSet(pair_slot));
    // Load b, stash it in the i64 slot, test for zero.
    body.instruction(&Instruction::LocalGet(pair_slot));
    body.instruction(&Instruction::I32Load(i32_at(12)));
    body.instruction(&Instruction::I64Load(i64_at(8)));
    body.instruction(&Instruction::LocalTee(div_i64_slot));
    body.instruction(&Instruction::I64Eqz);
    body.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
    // b == 0 : phi.
    body.instruction(&Instruction::I32Const(0));
    body.instruction(&Instruction::Else);
    // a / b, alloc_atom.
    body.instruction(&Instruction::LocalGet(pair_slot));
    body.instruction(&Instruction::I32Load(i32_at(8)));
    body.instruction(&Instruction::I64Load(i64_at(8)));
    body.instruction(&Instruction::LocalGet(div_i64_slot));
    body.instruction(&Instruction::I64DivS);
    body.instruction(&Instruction::Call(FN_ALLOC_ATOM));
    body.instruction(&Instruction::End);
}

/// Emit instructions that consume one i32 Object pointer from the
/// stack and leave one i32 Object pointer on the stack — the
/// stack-discipline lowering convention.
///
/// `next_scratch` is the first free i32 local index. Subterms that
/// need temporaries claim slots and pass `next_scratch + k` to their
/// children, so nested uses get distinct slots.
fn emit_body(
    func: &Func,
    body: &mut Function,
    next_scratch: u32,
    div_i64_slot: u32,
) -> Result<(), String> {
    match func {
        // id:x = x — ptr on stack is already the output.
        Func::Id => {}

        // Backus §11.2.4 Selector: s_i:<x₁, …, xₙ> = xᵢ (1-indexed).
        // Input is a Seq ptr; the element at index i-1 lives at
        // byte offset 8 + 4*(i-1). One i32.load does the whole job —
        // this is the cheapest lowering in the PoC.
        //
        // Out-of-bounds selection (i > length) reads undefined heap
        // contents in linear memory; we do not bounds-check at
        // emit time. Callers are expected to compose Selector with
        // constructions of sufficient arity — FORML 2 role positions
        // always do.
        Func::Selector(i) => {
            if *i < 1 {
                return Err(format!("Selector index must be ≥ 1, got {}", i));
            }
            let offset = (SEQ_HEADER_SIZE as u64) + 4 * (*i as u64 - 1);
            body.instruction(&Instruction::I32Load(MemArg {
                offset,
                align: 2,
                memory_index: 0,
            }));
        }

        // Backus §11.2.3 arithmetic on a pair Atom.
        //
        //   +:<y, z>  = y + z
        //   -:<y, z>  = y - z
        //   ×:<y, z>  = y × z
        //   ÷:<y, z>  = y ÷ z,  φ if z = 0
        //
        // Shared shape (pair_slot holds the Seq ptr, b_slot holds the
        // i64 rhs so we can load both operands and invoke the op):
        //
        //   local.set pair                ; stash Seq ptr
        //   local.get pair ; i32.load 12 ; i64.load 8   ; load b
        //   local.set b
        //   local.get pair ; i32.load  8 ; i64.load 8   ; load a
        //   local.get b
        //   <i64 op>
        //   call alloc_atom
        //
        // Div adds a zero-check on b before the op, returning phi
        // (ptr 0) if b == 0 so we never trap on division. This
        // matches Backus's ÷ and AREST's Object::Bottom propagation.
        Func::Add => emit_binary_i64_arith(body, next_scratch, Instruction::I64Add),
        Func::Sub => emit_binary_i64_arith(body, next_scratch, Instruction::I64Sub),
        Func::Mul => emit_binary_i64_arith(body, next_scratch, Instruction::I64Mul),
        Func::Div => emit_binary_i64_div(body, next_scratch, div_i64_slot),

        // Backus §11.2.3 comparisons — all signed, all on pair Atoms.
        // Result is an i64 Atom holding 0 (false) or 1 (true); this
        // slots naturally into Condition/Filter's truthy check.
        Func::Eq => emit_binary_i64_compare(body, next_scratch, Instruction::I64Eq),
        Func::Gt => emit_binary_i64_compare(body, next_scratch, Instruction::I64GtS),
        Func::Lt => emit_binary_i64_compare(body, next_scratch, Instruction::I64LtS),
        Func::Ge => emit_binary_i64_compare(body, next_scratch, Instruction::I64GeS),
        Func::Le => emit_binary_i64_compare(body, next_scratch, Instruction::I64LeS),

        // Backus §11.2.3 logic.
        //
        //   and:<y, z>  = 1 if truthy(y) ∧ truthy(z) else 0
        //   or:<y, z>   = 1 if truthy(y) ∨ truthy(z) else 0
        //   not:y       = 1 if ¬truthy(y) else 0
        //
        // For {0, 1} i32 values produced by truthy(), bitwise
        // i32.and/or coincide with logical and/or. The result Atom
        // can feed another logical op or flow through Condition.
        Func::And => {
            let pair_slot = next_scratch;
            body.instruction(&Instruction::LocalSet(pair_slot));
            body.instruction(&Instruction::LocalGet(pair_slot));
            body.instruction(&Instruction::I32Load(i32_at(8)));
            body.instruction(&Instruction::Call(FN_TRUTHY));
            body.instruction(&Instruction::LocalGet(pair_slot));
            body.instruction(&Instruction::I32Load(i32_at(12)));
            body.instruction(&Instruction::Call(FN_TRUTHY));
            body.instruction(&Instruction::I32And);
            body.instruction(&Instruction::I64ExtendI32U);
            body.instruction(&Instruction::Call(FN_ALLOC_ATOM));
        }
        Func::Or => {
            let pair_slot = next_scratch;
            body.instruction(&Instruction::LocalSet(pair_slot));
            body.instruction(&Instruction::LocalGet(pair_slot));
            body.instruction(&Instruction::I32Load(i32_at(8)));
            body.instruction(&Instruction::Call(FN_TRUTHY));
            body.instruction(&Instruction::LocalGet(pair_slot));
            body.instruction(&Instruction::I32Load(i32_at(12)));
            body.instruction(&Instruction::Call(FN_TRUTHY));
            body.instruction(&Instruction::I32Or);
            body.instruction(&Instruction::I64ExtendI32U);
            body.instruction(&Instruction::Call(FN_ALLOC_ATOM));
        }
        Func::Not => {
            // Unary: stack has one Object ptr. Call truthy (i32 0/1),
            // invert via i32.eqz (which maps 0→1 and nonzero→0),
            // extend, alloc Atom.
            body.instruction(&Instruction::Call(FN_TRUTHY));
            body.instruction(&Instruction::I32Eqz);
            body.instruction(&Instruction::I64ExtendI32U);
            body.instruction(&Instruction::Call(FN_ALLOC_ATOM));
        }

        // Backus §11.2.3 structural predicates. All unary, all produce
        // a {0, 1} Atom. They inspect the Object tag rather than
        // value, so they require a null-pointer guard — dereferencing
        // address 0 would silently load sentinel bytes.
        //
        //   atom:x  = 1 if x is a non-null Atom else 0
        //   null:x  = 1 if x = φ (null ptr or empty Seq) else 0
        //   length:x = the Seq length as an Atom (φ if x is an Atom)
        Func::AtomTest => {
            // Both numeric atoms (tag 0) and string atoms (tag 2) are
            // "atom" per Backus — the test predicate recognizes them
            // both, rejecting only sequences (tag 1). `tag != TAG_SEQ`
            // captures the full atomhood check with a single compare.
            let elem_slot = next_scratch;
            body.instruction(&Instruction::LocalTee(elem_slot));
            body.instruction(&Instruction::I32Eqz);
            body.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
            body.instruction(&Instruction::I32Const(0)); // null is not atom
            body.instruction(&Instruction::Else);
            body.instruction(&Instruction::LocalGet(elem_slot));
            body.instruction(&Instruction::I32Load(i32_at(0)));
            body.instruction(&Instruction::I32Const(TAG_SEQ));
            body.instruction(&Instruction::I32Ne);
            body.instruction(&Instruction::End);
            body.instruction(&Instruction::I64ExtendI32U);
            body.instruction(&Instruction::Call(FN_ALLOC_ATOM));
        }
        Func::NullTest => {
            // φ in the PoC is either a null ptr (from Insert on empty
            // Seq) or a Seq of length 0 (from Filter/empty
            // Construction). Both must test positive here.
            let elem_slot = next_scratch;
            body.instruction(&Instruction::LocalTee(elem_slot));
            body.instruction(&Instruction::I32Eqz);
            body.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
            body.instruction(&Instruction::I32Const(1)); // null ptr → φ
            body.instruction(&Instruction::Else);
            body.instruction(&Instruction::LocalGet(elem_slot));
            body.instruction(&Instruction::I32Load(i32_at(0)));
            body.instruction(&Instruction::I32Const(TAG_SEQ));
            body.instruction(&Instruction::I32Eq);
            body.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
            // Seq: φ iff length == 0
            body.instruction(&Instruction::LocalGet(elem_slot));
            body.instruction(&Instruction::I32Load(i32_at(4)));
            body.instruction(&Instruction::I32Eqz);
            body.instruction(&Instruction::Else);
            // Atom: not φ
            body.instruction(&Instruction::I32Const(0));
            body.instruction(&Instruction::End);
            body.instruction(&Instruction::End);
            body.instruction(&Instruction::I64ExtendI32U);
            body.instruction(&Instruction::Call(FN_ALLOC_ATOM));
        }
        Func::Length => {
            // length on an Atom is φ per Backus (bottom-on-type-error).
            // length on a Seq is the u32 length field, widened to i64.
            let elem_slot = next_scratch;
            body.instruction(&Instruction::LocalTee(elem_slot));
            body.instruction(&Instruction::I32Eqz);
            body.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
            body.instruction(&Instruction::I32Const(0)); // null → φ
            body.instruction(&Instruction::Else);
            body.instruction(&Instruction::LocalGet(elem_slot));
            body.instruction(&Instruction::I32Load(i32_at(0)));
            body.instruction(&Instruction::I32Const(TAG_SEQ));
            body.instruction(&Instruction::I32Eq);
            body.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
            body.instruction(&Instruction::LocalGet(elem_slot));
            body.instruction(&Instruction::I32Load(i32_at(4)));
            body.instruction(&Instruction::I64ExtendI32U);
            body.instruction(&Instruction::Call(FN_ALLOC_ATOM));
            body.instruction(&Instruction::Else);
            body.instruction(&Instruction::I32Const(0)); // Atom → φ
            body.instruction(&Instruction::End);
            body.instruction(&Instruction::End);
        }

        // Backus §11.2.3 string predicate.
        //
        //   contains:<haystack, needle> = 1 if haystack contains needle else 0
        //
        // Both inputs are string atoms (tag 2). Brute-force byte-
        // level substring search: outer loop tries each offset i in
        // haystack; inner loop compares needle bytes against
        // haystack[i..i+needle_len]. Early-exits on first match.
        //
        // Bounds: pre-check needle_len > haystack_len and return 0
        // immediately so the subtracted outer bound `haystack_len -
        // needle_len` stays non-negative. Empty needle is treated as
        // "always present" — matches the common string-contains
        // contract (0 chars trivially appear at position 0).
        Func::Contains => {
            let pair_slot = next_scratch;
            let haystack_slot = next_scratch + 1;
            let needle_slot = next_scratch + 2;
            let haystack_len_slot = next_scratch + 3;
            let needle_len_slot = next_scratch + 4;
            let i_slot = next_scratch + 5;
            let j_slot = next_scratch + 6;
            let match_slot = next_scratch + 7;
            let result_slot = next_scratch + 8;

            body.instruction(&Instruction::LocalSet(pair_slot));
            // haystack = pair[0]
            body.instruction(&Instruction::LocalGet(pair_slot));
            body.instruction(&Instruction::I32Load(i32_at(8)));
            body.instruction(&Instruction::LocalTee(haystack_slot));
            // haystack_len = haystack.length (offset 4 of StringAtom)
            body.instruction(&Instruction::I32Load(i32_at(4)));
            body.instruction(&Instruction::LocalSet(haystack_len_slot));
            // needle = pair[1]
            body.instruction(&Instruction::LocalGet(pair_slot));
            body.instruction(&Instruction::I32Load(i32_at(12)));
            body.instruction(&Instruction::LocalTee(needle_slot));
            body.instruction(&Instruction::I32Load(i32_at(4)));
            body.instruction(&Instruction::LocalSet(needle_len_slot));
            // result = 0 initially
            body.instruction(&Instruction::I32Const(0));
            body.instruction(&Instruction::LocalSet(result_slot));
            // If needle_len > haystack_len → skip search (result stays 0).
            body.instruction(&Instruction::LocalGet(needle_len_slot));
            body.instruction(&Instruction::LocalGet(haystack_len_slot));
            body.instruction(&Instruction::I32GtS);
            body.instruction(&Instruction::If(BlockType::Empty));
            body.instruction(&Instruction::Else);
            // i = 0
            body.instruction(&Instruction::I32Const(0));
            body.instruction(&Instruction::LocalSet(i_slot));
            body.instruction(&Instruction::Block(BlockType::Empty));
            body.instruction(&Instruction::Loop(BlockType::Empty));
            // if i > haystack_len - needle_len: break outer
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::LocalGet(haystack_len_slot));
            body.instruction(&Instruction::LocalGet(needle_len_slot));
            body.instruction(&Instruction::I32Sub);
            body.instruction(&Instruction::I32GtS);
            body.instruction(&Instruction::BrIf(1));
            // match = 1
            body.instruction(&Instruction::I32Const(1));
            body.instruction(&Instruction::LocalSet(match_slot));
            // j = 0
            body.instruction(&Instruction::I32Const(0));
            body.instruction(&Instruction::LocalSet(j_slot));
            body.instruction(&Instruction::Block(BlockType::Empty));
            body.instruction(&Instruction::Loop(BlockType::Empty));
            // if j >= needle_len: break inner
            body.instruction(&Instruction::LocalGet(j_slot));
            body.instruction(&Instruction::LocalGet(needle_len_slot));
            body.instruction(&Instruction::I32GeS);
            body.instruction(&Instruction::BrIf(1));
            // Load haystack byte at [8 + i + j]
            body.instruction(&Instruction::LocalGet(haystack_slot));
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::LocalGet(j_slot));
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::I32Load8U(MemArg {
                offset: SEQ_HEADER_SIZE as u64,
                align: 0,
                memory_index: 0,
            }));
            // Load needle byte at [8 + j]
            body.instruction(&Instruction::LocalGet(needle_slot));
            body.instruction(&Instruction::LocalGet(j_slot));
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::I32Load8U(MemArg {
                offset: SEQ_HEADER_SIZE as u64,
                align: 0,
                memory_index: 0,
            }));
            body.instruction(&Instruction::I32Ne);
            body.instruction(&Instruction::If(BlockType::Empty));
            // Mismatch: clear match flag and break inner.
            body.instruction(&Instruction::I32Const(0));
            body.instruction(&Instruction::LocalSet(match_slot));
            body.instruction(&Instruction::Br(2)); // exit inner block
            body.instruction(&Instruction::End);
            // j += 1
            body.instruction(&Instruction::LocalGet(j_slot));
            body.instruction(&Instruction::I32Const(1));
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::LocalSet(j_slot));
            body.instruction(&Instruction::Br(0)); // continue inner
            body.instruction(&Instruction::End); // end inner loop
            body.instruction(&Instruction::End); // end inner block
            // Post-inner: if match was still 1, record success and exit outer.
            body.instruction(&Instruction::LocalGet(match_slot));
            body.instruction(&Instruction::If(BlockType::Empty));
            body.instruction(&Instruction::I32Const(1));
            body.instruction(&Instruction::LocalSet(result_slot));
            body.instruction(&Instruction::Br(2)); // exit outer block
            body.instruction(&Instruction::End);
            // i += 1; continue outer
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::I32Const(1));
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::LocalSet(i_slot));
            body.instruction(&Instruction::Br(0));
            body.instruction(&Instruction::End); // end outer loop
            body.instruction(&Instruction::End); // end outer block
            body.instruction(&Instruction::End); // end if (needle_len <= haystack_len)
            // Wrap result as numeric Atom.
            body.instruction(&Instruction::LocalGet(result_slot));
            body.instruction(&Instruction::I64ExtendI32U);
            body.instruction(&Instruction::Call(FN_ALLOC_ATOM));
        }

        // Backus §11.2.3 unary string transformer.
        //
        //   lower:x = lowercase(x)   for string x
        //
        // ASCII-only case fold: bytes in [A-Z] (65..=90) get +32.
        // Everything else copies through unchanged, so multi-byte
        // UTF-8 sequences pass through byte-identical. Branchless
        // delta: `in_range ? 32 : 0` via bitmask + shift.
        Func::Lower => {
            let src_slot = next_scratch;
            let src_len_slot = next_scratch + 1;
            let dst_slot = next_scratch + 2;
            let i_slot = next_scratch + 3;
            let b_slot = next_scratch + 4;
            body.instruction(&Instruction::LocalSet(src_slot));
            // src_len = src.length
            body.instruction(&Instruction::LocalGet(src_slot));
            body.instruction(&Instruction::I32Load(i32_at(4)));
            body.instruction(&Instruction::LocalSet(src_len_slot));
            // dst = alloc_string_atom(src_len)
            body.instruction(&Instruction::LocalGet(src_len_slot));
            body.instruction(&Instruction::Call(FN_ALLOC_STRING_ATOM));
            body.instruction(&Instruction::LocalSet(dst_slot));
            // i = 0
            body.instruction(&Instruction::I32Const(0));
            body.instruction(&Instruction::LocalSet(i_slot));
            body.instruction(&Instruction::Block(BlockType::Empty));
            body.instruction(&Instruction::Loop(BlockType::Empty));
            // if i >= src_len: break
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::LocalGet(src_len_slot));
            body.instruction(&Instruction::I32GeS);
            body.instruction(&Instruction::BrIf(1));
            // b = src[8 + i]
            body.instruction(&Instruction::LocalGet(src_slot));
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::I32Load8U(MemArg {
                offset: SEQ_HEADER_SIZE as u64,
                align: 0,
                memory_index: 0,
            }));
            body.instruction(&Instruction::LocalSet(b_slot));
            // Branchless lowercase delta:
            //   in_range = (b >= 65) & (b <= 90)
            //   delta    = in_range << 5      ; 0 or 32
            //   lowered  = b + delta
            body.instruction(&Instruction::LocalGet(b_slot));
            body.instruction(&Instruction::I32Const(65));
            body.instruction(&Instruction::I32GeS);
            body.instruction(&Instruction::LocalGet(b_slot));
            body.instruction(&Instruction::I32Const(90));
            body.instruction(&Instruction::I32LeS);
            body.instruction(&Instruction::I32And);
            body.instruction(&Instruction::I32Const(5));
            body.instruction(&Instruction::I32Shl);
            body.instruction(&Instruction::LocalGet(b_slot));
            body.instruction(&Instruction::I32Add);
            // Store dst[8 + i] = lowered_b
            // Pull the result off the stack into b_slot so we can
            // position the store-address without losing it.
            body.instruction(&Instruction::LocalSet(b_slot));
            body.instruction(&Instruction::LocalGet(dst_slot));
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::LocalGet(b_slot));
            body.instruction(&Instruction::I32Store8(MemArg {
                offset: SEQ_HEADER_SIZE as u64,
                align: 0,
                memory_index: 0,
            }));
            // i += 1
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::I32Const(1));
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::LocalSet(i_slot));
            body.instruction(&Instruction::Br(0));
            body.instruction(&Instruction::End); // end loop
            body.instruction(&Instruction::End); // end block
            body.instruction(&Instruction::LocalGet(dst_slot));
        }

        // ── Backus §14.3 cell access via host imports ───────────────
        //
        //   ↑n:D        = host cell_fetch(n)           (Fetch)
        //   ↑n:D (soft) = host cell_fetch_or_phi(n)    (FetchOrPhi)
        //   ↓n:<x,D>    = host cell_store(n, x)        (Store)
        //
        // The PoC keeps D on the host side — it's the tenant's
        // CompiledState, not something that fits neatly in WASM
        // linear memory. Each operation marshals its name and
        // optional value through imported host functions which read
        // from / write to D and allocate the result back into the
        // module's arena via the exported alloc helpers.
        Func::Fetch => {
            // x on stack IS the name atom. Host reads its bytes,
            // looks up in D, and returns a freshly-allocated value
            // pointer (or 0 for φ on miss).
            body.instruction(&Instruction::Call(IMPORT_CELL_FETCH));
        }
        Func::FetchOrPhi => {
            // Same shape as Fetch; the host returns an empty-Seq
            // pointer (`<>`) on miss instead of φ, matching AREST's
            // indexed fact-type convention that avoids ⊥-propagation.
            body.instruction(&Instruction::Call(IMPORT_CELL_FETCH_OR_PHI));
        }
        Func::Store => {
            // Pair input `<name, value>`. Unpack, call the host.
            // Returns the stored value's pointer so Store composes
            // as a pass-through write.
            let pair_slot = next_scratch;
            body.instruction(&Instruction::LocalSet(pair_slot));
            body.instruction(&Instruction::LocalGet(pair_slot));
            body.instruction(&Instruction::I32Load(i32_at(8)));
            body.instruction(&Instruction::LocalGet(pair_slot));
            body.instruction(&Instruction::I32Load(i32_at(12)));
            body.instruction(&Instruction::Call(IMPORT_CELL_STORE));
        }

        // ── Def / Platform dispatch via host imports ────────────────
        //
        //   Func::Def(name):x      = host def_dispatch(name, x)
        //   Func::Platform(name):x = host platform_dispatch(name, x)
        //
        // The name is compile-time known; emit an inline StringAtom
        // for it at the call site. The host function receives (name,
        // x) pointers and runs `ast::apply(Func::Def(name), x, &d)`
        // (or Platform) against the host-side D, marshaling the
        // result back into linear memory.
        Func::Def(name) | Func::Platform(name) => {
            let import_idx = match func {
                Func::Def(_) => IMPORT_DEF_DISPATCH,
                _ => IMPORT_PLATFORM_DISPATCH,
            };
            let x_slot = next_scratch;
            let name_slot = next_scratch + 1;
            // Stash x.
            body.instruction(&Instruction::LocalSet(x_slot));
            // Allocate a StringAtom for the name.
            body.instruction(&Instruction::I32Const(name.len() as i32));
            body.instruction(&Instruction::Call(FN_ALLOC_STRING_ATOM));
            body.instruction(&Instruction::LocalSet(name_slot));
            for (i, byte) in name.as_bytes().iter().enumerate() {
                body.instruction(&Instruction::LocalGet(name_slot));
                body.instruction(&Instruction::I32Const(*byte as i32));
                body.instruction(&Instruction::I32Store8(MemArg {
                    offset: (SEQ_HEADER_SIZE as u64) + i as u64,
                    align: 0,
                    memory_index: 0,
                }));
            }
            // Call import(name, x) → result.
            body.instruction(&Instruction::LocalGet(name_slot));
            body.instruction(&Instruction::LocalGet(x_slot));
            body.instruction(&Instruction::Call(import_idx));
        }

        // Backus §11.2.3 unary Seq transformers. All four share the
        // allocate-and-copy skeleton in `emit_unary_seq_map`; they
        // differ only in out_len and the src_idx mapping.
        //
        //   tl:<x₁,...,xₙ>       = <x₂,...,xₙ>       (out_len = max(in_len-1, 0))
        //   reverse:<x₁,...,xₙ>  = <xₙ,...,x₁>       (out_len = in_len)
        //   rotl:<x₁,...,xₙ>     = <x₂,...,xₙ,x₁>    (out_len = in_len)
        //   rotr:<x₁,...,xₙ>     = <xₙ,x₁,...,xₙ₋₁> (out_len = in_len)
        //
        // Empty/null input yields an empty Seq. Rot's modulo uses
        // i32.rem_u which would trap on divisor 0 — safe because
        // the loop never executes when in_len == 0.
        Func::Tail => emit_unary_seq_map(
            body, next_scratch,
            // out_len = in_len - (in_len > 0 ? 1 : 0)  (saturated subtract)
            |body, in_len_slot| {
                body.instruction(&Instruction::LocalGet(in_len_slot));
                body.instruction(&Instruction::LocalGet(in_len_slot));
                body.instruction(&Instruction::I32Const(0));
                body.instruction(&Instruction::I32GtS);
                body.instruction(&Instruction::I32Sub);
            },
            // src_idx = i + 1
            |body, i_slot, _in_len_slot| {
                body.instruction(&Instruction::LocalGet(i_slot));
                body.instruction(&Instruction::I32Const(1));
                body.instruction(&Instruction::I32Add);
            },
        ),
        Func::Reverse => emit_unary_seq_map(
            body, next_scratch,
            // out_len = in_len
            |body, in_len_slot| {
                body.instruction(&Instruction::LocalGet(in_len_slot));
            },
            // src_idx = in_len - 1 - i
            |body, i_slot, in_len_slot| {
                body.instruction(&Instruction::LocalGet(in_len_slot));
                body.instruction(&Instruction::I32Const(1));
                body.instruction(&Instruction::I32Sub);
                body.instruction(&Instruction::LocalGet(i_slot));
                body.instruction(&Instruction::I32Sub);
            },
        ),
        Func::RotL => emit_unary_seq_map(
            body, next_scratch,
            |body, in_len_slot| {
                body.instruction(&Instruction::LocalGet(in_len_slot));
            },
            // src_idx = (i + 1) % in_len
            |body, i_slot, in_len_slot| {
                body.instruction(&Instruction::LocalGet(i_slot));
                body.instruction(&Instruction::I32Const(1));
                body.instruction(&Instruction::I32Add);
                body.instruction(&Instruction::LocalGet(in_len_slot));
                body.instruction(&Instruction::I32RemU);
            },
        ),
        Func::RotR => emit_unary_seq_map(
            body, next_scratch,
            |body, in_len_slot| {
                body.instruction(&Instruction::LocalGet(in_len_slot));
            },
            // src_idx = (i + in_len - 1) % in_len
            |body, i_slot, in_len_slot| {
                body.instruction(&Instruction::LocalGet(i_slot));
                body.instruction(&Instruction::LocalGet(in_len_slot));
                body.instruction(&Instruction::I32Add);
                body.instruction(&Instruction::I32Const(1));
                body.instruction(&Instruction::I32Sub);
                body.instruction(&Instruction::LocalGet(in_len_slot));
                body.instruction(&Instruction::I32RemU);
            },
        ),

        // Backus §11.2.3 binary Seq builders.
        //
        //   apndl:<y, <z₁,...,zₙ>> = <y, z₁,...,zₙ>        (prepend head)
        //   apndr:<<z₁,...,zₙ>, y> = <z₁,...,zₙ, y>        (append tail)
        //   concat:<<a₁...>, <b₁...>, ...> = <a₁..., b₁..., ...> (flatten)
        //
        // ApndL/ApndR take a pair input. Inner Seq's length dictates
        // output length minus 1. The scalar element goes to the
        // appropriate end (head or tail); the rest is a straight copy.
        Func::ApndL => {
            let pair_slot = next_scratch;
            let inner_slot = next_scratch + 1;
            let inner_len_slot = next_scratch + 2;
            let out_slot = next_scratch + 3;
            let i_slot = next_scratch + 4;
            body.instruction(&Instruction::LocalSet(pair_slot));
            // inner = pair[1], inner_len = inner.length
            body.instruction(&Instruction::LocalGet(pair_slot));
            body.instruction(&Instruction::I32Load(i32_at(12)));
            body.instruction(&Instruction::LocalTee(inner_slot));
            body.instruction(&Instruction::I32Load(i32_at(4)));
            body.instruction(&Instruction::LocalSet(inner_len_slot));
            // Alloc out: header + 4 * (inner_len + 1)
            body.instruction(&Instruction::I32Const(SEQ_HEADER_SIZE + 4));
            body.instruction(&Instruction::LocalGet(inner_len_slot));
            body.instruction(&Instruction::I32Const(4));
            body.instruction(&Instruction::I32Mul);
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::Call(FN_ALLOC));
            body.instruction(&Instruction::LocalTee(out_slot));
            body.instruction(&Instruction::I32Const(TAG_SEQ));
            body.instruction(&Instruction::I32Store(i32_at(0)));
            body.instruction(&Instruction::LocalGet(out_slot));
            body.instruction(&Instruction::LocalGet(inner_len_slot));
            body.instruction(&Instruction::I32Const(1));
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::I32Store(i32_at(4)));
            // out[0] = pair[0] = y
            body.instruction(&Instruction::LocalGet(out_slot));
            body.instruction(&Instruction::LocalGet(pair_slot));
            body.instruction(&Instruction::I32Load(i32_at(8)));
            body.instruction(&Instruction::I32Store(i32_at(SEQ_HEADER_SIZE as u64)));
            // Loop: for i in 0..inner_len, out[i+1] = inner[i]
            body.instruction(&Instruction::I32Const(0));
            body.instruction(&Instruction::LocalSet(i_slot));
            body.instruction(&Instruction::Block(BlockType::Empty));
            body.instruction(&Instruction::Loop(BlockType::Empty));
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::LocalGet(inner_len_slot));
            body.instruction(&Instruction::I32GeS);
            body.instruction(&Instruction::BrIf(1));
            // Store addr = out + 4 * (i + 1) = out + 4 + 4*i ; I32Store adds SEQ_HEADER_SIZE.
            body.instruction(&Instruction::LocalGet(out_slot));
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::I32Const(1));
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::I32Const(4));
            body.instruction(&Instruction::I32Mul);
            body.instruction(&Instruction::I32Add);
            // Load elem = inner[i]
            body.instruction(&Instruction::LocalGet(inner_slot));
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::I32Const(4));
            body.instruction(&Instruction::I32Mul);
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::I32Load(i32_at(SEQ_HEADER_SIZE as u64)));
            body.instruction(&Instruction::I32Store(i32_at(SEQ_HEADER_SIZE as u64)));
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::I32Const(1));
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::LocalSet(i_slot));
            body.instruction(&Instruction::Br(0));
            body.instruction(&Instruction::End);
            body.instruction(&Instruction::End);
            body.instruction(&Instruction::LocalGet(out_slot));
        }
        Func::ApndR => {
            // Mirror of ApndL: inner is pair[0], tail element is pair[1].
            // Copy inner → out[0..n], then out[n] = y.
            let pair_slot = next_scratch;
            let inner_slot = next_scratch + 1;
            let inner_len_slot = next_scratch + 2;
            let out_slot = next_scratch + 3;
            let i_slot = next_scratch + 4;
            body.instruction(&Instruction::LocalSet(pair_slot));
            body.instruction(&Instruction::LocalGet(pair_slot));
            body.instruction(&Instruction::I32Load(i32_at(8)));
            body.instruction(&Instruction::LocalTee(inner_slot));
            body.instruction(&Instruction::I32Load(i32_at(4)));
            body.instruction(&Instruction::LocalSet(inner_len_slot));
            body.instruction(&Instruction::I32Const(SEQ_HEADER_SIZE + 4));
            body.instruction(&Instruction::LocalGet(inner_len_slot));
            body.instruction(&Instruction::I32Const(4));
            body.instruction(&Instruction::I32Mul);
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::Call(FN_ALLOC));
            body.instruction(&Instruction::LocalTee(out_slot));
            body.instruction(&Instruction::I32Const(TAG_SEQ));
            body.instruction(&Instruction::I32Store(i32_at(0)));
            body.instruction(&Instruction::LocalGet(out_slot));
            body.instruction(&Instruction::LocalGet(inner_len_slot));
            body.instruction(&Instruction::I32Const(1));
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::I32Store(i32_at(4)));
            // Loop: for i in 0..inner_len, out[i] = inner[i]
            body.instruction(&Instruction::I32Const(0));
            body.instruction(&Instruction::LocalSet(i_slot));
            body.instruction(&Instruction::Block(BlockType::Empty));
            body.instruction(&Instruction::Loop(BlockType::Empty));
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::LocalGet(inner_len_slot));
            body.instruction(&Instruction::I32GeS);
            body.instruction(&Instruction::BrIf(1));
            body.instruction(&Instruction::LocalGet(out_slot));
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::I32Const(4));
            body.instruction(&Instruction::I32Mul);
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::LocalGet(inner_slot));
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::I32Const(4));
            body.instruction(&Instruction::I32Mul);
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::I32Load(i32_at(SEQ_HEADER_SIZE as u64)));
            body.instruction(&Instruction::I32Store(i32_at(SEQ_HEADER_SIZE as u64)));
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::I32Const(1));
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::LocalSet(i_slot));
            body.instruction(&Instruction::Br(0));
            body.instruction(&Instruction::End);
            body.instruction(&Instruction::End);
            // out[inner_len] = y
            body.instruction(&Instruction::LocalGet(out_slot));
            body.instruction(&Instruction::LocalGet(inner_len_slot));
            body.instruction(&Instruction::I32Const(4));
            body.instruction(&Instruction::I32Mul);
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::LocalGet(pair_slot));
            body.instruction(&Instruction::I32Load(i32_at(12)));
            body.instruction(&Instruction::I32Store(i32_at(SEQ_HEADER_SIZE as u64)));
            body.instruction(&Instruction::LocalGet(out_slot));
        }
        // Backus §11.2.3 distribution.
        //
        //   distl:<y, <z₁,...,zₙ>> = <<y,z₁>, <y,z₂>, ..., <y,zₙ>>
        //   distr:<<y₁,...,yₙ>, z> = <<y₁,z>, <y₂,z>, ..., <yₙ,z>>
        //
        // Allocate an outer Seq of length n, and in each iteration
        // allocate a fresh 2-element inner pair <scalar, inner_elem>
        // (for DistL) or <inner_elem, scalar> (for DistR).
        Func::DistL => {
            let pair_slot = next_scratch;
            let inner_slot = next_scratch + 1;
            let inner_len_slot = next_scratch + 2;
            let out_slot = next_scratch + 3;
            let i_slot = next_scratch + 4;
            let pair_i_slot = next_scratch + 5;
            let scalar_slot = next_scratch + 6;
            body.instruction(&Instruction::LocalSet(pair_slot));
            // scalar = pair[0]
            body.instruction(&Instruction::LocalGet(pair_slot));
            body.instruction(&Instruction::I32Load(i32_at(8)));
            body.instruction(&Instruction::LocalSet(scalar_slot));
            // inner = pair[1] ; inner_len = inner.length
            body.instruction(&Instruction::LocalGet(pair_slot));
            body.instruction(&Instruction::I32Load(i32_at(12)));
            body.instruction(&Instruction::LocalTee(inner_slot));
            body.instruction(&Instruction::I32Load(i32_at(4)));
            body.instruction(&Instruction::LocalSet(inner_len_slot));
            // Alloc out = header + 4 * inner_len
            body.instruction(&Instruction::I32Const(SEQ_HEADER_SIZE));
            body.instruction(&Instruction::LocalGet(inner_len_slot));
            body.instruction(&Instruction::I32Const(4));
            body.instruction(&Instruction::I32Mul);
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::Call(FN_ALLOC));
            body.instruction(&Instruction::LocalTee(out_slot));
            body.instruction(&Instruction::I32Const(TAG_SEQ));
            body.instruction(&Instruction::I32Store(i32_at(0)));
            body.instruction(&Instruction::LocalGet(out_slot));
            body.instruction(&Instruction::LocalGet(inner_len_slot));
            body.instruction(&Instruction::I32Store(i32_at(4)));
            // Loop: for i in 0..inner_len, allocate <scalar, inner[i]> and store at out[i].
            body.instruction(&Instruction::I32Const(0));
            body.instruction(&Instruction::LocalSet(i_slot));
            body.instruction(&Instruction::Block(BlockType::Empty));
            body.instruction(&Instruction::Loop(BlockType::Empty));
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::LocalGet(inner_len_slot));
            body.instruction(&Instruction::I32GeS);
            body.instruction(&Instruction::BrIf(1));
            // pair_i = alloc 16 (header + 2 elements)
            body.instruction(&Instruction::I32Const(SEQ_HEADER_SIZE + 8));
            body.instruction(&Instruction::Call(FN_ALLOC));
            body.instruction(&Instruction::LocalTee(pair_i_slot));
            body.instruction(&Instruction::I32Const(TAG_SEQ));
            body.instruction(&Instruction::I32Store(i32_at(0)));
            body.instruction(&Instruction::LocalGet(pair_i_slot));
            body.instruction(&Instruction::I32Const(2));
            body.instruction(&Instruction::I32Store(i32_at(4)));
            // pair_i[0] = scalar
            body.instruction(&Instruction::LocalGet(pair_i_slot));
            body.instruction(&Instruction::LocalGet(scalar_slot));
            body.instruction(&Instruction::I32Store(i32_at(SEQ_HEADER_SIZE as u64)));
            // pair_i[1] = inner[i]
            body.instruction(&Instruction::LocalGet(pair_i_slot));
            body.instruction(&Instruction::LocalGet(inner_slot));
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::I32Const(4));
            body.instruction(&Instruction::I32Mul);
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::I32Load(i32_at(SEQ_HEADER_SIZE as u64)));
            body.instruction(&Instruction::I32Store(i32_at((SEQ_HEADER_SIZE + 4) as u64)));
            // out[i] = pair_i
            body.instruction(&Instruction::LocalGet(out_slot));
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::I32Const(4));
            body.instruction(&Instruction::I32Mul);
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::LocalGet(pair_i_slot));
            body.instruction(&Instruction::I32Store(i32_at(SEQ_HEADER_SIZE as u64)));
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::I32Const(1));
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::LocalSet(i_slot));
            body.instruction(&Instruction::Br(0));
            body.instruction(&Instruction::End);
            body.instruction(&Instruction::End);
            body.instruction(&Instruction::LocalGet(out_slot));
        }
        Func::DistR => {
            // Mirror of DistL: inner is pair[0], scalar is pair[1],
            // output pair's layout is <inner_elem, scalar>.
            let pair_slot = next_scratch;
            let inner_slot = next_scratch + 1;
            let inner_len_slot = next_scratch + 2;
            let out_slot = next_scratch + 3;
            let i_slot = next_scratch + 4;
            let pair_i_slot = next_scratch + 5;
            let scalar_slot = next_scratch + 6;
            body.instruction(&Instruction::LocalSet(pair_slot));
            body.instruction(&Instruction::LocalGet(pair_slot));
            body.instruction(&Instruction::I32Load(i32_at(12)));
            body.instruction(&Instruction::LocalSet(scalar_slot));
            body.instruction(&Instruction::LocalGet(pair_slot));
            body.instruction(&Instruction::I32Load(i32_at(8)));
            body.instruction(&Instruction::LocalTee(inner_slot));
            body.instruction(&Instruction::I32Load(i32_at(4)));
            body.instruction(&Instruction::LocalSet(inner_len_slot));
            body.instruction(&Instruction::I32Const(SEQ_HEADER_SIZE));
            body.instruction(&Instruction::LocalGet(inner_len_slot));
            body.instruction(&Instruction::I32Const(4));
            body.instruction(&Instruction::I32Mul);
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::Call(FN_ALLOC));
            body.instruction(&Instruction::LocalTee(out_slot));
            body.instruction(&Instruction::I32Const(TAG_SEQ));
            body.instruction(&Instruction::I32Store(i32_at(0)));
            body.instruction(&Instruction::LocalGet(out_slot));
            body.instruction(&Instruction::LocalGet(inner_len_slot));
            body.instruction(&Instruction::I32Store(i32_at(4)));
            body.instruction(&Instruction::I32Const(0));
            body.instruction(&Instruction::LocalSet(i_slot));
            body.instruction(&Instruction::Block(BlockType::Empty));
            body.instruction(&Instruction::Loop(BlockType::Empty));
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::LocalGet(inner_len_slot));
            body.instruction(&Instruction::I32GeS);
            body.instruction(&Instruction::BrIf(1));
            body.instruction(&Instruction::I32Const(SEQ_HEADER_SIZE + 8));
            body.instruction(&Instruction::Call(FN_ALLOC));
            body.instruction(&Instruction::LocalTee(pair_i_slot));
            body.instruction(&Instruction::I32Const(TAG_SEQ));
            body.instruction(&Instruction::I32Store(i32_at(0)));
            body.instruction(&Instruction::LocalGet(pair_i_slot));
            body.instruction(&Instruction::I32Const(2));
            body.instruction(&Instruction::I32Store(i32_at(4)));
            // pair_i[0] = inner[i]
            body.instruction(&Instruction::LocalGet(pair_i_slot));
            body.instruction(&Instruction::LocalGet(inner_slot));
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::I32Const(4));
            body.instruction(&Instruction::I32Mul);
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::I32Load(i32_at(SEQ_HEADER_SIZE as u64)));
            body.instruction(&Instruction::I32Store(i32_at(SEQ_HEADER_SIZE as u64)));
            // pair_i[1] = scalar
            body.instruction(&Instruction::LocalGet(pair_i_slot));
            body.instruction(&Instruction::LocalGet(scalar_slot));
            body.instruction(&Instruction::I32Store(i32_at((SEQ_HEADER_SIZE + 4) as u64)));
            // out[i] = pair_i
            body.instruction(&Instruction::LocalGet(out_slot));
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::I32Const(4));
            body.instruction(&Instruction::I32Mul);
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::LocalGet(pair_i_slot));
            body.instruction(&Instruction::I32Store(i32_at(SEQ_HEADER_SIZE as u64)));
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::I32Const(1));
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::LocalSet(i_slot));
            body.instruction(&Instruction::Br(0));
            body.instruction(&Instruction::End);
            body.instruction(&Instruction::End);
            body.instruction(&Instruction::LocalGet(out_slot));
        }

        // Backus §11.2.3 transpose.
        //
        //   trans:<<x₁₁,...,x₁ₘ>, <x₂₁,...,x₂ₘ>, ..., <xₙ₁,...,xₙₘ>>
        //     = <<x₁₁, x₂₁, ..., xₙ₁>, ..., <x₁ₘ, x₂ₘ, ..., xₙₘ>>
        //
        // Preconditions: outer has length ≥ 1 and every inner Seq has
        // the same length m (= outer[0].length). Output has length m,
        // each element is a Seq of length n (= outer length).
        //
        // Empty outer or m == 0 → empty output.
        Func::Trans => {
            let outer_slot = next_scratch;
            let outer_len_slot = next_scratch + 1;
            let inner_len_slot = next_scratch + 2;
            let out_slot = next_scratch + 3;
            let i_slot = next_scratch + 4;
            let pair_i_slot = next_scratch + 5;
            let j_slot = next_scratch + 6;
            body.instruction(&Instruction::LocalSet(outer_slot));
            body.instruction(&Instruction::LocalGet(outer_slot));
            body.instruction(&Instruction::I32Load(i32_at(4)));
            body.instruction(&Instruction::LocalTee(outer_len_slot));
            // Handle empty-outer edge: allocate empty Seq and return.
            body.instruction(&Instruction::I32Eqz);
            body.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
            body.instruction(&Instruction::I32Const(SEQ_HEADER_SIZE));
            body.instruction(&Instruction::Call(FN_ALLOC));
            body.instruction(&Instruction::LocalTee(out_slot));
            body.instruction(&Instruction::I32Const(TAG_SEQ));
            body.instruction(&Instruction::I32Store(i32_at(0)));
            body.instruction(&Instruction::LocalGet(out_slot));
            body.instruction(&Instruction::I32Const(0));
            body.instruction(&Instruction::I32Store(i32_at(4)));
            body.instruction(&Instruction::LocalGet(out_slot));
            body.instruction(&Instruction::Else);
            // inner_len = outer[0].length
            body.instruction(&Instruction::LocalGet(outer_slot));
            body.instruction(&Instruction::I32Load(i32_at(SEQ_HEADER_SIZE as u64)));
            body.instruction(&Instruction::I32Load(i32_at(4)));
            body.instruction(&Instruction::LocalSet(inner_len_slot));
            // Alloc out = header + 4 * inner_len
            body.instruction(&Instruction::I32Const(SEQ_HEADER_SIZE));
            body.instruction(&Instruction::LocalGet(inner_len_slot));
            body.instruction(&Instruction::I32Const(4));
            body.instruction(&Instruction::I32Mul);
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::Call(FN_ALLOC));
            body.instruction(&Instruction::LocalTee(out_slot));
            body.instruction(&Instruction::I32Const(TAG_SEQ));
            body.instruction(&Instruction::I32Store(i32_at(0)));
            body.instruction(&Instruction::LocalGet(out_slot));
            body.instruction(&Instruction::LocalGet(inner_len_slot));
            body.instruction(&Instruction::I32Store(i32_at(4)));
            // Outer loop: for i in 0..inner_len, build out[i] = <outer[0][i], ..., outer[n-1][i]>
            body.instruction(&Instruction::I32Const(0));
            body.instruction(&Instruction::LocalSet(i_slot));
            body.instruction(&Instruction::Block(BlockType::Empty));
            body.instruction(&Instruction::Loop(BlockType::Empty));
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::LocalGet(inner_len_slot));
            body.instruction(&Instruction::I32GeS);
            body.instruction(&Instruction::BrIf(1));
            // pair_i = alloc header + 4 * outer_len
            body.instruction(&Instruction::I32Const(SEQ_HEADER_SIZE));
            body.instruction(&Instruction::LocalGet(outer_len_slot));
            body.instruction(&Instruction::I32Const(4));
            body.instruction(&Instruction::I32Mul);
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::Call(FN_ALLOC));
            body.instruction(&Instruction::LocalTee(pair_i_slot));
            body.instruction(&Instruction::I32Const(TAG_SEQ));
            body.instruction(&Instruction::I32Store(i32_at(0)));
            body.instruction(&Instruction::LocalGet(pair_i_slot));
            body.instruction(&Instruction::LocalGet(outer_len_slot));
            body.instruction(&Instruction::I32Store(i32_at(4)));
            // Inner loop: for j in 0..outer_len, pair_i[j] = outer[j][i]
            body.instruction(&Instruction::I32Const(0));
            body.instruction(&Instruction::LocalSet(j_slot));
            body.instruction(&Instruction::Block(BlockType::Empty));
            body.instruction(&Instruction::Loop(BlockType::Empty));
            body.instruction(&Instruction::LocalGet(j_slot));
            body.instruction(&Instruction::LocalGet(outer_len_slot));
            body.instruction(&Instruction::I32GeS);
            body.instruction(&Instruction::BrIf(1));
            // Store addr = pair_i + 4*j
            body.instruction(&Instruction::LocalGet(pair_i_slot));
            body.instruction(&Instruction::LocalGet(j_slot));
            body.instruction(&Instruction::I32Const(4));
            body.instruction(&Instruction::I32Mul);
            body.instruction(&Instruction::I32Add);
            // Load outer[j][i]: first outer[j], then that Seq's elem i.
            body.instruction(&Instruction::LocalGet(outer_slot));
            body.instruction(&Instruction::LocalGet(j_slot));
            body.instruction(&Instruction::I32Const(4));
            body.instruction(&Instruction::I32Mul);
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::I32Load(i32_at(SEQ_HEADER_SIZE as u64)));
            // Now have outer[j] on stack; read its elem i.
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::I32Const(4));
            body.instruction(&Instruction::I32Mul);
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::I32Load(i32_at(SEQ_HEADER_SIZE as u64)));
            body.instruction(&Instruction::I32Store(i32_at(SEQ_HEADER_SIZE as u64)));
            body.instruction(&Instruction::LocalGet(j_slot));
            body.instruction(&Instruction::I32Const(1));
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::LocalSet(j_slot));
            body.instruction(&Instruction::Br(0));
            body.instruction(&Instruction::End); // inner loop
            body.instruction(&Instruction::End); // inner block
            // out[i] = pair_i
            body.instruction(&Instruction::LocalGet(out_slot));
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::I32Const(4));
            body.instruction(&Instruction::I32Mul);
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::LocalGet(pair_i_slot));
            body.instruction(&Instruction::I32Store(i32_at(SEQ_HEADER_SIZE as u64)));
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::I32Const(1));
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::LocalSet(i_slot));
            body.instruction(&Instruction::Br(0));
            body.instruction(&Instruction::End); // outer loop
            body.instruction(&Instruction::End); // outer block
            body.instruction(&Instruction::LocalGet(out_slot));
            body.instruction(&Instruction::End); // end if/else
        }

        // Backus §11.2.4 While (bounded iteration):
        //
        //   while:(p, f):x = if p:x then while:(p, f):(f:x) else x
        //
        // Evaluate p on the current accumulator; if truthy, replace
        // the accumulator with f:acc and repeat. We cap at 1_000_000
        // iterations as a safety net — AREST derivation rules are
        // monotonic-bounded in principle, but emitting silicon and
        // running under malicious-input conditions makes a hard cap
        // worth the handful of extra instructions per iteration.
        //
        // Scratch:
        //   acc_slot     = next_scratch     : current value
        //   counter_slot = next_scratch + 1 : safety counter
        //   ≥ next_scratch + 2              : shared by p and f
        Func::While(p, f) => {
            let acc_slot = next_scratch;
            let counter_slot = next_scratch + 1;
            body.instruction(&Instruction::LocalSet(acc_slot));
            body.instruction(&Instruction::I32Const(0));
            body.instruction(&Instruction::LocalSet(counter_slot));
            body.instruction(&Instruction::Block(BlockType::Empty));
            body.instruction(&Instruction::Loop(BlockType::Empty));
            // Evaluate p on acc; exit if falsy.
            body.instruction(&Instruction::LocalGet(acc_slot));
            emit_body(p, body, next_scratch + 2, div_i64_slot)?;
            body.instruction(&Instruction::Call(FN_TRUTHY));
            body.instruction(&Instruction::I32Eqz);
            body.instruction(&Instruction::BrIf(1));
            // Safety cap: exit if counter ≥ 1_000_000.
            body.instruction(&Instruction::LocalGet(counter_slot));
            body.instruction(&Instruction::I32Const(1_000_000));
            body.instruction(&Instruction::I32GeS);
            body.instruction(&Instruction::BrIf(1));
            // acc = f(acc)
            body.instruction(&Instruction::LocalGet(acc_slot));
            emit_body(f, body, next_scratch + 2, div_i64_slot)?;
            body.instruction(&Instruction::LocalSet(acc_slot));
            // counter += 1
            body.instruction(&Instruction::LocalGet(counter_slot));
            body.instruction(&Instruction::I32Const(1));
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::LocalSet(counter_slot));
            body.instruction(&Instruction::Br(0));
            body.instruction(&Instruction::End); // end loop
            body.instruction(&Instruction::End); // end block
            body.instruction(&Instruction::LocalGet(acc_slot));
        }

        Func::Concat => {
            // Two-pass: sum inner lengths, then flatten. The bump
            // allocator can't grow an existing region, so we must
            // know the total up front.
            let outer_slot = next_scratch;
            let outer_len_slot = next_scratch + 1;
            let i_slot = next_scratch + 2;
            let total_len_slot = next_scratch + 3;
            let out_slot = next_scratch + 4;
            let out_pos_slot = next_scratch + 5;
            let inner_slot = next_scratch + 6;
            let inner_len_slot = next_scratch + 7;
            let j_slot = next_scratch + 8;
            body.instruction(&Instruction::LocalSet(outer_slot));
            body.instruction(&Instruction::LocalGet(outer_slot));
            body.instruction(&Instruction::I32Load(i32_at(4)));
            body.instruction(&Instruction::LocalSet(outer_len_slot));
            // Pass 1: total_len = Σ outer[i].length
            body.instruction(&Instruction::I32Const(0));
            body.instruction(&Instruction::LocalSet(total_len_slot));
            body.instruction(&Instruction::I32Const(0));
            body.instruction(&Instruction::LocalSet(i_slot));
            body.instruction(&Instruction::Block(BlockType::Empty));
            body.instruction(&Instruction::Loop(BlockType::Empty));
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::LocalGet(outer_len_slot));
            body.instruction(&Instruction::I32GeS);
            body.instruction(&Instruction::BrIf(1));
            // total_len += outer[i].length
            body.instruction(&Instruction::LocalGet(total_len_slot));
            body.instruction(&Instruction::LocalGet(outer_slot));
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::I32Const(4));
            body.instruction(&Instruction::I32Mul);
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::I32Load(i32_at(SEQ_HEADER_SIZE as u64)));
            body.instruction(&Instruction::I32Load(i32_at(4)));
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::LocalSet(total_len_slot));
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::I32Const(1));
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::LocalSet(i_slot));
            body.instruction(&Instruction::Br(0));
            body.instruction(&Instruction::End);
            body.instruction(&Instruction::End);
            // Alloc out = header + 4 * total_len
            body.instruction(&Instruction::I32Const(SEQ_HEADER_SIZE));
            body.instruction(&Instruction::LocalGet(total_len_slot));
            body.instruction(&Instruction::I32Const(4));
            body.instruction(&Instruction::I32Mul);
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::Call(FN_ALLOC));
            body.instruction(&Instruction::LocalTee(out_slot));
            body.instruction(&Instruction::I32Const(TAG_SEQ));
            body.instruction(&Instruction::I32Store(i32_at(0)));
            body.instruction(&Instruction::LocalGet(out_slot));
            body.instruction(&Instruction::LocalGet(total_len_slot));
            body.instruction(&Instruction::I32Store(i32_at(4)));
            // Pass 2: copy inner elements to out[out_pos++]
            body.instruction(&Instruction::I32Const(0));
            body.instruction(&Instruction::LocalSet(out_pos_slot));
            body.instruction(&Instruction::I32Const(0));
            body.instruction(&Instruction::LocalSet(i_slot));
            body.instruction(&Instruction::Block(BlockType::Empty));
            body.instruction(&Instruction::Loop(BlockType::Empty));
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::LocalGet(outer_len_slot));
            body.instruction(&Instruction::I32GeS);
            body.instruction(&Instruction::BrIf(1));
            // inner = outer[i]
            body.instruction(&Instruction::LocalGet(outer_slot));
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::I32Const(4));
            body.instruction(&Instruction::I32Mul);
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::I32Load(i32_at(SEQ_HEADER_SIZE as u64)));
            body.instruction(&Instruction::LocalTee(inner_slot));
            body.instruction(&Instruction::I32Load(i32_at(4)));
            body.instruction(&Instruction::LocalSet(inner_len_slot));
            // Inner loop: for j in 0..inner_len: out[out_pos++] = inner[j]
            body.instruction(&Instruction::I32Const(0));
            body.instruction(&Instruction::LocalSet(j_slot));
            body.instruction(&Instruction::Block(BlockType::Empty));
            body.instruction(&Instruction::Loop(BlockType::Empty));
            body.instruction(&Instruction::LocalGet(j_slot));
            body.instruction(&Instruction::LocalGet(inner_len_slot));
            body.instruction(&Instruction::I32GeS);
            body.instruction(&Instruction::BrIf(1));
            // Store addr = out + 4 * out_pos
            body.instruction(&Instruction::LocalGet(out_slot));
            body.instruction(&Instruction::LocalGet(out_pos_slot));
            body.instruction(&Instruction::I32Const(4));
            body.instruction(&Instruction::I32Mul);
            body.instruction(&Instruction::I32Add);
            // Elem = inner[j]
            body.instruction(&Instruction::LocalGet(inner_slot));
            body.instruction(&Instruction::LocalGet(j_slot));
            body.instruction(&Instruction::I32Const(4));
            body.instruction(&Instruction::I32Mul);
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::I32Load(i32_at(SEQ_HEADER_SIZE as u64)));
            body.instruction(&Instruction::I32Store(i32_at(SEQ_HEADER_SIZE as u64)));
            // out_pos += 1
            body.instruction(&Instruction::LocalGet(out_pos_slot));
            body.instruction(&Instruction::I32Const(1));
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::LocalSet(out_pos_slot));
            // j += 1
            body.instruction(&Instruction::LocalGet(j_slot));
            body.instruction(&Instruction::I32Const(1));
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::LocalSet(j_slot));
            body.instruction(&Instruction::Br(0));
            body.instruction(&Instruction::End); // inner loop
            body.instruction(&Instruction::End); // inner block
            // i += 1
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::I32Const(1));
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::LocalSet(i_slot));
            body.instruction(&Instruction::Br(0));
            body.instruction(&Instruction::End); // outer loop
            body.instruction(&Instruction::End); // outer block
            body.instruction(&Instruction::LocalGet(out_slot));
        }

        // c̄:x = c (when x ≠ ⊥) — drop input ptr, allocate fresh Atom.
        //
        // Two shapes: atoms that parse as `i64` go through the
        // numeric fast path (`alloc_atom` → 16B, zero scratch).
        // Anything else becomes a StringAtom — variable-sized heap
        // object holding the raw UTF-8 bytes.
        Func::Constant(Object::Atom(s)) => {
            if let Ok(n) = s.parse::<i64>() {
                body.instruction(&Instruction::Drop);
                body.instruction(&Instruction::I64Const(n));
                body.instruction(&Instruction::Call(FN_ALLOC_ATOM));
            } else {
                // Drop x, allocate a fresh StringAtom, write bytes
                // one at a time into the payload region at offset 8.
                // The `str_slot` scratch holds the pointer across
                // the byte-writes; per-byte stores use i32.store8
                // with unaligned access (align=0) since the payload
                // is a byte array with no alignment guarantee.
                let str_slot = next_scratch;
                body.instruction(&Instruction::Drop);
                body.instruction(&Instruction::I32Const(s.len() as i32));
                body.instruction(&Instruction::Call(FN_ALLOC_STRING_ATOM));
                body.instruction(&Instruction::LocalSet(str_slot));
                for (i, byte) in s.as_bytes().iter().enumerate() {
                    body.instruction(&Instruction::LocalGet(str_slot));
                    body.instruction(&Instruction::I32Const(*byte as i32));
                    body.instruction(&Instruction::I32Store8(MemArg {
                        offset: (SEQ_HEADER_SIZE as u64) + i as u64,
                        align: 0,
                        memory_index: 0,
                    }));
                }
                body.instruction(&Instruction::LocalGet(str_slot));
            }
        }

        // Backus §11.2.4 Composition. Stack discipline makes this
        // concatenation: emit g (consumes input, leaves g(x)) then
        // emit f (consumes g(x), leaves f(g(x))).
        Func::Compose(f, g) => {
            emit_body(g, body, next_scratch, div_i64_slot)?;
            emit_body(f, body, next_scratch, div_i64_slot)?;
        }

        // Backus §11.2.4 Condition: (p → f; g):x = if p:x then f:x else g:x.
        //
        // p consumes x and leaves a predicate Object pointer. But f
        // or g needs x *again*, so we stash x in a scratch local
        // before running p, and restore it into the chosen branch.
        Func::Condition(p, f, g) => {
            let my = next_scratch;
            body.instruction(&Instruction::LocalTee(my));
            emit_body(p, body, my + 1, div_i64_slot)?;
            body.instruction(&Instruction::Call(FN_TRUTHY));
            body.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
            body.instruction(&Instruction::LocalGet(my));
            emit_body(f, body, my + 1, div_i64_slot)?;
            body.instruction(&Instruction::Else);
            body.instruction(&Instruction::LocalGet(my));
            emit_body(g, body, my + 1, div_i64_slot)?;
            body.instruction(&Instruction::End);
        }

        // Backus §11.2.4 Construction: <CONS, f₁, …, fₙ>:x = <f₁:x, …, fₙ:x>.
        //
        // Allocate a Seq of length n, then for each child:
        //   a. push x_ptr onto the stack (from scratch[x_slot]);
        //   b. emit the child (consumes x_ptr, leaves result ptr);
        //   c. store the result ptr at seq_ptr + header + 4·i.
        // Leave seq_ptr on the stack as the Construction's result.
        //
        // Scratch usage:
        //   x_slot   = next_scratch     : input x ptr (survives all children)
        //   seq_slot = next_scratch + 1 : result seq ptr (while filling)
        //   ≥ seq_slot + 1              : free for children
        Func::Construction(children) => {
            let x_slot = next_scratch;
            let seq_slot = next_scratch + 1;
            let n = children.len() as i32;
            // Stash x.
            body.instruction(&Instruction::LocalSet(x_slot));
            // Allocate the Seq object: header + n element slots.
            body.instruction(&Instruction::I32Const(SEQ_HEADER_SIZE + 4 * n));
            body.instruction(&Instruction::Call(FN_ALLOC));
            body.instruction(&Instruction::LocalTee(seq_slot));
            // tag = SEQ at offset 0
            body.instruction(&Instruction::I32Const(TAG_SEQ));
            body.instruction(&Instruction::I32Store(i32_at(0)));
            // length = n at offset 4
            body.instruction(&Instruction::LocalGet(seq_slot));
            body.instruction(&Instruction::I32Const(n));
            body.instruction(&Instruction::I32Store(i32_at(4)));
            // Fill each element slot.
            for (i, child) in children.iter().enumerate() {
                let elem_offset = (SEQ_HEADER_SIZE + 4 * i as i32) as u64;
                // push seq_ptr (address for the upcoming i32.store)
                body.instruction(&Instruction::LocalGet(seq_slot));
                // push x_ptr as the child's input
                body.instruction(&Instruction::LocalGet(x_slot));
                emit_body(child, body, seq_slot + 1, div_i64_slot)?;
                // Stack: [seq_ptr, child_result_ptr]
                body.instruction(&Instruction::I32Store(MemArg {
                    offset: elem_offset,
                    align: 2,
                    memory_index: 0,
                }));
            }
            // Leave seq_ptr on the stack.
            body.instruction(&Instruction::LocalGet(seq_slot));
        }

        // Backus §11.2.4 Apply-to-all: (αf):<x₁, …, xₙ> = <f:x₁, …, f:xₙ>.
        //
        // Input is a Seq pointer on the stack. We allocate an output
        // Seq of the same length and loop over the input, applying f
        // to each element and storing the result. The loop is a
        // standard WASM block/loop with br_if as the exit guard.
        //
        // Scratch usage:
        //   i_slot   = next_scratch     : loop index
        //   len_slot = next_scratch + 1 : input length (read once)
        //   in_slot  = next_scratch + 2 : input seq ptr
        //   out_slot = next_scratch + 3 : output seq ptr (also result)
        //   ≥ next_scratch + 4          : free for child f
        //
        // Empty-seq input: len = 0, the br_if fires on the first
        // iteration, and we return the freshly allocated 8-byte Seq.
        Func::ApplyToAll(f) => {
            let i_slot = next_scratch;
            let len_slot = next_scratch + 1;
            let in_slot = next_scratch + 2;
            let out_slot = next_scratch + 3;
            // Stash input seq ptr.
            body.instruction(&Instruction::LocalSet(in_slot));
            // len = in_seq.length
            body.instruction(&Instruction::LocalGet(in_slot));
            body.instruction(&Instruction::I32Load(i32_at(4)));
            body.instruction(&Instruction::LocalSet(len_slot));
            // Alloc output: header + 4 * len.
            body.instruction(&Instruction::I32Const(SEQ_HEADER_SIZE));
            body.instruction(&Instruction::LocalGet(len_slot));
            body.instruction(&Instruction::I32Const(4));
            body.instruction(&Instruction::I32Mul);
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::Call(FN_ALLOC));
            body.instruction(&Instruction::LocalTee(out_slot));
            // Write tag = SEQ at offset 0 (out_slot still on stack from tee).
            body.instruction(&Instruction::I32Const(TAG_SEQ));
            body.instruction(&Instruction::I32Store(i32_at(0)));
            // Write length at offset 4.
            body.instruction(&Instruction::LocalGet(out_slot));
            body.instruction(&Instruction::LocalGet(len_slot));
            body.instruction(&Instruction::I32Store(i32_at(4)));
            // i = 0
            body.instruction(&Instruction::I32Const(0));
            body.instruction(&Instruction::LocalSet(i_slot));
            // Loop over i in 0..len.
            body.instruction(&Instruction::Block(BlockType::Empty));
            body.instruction(&Instruction::Loop(BlockType::Empty));
            // if i >= len: break (depth 1 → exits the Block).
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::LocalGet(len_slot));
            body.instruction(&Instruction::I32GeS);
            body.instruction(&Instruction::BrIf(1));
            // Push store addr = out_seq + 4*i; the I32Store below
            // adds its own SEQ_HEADER_SIZE offset to land at elem i.
            body.instruction(&Instruction::LocalGet(out_slot));
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::I32Const(4));
            body.instruction(&Instruction::I32Mul);
            body.instruction(&Instruction::I32Add);
            // Push child input = in_seq elem i (load at in_seq + 4*i + 8).
            body.instruction(&Instruction::LocalGet(in_slot));
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::I32Const(4));
            body.instruction(&Instruction::I32Mul);
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::I32Load(i32_at(SEQ_HEADER_SIZE as u64)));
            // emit f : consumes elem ptr, leaves result ptr.
            emit_body(f, body, next_scratch + 4, div_i64_slot)?;
            // Store result at out_seq + 4*i + 8.
            body.instruction(&Instruction::I32Store(i32_at(SEQ_HEADER_SIZE as u64)));
            // i += 1
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::I32Const(1));
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::LocalSet(i_slot));
            // continue (depth 0 → back to loop start).
            body.instruction(&Instruction::Br(0));
            body.instruction(&Instruction::End); // end loop
            body.instruction(&Instruction::End); // end block
            // Leave out_seq on stack.
            body.instruction(&Instruction::LocalGet(out_slot));
        }

        // AREST.tex eq. (filter-def): Filter(p) ≡ compact ∘ α(p → id; ⊥).
        // Keep only elements where p is truthy.
        //
        // Strategy: over-allocate an output Seq at the upper bound
        // (len of input), write tag immediately, then loop and
        // append kept elements. After the loop, patch the length
        // field to the kept count. The heap tail beyond the length
        // is leaked but correct — a future compact pass can reclaim
        // by decrementing heap_ptr. PoC-level.
        //
        // Scratch usage:
        //   i_slot    = next_scratch     : loop index
        //   len_slot  = next_scratch + 1 : input length
        //   in_slot   = next_scratch + 2 : input seq ptr
        //   out_slot  = next_scratch + 3 : output seq ptr
        //   kept_slot = next_scratch + 4 : count of kept elements
        //   elem_slot = next_scratch + 5 : current element ptr (stashed
        //                                  so both p(elem) and the
        //                                  truthy-branch store can see it)
        //   ≥ next_scratch + 6           : free for child p
        Func::Filter(p) => {
            let i_slot = next_scratch;
            let len_slot = next_scratch + 1;
            let in_slot = next_scratch + 2;
            let out_slot = next_scratch + 3;
            let kept_slot = next_scratch + 4;
            let elem_slot = next_scratch + 5;
            // Stash input seq ptr.
            body.instruction(&Instruction::LocalSet(in_slot));
            // len = input.length
            body.instruction(&Instruction::LocalGet(in_slot));
            body.instruction(&Instruction::I32Load(i32_at(4)));
            body.instruction(&Instruction::LocalSet(len_slot));
            // Alloc upper bound: header + 4 * len.
            body.instruction(&Instruction::I32Const(SEQ_HEADER_SIZE));
            body.instruction(&Instruction::LocalGet(len_slot));
            body.instruction(&Instruction::I32Const(4));
            body.instruction(&Instruction::I32Mul);
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::Call(FN_ALLOC));
            body.instruction(&Instruction::LocalTee(out_slot));
            // tag = SEQ (still have out_slot on stack from tee).
            body.instruction(&Instruction::I32Const(TAG_SEQ));
            body.instruction(&Instruction::I32Store(i32_at(0)));
            // kept = 0, i = 0
            body.instruction(&Instruction::I32Const(0));
            body.instruction(&Instruction::LocalSet(kept_slot));
            body.instruction(&Instruction::I32Const(0));
            body.instruction(&Instruction::LocalSet(i_slot));
            // Loop
            body.instruction(&Instruction::Block(BlockType::Empty));
            body.instruction(&Instruction::Loop(BlockType::Empty));
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::LocalGet(len_slot));
            body.instruction(&Instruction::I32GeS);
            body.instruction(&Instruction::BrIf(1));
            // Load elem_ptr = in_seq[i]; stash to elem_slot and keep
            // on stack for p to consume.
            body.instruction(&Instruction::LocalGet(in_slot));
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::I32Const(4));
            body.instruction(&Instruction::I32Mul);
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::I32Load(i32_at(SEQ_HEADER_SIZE as u64)));
            body.instruction(&Instruction::LocalTee(elem_slot));
            // emit p : consumes elem ptr, leaves predicate Object ptr.
            emit_body(p, body, next_scratch + 6, div_i64_slot)?;
            body.instruction(&Instruction::Call(FN_TRUTHY));
            body.instruction(&Instruction::If(BlockType::Empty));
            // Store elem at out[kept].
            body.instruction(&Instruction::LocalGet(out_slot));
            body.instruction(&Instruction::LocalGet(kept_slot));
            body.instruction(&Instruction::I32Const(4));
            body.instruction(&Instruction::I32Mul);
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::LocalGet(elem_slot));
            body.instruction(&Instruction::I32Store(i32_at(SEQ_HEADER_SIZE as u64)));
            // kept += 1
            body.instruction(&Instruction::LocalGet(kept_slot));
            body.instruction(&Instruction::I32Const(1));
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::LocalSet(kept_slot));
            body.instruction(&Instruction::End); // end if
            // i += 1
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::I32Const(1));
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::LocalSet(i_slot));
            body.instruction(&Instruction::Br(0));
            body.instruction(&Instruction::End); // end loop
            body.instruction(&Instruction::End); // end block
            // Patch length: out.length = kept
            body.instruction(&Instruction::LocalGet(out_slot));
            body.instruction(&Instruction::LocalGet(kept_slot));
            body.instruction(&Instruction::I32Store(i32_at(4)));
            // Leave out_seq on stack.
            body.instruction(&Instruction::LocalGet(out_slot));
        }

        // Backus §11.2.4 Insert: (/f):<x₁, …, xₙ> left-folds the Seq
        // with the binary function f.
        //
        //   /f:<>              = φ (null ptr; Backus specifies e(f),
        //                           an identity we don't know at
        //                           compile time — use phi for PoC)
        //   /f:<x>             = x
        //   /f:<x₁, …, xₙ>     = fold: acc = x₁; for i in 1..n:
        //                        acc = f:<acc, xᵢ>
        //
        // Each iteration allocates a fresh 2-element Seq (the "pair")
        // to feed to f. The pair lives on the heap arena and is never
        // reclaimed within a single apply — fine because the arena
        // resets on the next call.
        //
        // Scratch usage:
        //   i_slot    = next_scratch     : loop index (starts at 1)
        //   len_slot  = next_scratch + 1 : input length
        //   in_slot   = next_scratch + 2 : input seq ptr
        //   acc_slot  = next_scratch + 3 : accumulator ptr
        //   pair_slot = next_scratch + 4 : scratch 2-elem Seq ptr
        //   ≥ next_scratch + 5           : free for child f
        //
        // Note: len==1 is handled naturally — we init acc to in[0]
        // and start the loop with i=1, which fails the i<len guard
        // and falls through, returning acc.
        Func::Insert(f) => {
            let i_slot = next_scratch;
            let len_slot = next_scratch + 1;
            let in_slot = next_scratch + 2;
            let acc_slot = next_scratch + 3;
            let pair_slot = next_scratch + 4;
            // Stash input, read length.
            body.instruction(&Instruction::LocalSet(in_slot));
            body.instruction(&Instruction::LocalGet(in_slot));
            body.instruction(&Instruction::I32Load(i32_at(4)));
            body.instruction(&Instruction::LocalSet(len_slot));
            // len == 0 → return phi; else run the fold and return acc.
            body.instruction(&Instruction::LocalGet(len_slot));
            body.instruction(&Instruction::I32Eqz);
            body.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
            body.instruction(&Instruction::I32Const(0)); // phi
            body.instruction(&Instruction::Else);
            // acc = in[0]
            body.instruction(&Instruction::LocalGet(in_slot));
            body.instruction(&Instruction::I32Load(i32_at(SEQ_HEADER_SIZE as u64)));
            body.instruction(&Instruction::LocalSet(acc_slot));
            // i = 1
            body.instruction(&Instruction::I32Const(1));
            body.instruction(&Instruction::LocalSet(i_slot));
            // Loop: for i in 1..len, acc = f:<acc, in[i]>.
            body.instruction(&Instruction::Block(BlockType::Empty));
            body.instruction(&Instruction::Loop(BlockType::Empty));
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::LocalGet(len_slot));
            body.instruction(&Instruction::I32GeS);
            body.instruction(&Instruction::BrIf(1));
            // Allocate a 2-element pair. 8 header + 4 * 2 elements = 16 bytes.
            body.instruction(&Instruction::I32Const(SEQ_HEADER_SIZE + 8));
            body.instruction(&Instruction::Call(FN_ALLOC));
            body.instruction(&Instruction::LocalTee(pair_slot));
            // tag = SEQ
            body.instruction(&Instruction::I32Const(TAG_SEQ));
            body.instruction(&Instruction::I32Store(i32_at(0)));
            // length = 2
            body.instruction(&Instruction::LocalGet(pair_slot));
            body.instruction(&Instruction::I32Const(2));
            body.instruction(&Instruction::I32Store(i32_at(4)));
            // pair[0] = acc
            body.instruction(&Instruction::LocalGet(pair_slot));
            body.instruction(&Instruction::LocalGet(acc_slot));
            body.instruction(&Instruction::I32Store(i32_at(SEQ_HEADER_SIZE as u64)));
            // pair[1] = in[i]  (load in_seq + 8 + 4*i)
            body.instruction(&Instruction::LocalGet(pair_slot));
            body.instruction(&Instruction::LocalGet(in_slot));
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::I32Const(4));
            body.instruction(&Instruction::I32Mul);
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::I32Load(i32_at(SEQ_HEADER_SIZE as u64)));
            body.instruction(&Instruction::I32Store(i32_at((SEQ_HEADER_SIZE + 4) as u64)));
            // acc = f(pair)
            body.instruction(&Instruction::LocalGet(pair_slot));
            emit_body(f, body, next_scratch + 5, div_i64_slot)?;
            body.instruction(&Instruction::LocalSet(acc_slot));
            // i += 1
            body.instruction(&Instruction::LocalGet(i_slot));
            body.instruction(&Instruction::I32Const(1));
            body.instruction(&Instruction::I32Add);
            body.instruction(&Instruction::LocalSet(i_slot));
            body.instruction(&Instruction::Br(0));
            body.instruction(&Instruction::End); // end loop
            body.instruction(&Instruction::End); // end block
            // Return acc.
            body.instruction(&Instruction::LocalGet(acc_slot));
            body.instruction(&Instruction::End); // end if/else
        }

        other => return Err(format!("wasm_lower: variant not yet supported: {:?}",
            std::mem::discriminant(other))),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast;
    use wasmi::{Caller, Engine, Linker, Module as WiModule, Store};

    /// Bind the five host-import functions as φ-returning stubs.
    /// Tests that don't exercise D-access (the majority) use these so
    /// the module still instantiates. Tests that do hit Fetch / Store
    /// etc. use `invoke_with_host` below, which swaps the stubs for
    /// real handlers that dispatch against an `ast::Object` D.
    fn bind_phi_stubs<T: 'static>(linker: &mut Linker<T>) {
        linker.func_wrap("arest", "cell_fetch",
            |_: Caller<'_, T>, _name: i32| -> i32 { 0 }).unwrap();
        linker.func_wrap("arest", "cell_fetch_or_phi",
            |_: Caller<'_, T>, _name: i32| -> i32 { 0 }).unwrap();
        linker.func_wrap("arest", "cell_store",
            |_: Caller<'_, T>, _name: i32, val: i32| -> i32 { val }).unwrap();
        linker.func_wrap("arest", "def_dispatch",
            |_: Caller<'_, T>, _name: i32, _x: i32| -> i32 { 0 }).unwrap();
        linker.func_wrap("arest", "platform_dispatch",
            |_: Caller<'_, T>, _name: i32, _x: i32| -> i32 { 0 }).unwrap();
    }

    /// Instantiate the module and invoke `apply(input)`, returning
    /// the result pointer and a snapshot of linear memory that
    /// callers decode at their own cadence. Host imports are bound
    /// to φ-returning stubs — sufficient for tests that don't
    /// exercise D-access.
    fn invoke(func: &Func, input: i64) -> (u32, Vec<u8>) {
        let bytes = lower_to_wasm(func).expect("lower must succeed for supported variants");
        let engine = Engine::default();
        let module = WiModule::new(&engine, &bytes[..]).expect("emitted WASM must validate");
        let mut store: Store<()> = Store::new(&engine, ());
        let mut linker: Linker<()> = Linker::new(&engine);
        bind_phi_stubs(&mut linker);
        let instance = linker
            .instantiate_and_start(&mut store, &module)
            .expect("module must instantiate and start");
        let apply = instance
            .get_typed_func::<i64, i32>(&store, "apply")
            .expect("exported `apply` must exist with (i64) -> i32 signature");
        let memory = instance
            .get_memory(&store, "memory")
            .expect("exported `memory` must exist");
        let ptr = apply.call(&mut store, input).expect("apply must invoke");
        let data = memory.data(&store).to_vec();
        (ptr as u32, data)
    }

    /// Invoke and decode the result as an Atom (tag=0). Returns the
    /// underlying i64 value. Panics if the tag is not ATOM.
    fn roundtrip(func: &Func, input: i64) -> i64 {
        let (ptr, data) = invoke(func, input);
        let tag = read_u32(&data, ptr);
        assert_eq!(tag, TAG_ATOM as u32,
            "expected Atom tag=0 at ptr={} but saw tag={}", ptr, tag);
        read_i64(&data, ptr + 8)
    }

    /// Invoke and decode the result as a Seq of Atoms. Returns the
    /// element i64s in order. Panics if the outer tag is not SEQ or
    /// any element tag is not ATOM.
    fn roundtrip_seq(func: &Func, input: i64) -> Vec<i64> {
        let (ptr, data) = invoke(func, input);
        let tag = read_u32(&data, ptr);
        assert_eq!(tag, TAG_SEQ as u32,
            "expected Seq tag=1 at ptr={} but saw tag={}", ptr, tag);
        let len = read_u32(&data, ptr + 4);
        (0..len)
            .map(|i| {
                let elem_ptr = read_u32(&data, ptr + 8 + 4 * i);
                let elem_tag = read_u32(&data, elem_ptr);
                assert_eq!(elem_tag, TAG_ATOM as u32,
                    "expected Atom inside Seq at ptr={} but saw tag={}", elem_ptr, elem_tag);
                read_i64(&data, elem_ptr + 8)
            })
            .collect()
    }

    fn read_u32(data: &[u8], offset: u32) -> u32 {
        u32::from_le_bytes(data[offset as usize..(offset + 4) as usize].try_into().unwrap())
    }
    fn read_i64(data: &[u8], offset: u32) -> i64 {
        i64::from_le_bytes(data[offset as usize..(offset + 8) as usize].try_into().unwrap())
    }

    // ── Host-import test harness ────────────────────────────────────

    /// Decode an Object from linear memory at `ptr`. `ptr == 0` is
    /// treated as φ (empty Seq) per the PoC's null-pointer convention.
    /// Recurses on Seq children.
    fn read_object_from_memory(data: &[u8], ptr: u32) -> ast::Object {
        if ptr == 0 {
            return ast::Object::phi();
        }
        let tag = read_u32(data, ptr);
        match tag {
            0 => ast::Object::atom(&read_i64(data, ptr + 8).to_string()),
            1 => {
                let len = read_u32(data, ptr + 4);
                let items: Vec<ast::Object> = (0..len)
                    .map(|i| read_object_from_memory(data, read_u32(data, ptr + 8 + 4 * i)))
                    .collect();
                ast::Object::seq(items)
            }
            2 => {
                let n = read_u32(data, ptr + 4) as usize;
                let bytes = &data[(ptr + 8) as usize..(ptr + 8) as usize + n];
                ast::Object::atom(&String::from_utf8_lossy(bytes))
            }
            _ => ast::Object::Bottom,
        }
    }

    /// Encode an Object back into the module's linear memory by
    /// calling the exported alloc helpers. Returns the pointer.
    /// Non-encodable shapes (Map, Bottom) return 0 = φ.
    fn write_object_to_memory(
        caller: &mut Caller<'_, HostState>,
        obj: &ast::Object,
    ) -> i32 {
        match obj {
            ast::Object::Bottom => 0,
            ast::Object::Atom(s) => {
                if let Ok(n) = s.parse::<i64>() {
                    let f = caller.get_export("alloc_atom")
                        .and_then(|e| e.into_func())
                        .expect("alloc_atom export");
                    let typed = f.typed::<i64, i32>(&*caller)
                        .expect("alloc_atom type");
                    typed.call(&mut *caller, n).expect("alloc_atom call")
                } else {
                    let f = caller.get_export("alloc_string_atom")
                        .and_then(|e| e.into_func())
                        .expect("alloc_string_atom export");
                    let typed = f.typed::<i32, i32>(&*caller)
                        .expect("alloc_string_atom type");
                    let ptr = typed.call(&mut *caller, s.len() as i32)
                        .expect("alloc_string_atom call");
                    let memory = caller.get_export("memory")
                        .and_then(|e| e.into_memory())
                        .expect("memory export");
                    memory.write(&mut *caller, (ptr + 8) as usize, s.as_bytes())
                        .expect("memory write (string payload)");
                    ptr
                }
            }
            ast::Object::Seq(items) => {
                // Recurse first — each child needs its own allocation
                // before we can write the parent Seq's element ptrs.
                let child_ptrs: Vec<i32> = items
                    .iter()
                    .map(|c| write_object_to_memory(caller, c))
                    .collect();
                let alloc = caller.get_export("alloc")
                    .and_then(|e| e.into_func())
                    .expect("alloc export");
                let typed = alloc.typed::<i32, i32>(&*caller).expect("alloc type");
                let total = SEQ_HEADER_SIZE + 4 * items.len() as i32;
                let ptr = typed.call(&mut *caller, total).expect("alloc call");
                let memory = caller.get_export("memory")
                    .and_then(|e| e.into_memory())
                    .expect("memory export");
                memory.write(&mut *caller, ptr as usize,
                    &(TAG_SEQ as u32).to_le_bytes()).unwrap();
                memory.write(&mut *caller, (ptr + 4) as usize,
                    &(items.len() as u32).to_le_bytes()).unwrap();
                for (i, cptr) in child_ptrs.iter().enumerate() {
                    let off = (ptr + SEQ_HEADER_SIZE + 4 * i as i32) as usize;
                    memory.write(&mut *caller, off,
                        &(*cptr as u32).to_le_bytes()).unwrap();
                }
                ptr
            }
            ast::Object::Map(_) => 0,
        }
    }

    /// Thin HostState for the tests: a named-cell store. Production
    /// hosts would plug in CompiledState or whatever context their
    /// runtime carries.
    struct HostState {
        cells: hashbrown::HashMap<String, ast::Object>,
    }

    /// Build a synthetic `&Object` D for `ast::apply` from the
    /// cells map, so def_dispatch / platform_dispatch can run
    /// `ast::apply(Func::Def(name), x, &d)` with realistic state.
    fn build_d_from_cells(
        cells: &hashbrown::HashMap<String, ast::Object>,
    ) -> ast::Object {
        ast::Object::Map(cells.clone())
    }

    /// Decode a name StringAtom (or numeric atom) at `ptr` and return
    /// its text form. Returns None if the Object at `ptr` isn't an
    /// atom shape.
    fn name_from_memory(data: &[u8], ptr: u32) -> Option<String> {
        let obj = read_object_from_memory(data, ptr);
        obj.as_atom().map(|s| s.to_string())
    }

    /// Instantiate with real Fetch / FetchOrPhi / Store / Def /
    /// Platform handlers that read and write a `HashMap<String,
    /// Object>` held in `HostState`. Returns the result pointer, a
    /// memory snapshot, and the final cells map (so Store can be
    /// asserted against).
    fn invoke_with_host(
        func: &Func,
        input: i64,
        initial_cells: hashbrown::HashMap<String, ast::Object>,
    ) -> (u32, Vec<u8>, hashbrown::HashMap<String, ast::Object>) {
        let bytes = lower_to_wasm(func).expect("lower must succeed");
        let engine = Engine::default();
        let module = WiModule::new(&engine, &bytes[..]).expect("validate");
        let mut store: Store<HostState> = Store::new(&engine, HostState { cells: initial_cells });
        let mut linker: Linker<HostState> = Linker::new(&engine);

        linker.func_wrap("arest", "cell_fetch",
            |mut caller: Caller<'_, HostState>, name_ptr: i32| -> i32 {
                let memory = caller.get_export("memory")
                    .and_then(|e| e.into_memory()).unwrap();
                let name = {
                    let data = memory.data(&caller);
                    name_from_memory(data, name_ptr as u32)
                };
                let Some(name) = name else { return 0; };
                let value = caller.data().cells.get(&name).cloned()
                    .unwrap_or(ast::Object::Bottom);
                write_object_to_memory(&mut caller, &value)
            }).unwrap();

        linker.func_wrap("arest", "cell_fetch_or_phi",
            |mut caller: Caller<'_, HostState>, name_ptr: i32| -> i32 {
                let memory = caller.get_export("memory")
                    .and_then(|e| e.into_memory()).unwrap();
                let name = {
                    let data = memory.data(&caller);
                    name_from_memory(data, name_ptr as u32)
                };
                let Some(name) = name else { return 0; };
                let value = caller.data().cells.get(&name).cloned()
                    .unwrap_or_else(ast::Object::phi);
                write_object_to_memory(&mut caller, &value)
            }).unwrap();

        linker.func_wrap("arest", "cell_store",
            |mut caller: Caller<'_, HostState>, name_ptr: i32, val_ptr: i32| -> i32 {
                let memory = caller.get_export("memory")
                    .and_then(|e| e.into_memory()).unwrap();
                let (name, value) = {
                    let data = memory.data(&caller);
                    let name = name_from_memory(data, name_ptr as u32);
                    let value = read_object_from_memory(data, val_ptr as u32);
                    (name, value)
                };
                let Some(name) = name else { return 0; };
                caller.data_mut().cells.insert(name, value);
                val_ptr // pass-through
            }).unwrap();

        linker.func_wrap("arest", "def_dispatch",
            |mut caller: Caller<'_, HostState>, name_ptr: i32, x_ptr: i32| -> i32 {
                let memory = caller.get_export("memory")
                    .and_then(|e| e.into_memory()).unwrap();
                let (name, x) = {
                    let data = memory.data(&caller);
                    let name = name_from_memory(data, name_ptr as u32);
                    let x = read_object_from_memory(data, x_ptr as u32);
                    (name, x)
                };
                let Some(name) = name else { return 0; };
                let d = build_d_from_cells(&caller.data().cells);
                let result = ast::apply(&ast::Func::Def(name), &x, &d);
                write_object_to_memory(&mut caller, &result)
            }).unwrap();

        linker.func_wrap("arest", "platform_dispatch",
            |mut caller: Caller<'_, HostState>, name_ptr: i32, x_ptr: i32| -> i32 {
                let memory = caller.get_export("memory")
                    .and_then(|e| e.into_memory()).unwrap();
                let (name, x) = {
                    let data = memory.data(&caller);
                    let name = name_from_memory(data, name_ptr as u32);
                    let x = read_object_from_memory(data, x_ptr as u32);
                    (name, x)
                };
                let Some(name) = name else { return 0; };
                let d = build_d_from_cells(&caller.data().cells);
                let result = ast::apply(&ast::Func::Platform(name), &x, &d);
                write_object_to_memory(&mut caller, &result)
            }).unwrap();

        let instance = linker.instantiate_and_start(&mut store, &module).expect("instantiate");
        let apply = instance.get_typed_func::<i64, i32>(&store, "apply").expect("apply");
        let memory = instance.get_memory(&store, "memory").expect("memory");
        let ptr = apply.call(&mut store, input).expect("apply");
        let snapshot = memory.data(&store).to_vec();
        let final_cells = store.data().cells.clone();
        (ptr as u32, snapshot, final_cells)
    }

    // ── Primitives ────────────────────────────────────────────────

    #[test]
    fn lower_id_emits_valid_module_and_returns_argument() {
        assert_eq!(roundtrip(&Func::Id, 42), 42);
        assert_eq!(roundtrip(&Func::Id, -7), -7);
        assert_eq!(roundtrip(&Func::Id, 0), 0);
    }

    #[test]
    fn lower_constant_emits_valid_module_and_returns_literal() {
        let f = Func::Constant(Object::atom("100"));
        assert_eq!(roundtrip(&f, 0), 100);
        assert_eq!(roundtrip(&f, 42), 100);
        assert_eq!(roundtrip(&f, -1), 100);
    }

    #[test]
    fn lower_constant_with_non_numeric_atom_emits_string_atom() {
        // Atoms that don't parse as i64 are emitted as StringAtom
        // (tag 2, byte length prefix, raw UTF-8 payload).
        let f = Func::Constant(Object::atom("hello"));
        let (ptr, data) = invoke(&f, 0);
        assert_eq!(read_u32(&data, ptr), TAG_STRING_ATOM as u32,
            "result is a StringAtom");
        assert_eq!(read_u32(&data, ptr + 4), 5, "byte length of \"hello\"");
        let bytes = &data[(ptr + 8) as usize..(ptr + 8 + 5) as usize];
        assert_eq!(bytes, b"hello");
    }

    #[test]
    fn lower_rejects_unsupported_variant_native() {
        // Func::Native wraps a Rust closure — truly impossible to
        // ship across the FFI boundary. Left as the only remaining
        // unsupported-variant sentinel.
        let f = Func::Native(crate::sync::Arc::new(|x| x.clone()));
        let err = lower_to_wasm(&f).expect_err("Native should be unsupported");
        assert!(err.contains("not yet supported"));
    }

    // ── Host imports: Fetch / FetchOrPhi / Store / Def / Platform ──

    #[test]
    fn lower_fetch_returns_numeric_cell_value_from_host() {
        // Host D has `pi = "314"`; Fetch("pi") returns Atom(314).
        let mut cells = hashbrown::HashMap::new();
        cells.insert("pi".to_string(), ast::Object::atom("314"));
        let f = Func::Compose(
            Box::new(Func::Fetch),
            Box::new(Func::Constant(ast::Object::atom("pi"))),
        );
        let (ptr, data, _) = invoke_with_host(&f, 0, cells);
        assert_eq!(read_u32(&data, ptr), TAG_ATOM as u32, "result is a numeric Atom");
        assert_eq!(read_i64(&data, ptr + 8), 314);
    }

    #[test]
    fn lower_fetch_returns_string_cell_value_from_host() {
        // Non-numeric cells come back as StringAtoms.
        let mut cells = hashbrown::HashMap::new();
        cells.insert("motto".to_string(), ast::Object::atom("carpe diem"));
        let f = Func::Compose(
            Box::new(Func::Fetch),
            Box::new(Func::Constant(ast::Object::atom("motto"))),
        );
        let (ptr, data, _) = invoke_with_host(&f, 0, cells);
        assert_eq!(read_u32(&data, ptr), TAG_STRING_ATOM as u32);
        let len = read_u32(&data, ptr + 4) as usize;
        let bytes = &data[(ptr + 8) as usize..(ptr + 8) as usize + len];
        assert_eq!(bytes, b"carpe diem");
    }

    #[test]
    fn lower_fetch_returns_seq_cell_value_from_host() {
        // Cells holding Seqs round-trip: each element is encoded
        // recursively as a child Atom, and a fresh outer Seq
        // collects the element ptrs.
        let mut cells = hashbrown::HashMap::new();
        cells.insert(
            "scores".to_string(),
            ast::Object::seq(vec![
                ast::Object::atom("10"),
                ast::Object::atom("20"),
                ast::Object::atom("30"),
            ]),
        );
        let f = Func::Compose(
            Box::new(Func::Fetch),
            Box::new(Func::Constant(ast::Object::atom("scores"))),
        );
        let (ptr, data, _) = invoke_with_host(&f, 0, cells);
        assert_eq!(read_u32(&data, ptr), TAG_SEQ as u32);
        assert_eq!(read_u32(&data, ptr + 4), 3);
        for (i, expected) in [10i64, 20, 30].iter().enumerate() {
            let cptr = read_u32(&data, ptr + 8 + 4 * i as u32);
            assert_eq!(read_u32(&data, cptr), TAG_ATOM as u32);
            assert_eq!(read_i64(&data, cptr + 8), *expected);
        }
    }

    #[test]
    fn lower_fetch_missing_cell_returns_null_phi() {
        // AREST's bottom-propagating Fetch returns φ as a null ptr.
        let f = Func::Compose(
            Box::new(Func::Fetch),
            Box::new(Func::Constant(ast::Object::atom("nonexistent"))),
        );
        let (ptr, _, _) = invoke_with_host(&f, 0, hashbrown::HashMap::new());
        assert_eq!(ptr, 0, "missing cell → null ptr (φ)");
    }

    #[test]
    fn lower_fetch_or_phi_missing_cell_returns_empty_seq() {
        // FetchOrPhi avoids ⊥-propagation on missing cells — returns
        // an empty Seq instead of null ptr.
        let f = Func::Compose(
            Box::new(Func::FetchOrPhi),
            Box::new(Func::Constant(ast::Object::atom("nonexistent"))),
        );
        let (ptr, data, _) = invoke_with_host(&f, 0, hashbrown::HashMap::new());
        assert_eq!(read_u32(&data, ptr), TAG_SEQ as u32,
            "miss → empty Seq, not null ptr");
        assert_eq!(read_u32(&data, ptr + 4), 0);
    }

    #[test]
    fn lower_store_writes_through_to_host_cells() {
        // Compose(Store, <name, value>) commits `value` to cell
        // `name` in the host's D. The WASM module returns the
        // value ptr as a pass-through.
        let f = Func::Compose(
            Box::new(Func::Store),
            Box::new(Func::Construction(vec![
                Func::Constant(ast::Object::atom("result")),
                Func::Constant(ast::Object::atom("42")),
            ])),
        );
        let (_ptr, _, cells) = invoke_with_host(&f, 0, hashbrown::HashMap::new());
        assert_eq!(cells.get("result"), Some(&ast::Object::atom("42")),
            "Store landed the value in the host's cells map");
    }

    #[test]
    fn lower_store_then_fetch_round_trips_through_host() {
        // A two-step compose: first store value to cell, then fetch
        // back. Proves Store's pass-through return threads into a
        // following Fetch by cell name with no intermediate leak.
        // The final WASM result is the fetched value.
        //
        // Note: Compose runs the RHS first; so this is:
        //   fetch("result") on the post-store state.
        // The Store happens inside its own composition branch.
        let store_branch = Func::Compose(
            Box::new(Func::Store),
            Box::new(Func::Construction(vec![
                Func::Constant(ast::Object::atom("result")),
                Func::Constant(ast::Object::atom("777")),
            ])),
        );
        let fetch_branch = Func::Compose(
            Box::new(Func::Fetch),
            Box::new(Func::Constant(ast::Object::atom("result"))),
        );
        // Apply store, discard, then fetch — Compose is right-to-left
        // so inner happens first.
        let f = Func::Compose(
            Box::new(fetch_branch),
            Box::new(store_branch),
        );
        let (ptr, data, cells) = invoke_with_host(&f, 0, hashbrown::HashMap::new());
        assert_eq!(cells.get("result"), Some(&ast::Object::atom("777")));
        assert_eq!(read_u32(&data, ptr), TAG_ATOM as u32);
        assert_eq!(read_i64(&data, ptr + 8), 777);
    }

    #[test]
    fn lower_def_dispatch_runs_host_side_apply() {
        // Wire a simple Def("echo_noun") at the host side by
        // pre-populating the cells map with a "DEFS" cell whose
        // echo_noun def is `Func::Id`. The WASM call:
        //   Func::Def("echo_noun") applied to x
        // dispatches to the host, which runs ast::apply on the
        // matching Func, returning x unchanged.
        //
        // For this PoC the host has a cells map, not a full DEFS
        // lookup — so we instead test by naming a Func variant
        // whose behavior doesn't actually need a real DEFS lookup:
        // Func::Def("anything_not_resolvable") against ast::apply
        // produces Object::Bottom, which we encode as null ptr.
        // The test asserts the dispatch path itself is wired —
        // that the import is called and yields SOMETHING the
        // module accepts.
        let f = Func::Compose(
            Box::new(Func::Def("not_in_defs".to_string())),
            Box::new(Func::Constant(ast::Object::atom("42"))),
        );
        let (ptr, _, _) = invoke_with_host(&f, 0, hashbrown::HashMap::new());
        // No DEFS resolution available → Bottom → ptr 0. The point
        // is that the module didn't trap; the host import fired
        // and returned a valid pointer value.
        assert_eq!(ptr, 0,
            "def_dispatch against a name with no backing Def returns φ (0)");
    }

    // ── String Atom helpers + tests ────────────────────────────────

    /// Decode the result as a StringAtom; panic on any other shape.
    fn roundtrip_string(func: &Func, input: i64) -> String {
        let (ptr, data) = invoke(func, input);
        let tag = read_u32(&data, ptr);
        assert_eq!(tag, TAG_STRING_ATOM as u32,
            "expected StringAtom tag=2 at ptr={} but saw tag={}", ptr, tag);
        let len = read_u32(&data, ptr + 4) as usize;
        let bytes = &data[(ptr + 8) as usize..(ptr + 8) as usize + len];
        String::from_utf8(bytes.to_vec())
            .expect("string atom payload must be valid UTF-8")
    }

    #[test]
    fn lower_string_constant_roundtrips() {
        // Round-trip several string literals through the emitter:
        // ASCII-only, mixed-case, UTF-8 multi-byte, and an empty
        // string. All should come back byte-identical.
        for s in ["hello", "HeLLo World", "résumé", ""] {
            let f = Func::Constant(Object::atom(s));
            let got = roundtrip_string(&f, 0);
            assert_eq!(got, s, "string constant {:?} round-tripped", s);
        }
    }

    #[test]
    fn lower_atom_test_on_string_atom_returns_one() {
        // `atom:"hello"` must report true — StringAtoms are atoms.
        // The predicate checks `tag != TAG_SEQ`, so both numeric
        // and string atoms classify as atomhood-positive.
        let f = Func::Compose(
            Box::new(Func::AtomTest),
            Box::new(Func::Constant(Object::atom("hello"))),
        );
        assert_eq!(roundtrip(&f, 0), 1);
    }

    #[test]
    fn lower_null_test_distinguishes_empty_string_from_phi() {
        // Empty-string StringAtom is still an atom, not φ.
        let empty = Func::Compose(
            Box::new(Func::NullTest),
            Box::new(Func::Constant(Object::atom(""))),
        );
        assert_eq!(roundtrip(&empty, 0), 0,
            "null:\"\" = 0 — an empty-string atom is not φ");

        // Non-empty string atom also not φ.
        let hello = Func::Compose(
            Box::new(Func::NullTest),
            Box::new(Func::Constant(Object::atom("hello"))),
        );
        assert_eq!(roundtrip(&hello, 0), 0);
    }

    #[test]
    fn lower_truthy_on_string_atom_follows_byte_length() {
        // Empty string is falsy; non-empty is truthy. Exercised via
        // Condition.
        let if_empty = Func::Compose(
            Box::new(Func::Condition(
                Box::new(Func::Id),
                Box::new(Func::Constant(Object::atom("42"))),
                Box::new(Func::Constant(Object::atom("99"))),
            )),
            Box::new(Func::Constant(Object::atom(""))),
        );
        assert_eq!(roundtrip(&if_empty, 0), 99, "empty string → false");

        let if_nonempty = Func::Compose(
            Box::new(Func::Condition(
                Box::new(Func::Id),
                Box::new(Func::Constant(Object::atom("42"))),
                Box::new(Func::Constant(Object::atom("99"))),
            )),
            Box::new(Func::Constant(Object::atom("x"))),
        );
        assert_eq!(roundtrip(&if_nonempty, 0), 42, "non-empty string → true");
    }

    // ── Contains (byte substring) ───────────────────────────────────

    #[test]
    fn lower_contains_finds_substring() {
        // contains:<"hello world", "world"> = 1
        let f = Func::Compose(
            Box::new(Func::Contains),
            Box::new(Func::Construction(vec![
                Func::Constant(Object::atom("hello world")),
                Func::Constant(Object::atom("world")),
            ])),
        );
        assert_eq!(roundtrip(&f, 0), 1);
    }

    #[test]
    fn lower_contains_returns_zero_for_absent_needle() {
        let f = Func::Compose(
            Box::new(Func::Contains),
            Box::new(Func::Construction(vec![
                Func::Constant(Object::atom("alpha beta")),
                Func::Constant(Object::atom("gamma")),
            ])),
        );
        assert_eq!(roundtrip(&f, 0), 0);
    }

    #[test]
    fn lower_contains_returns_zero_when_needle_longer_than_haystack() {
        let f = Func::Compose(
            Box::new(Func::Contains),
            Box::new(Func::Construction(vec![
                Func::Constant(Object::atom("ab")),
                Func::Constant(Object::atom("abcdef")),
            ])),
        );
        assert_eq!(roundtrip(&f, 0), 0);
    }

    #[test]
    fn lower_contains_matches_at_start_middle_and_end() {
        // Start
        let start = Func::Compose(
            Box::new(Func::Contains),
            Box::new(Func::Construction(vec![
                Func::Constant(Object::atom("prefix-rest")),
                Func::Constant(Object::atom("prefix")),
            ])),
        );
        assert_eq!(roundtrip(&start, 0), 1);
        // Middle
        let middle = Func::Compose(
            Box::new(Func::Contains),
            Box::new(Func::Construction(vec![
                Func::Constant(Object::atom("a-mid-b")),
                Func::Constant(Object::atom("mid")),
            ])),
        );
        assert_eq!(roundtrip(&middle, 0), 1);
        // End
        let end = Func::Compose(
            Box::new(Func::Contains),
            Box::new(Func::Construction(vec![
                Func::Constant(Object::atom("leading-tail")),
                Func::Constant(Object::atom("tail")),
            ])),
        );
        assert_eq!(roundtrip(&end, 0), 1);
    }

    #[test]
    fn lower_contains_whole_string_matches_itself() {
        let f = Func::Compose(
            Box::new(Func::Contains),
            Box::new(Func::Construction(vec![
                Func::Constant(Object::atom("same")),
                Func::Constant(Object::atom("same")),
            ])),
        );
        assert_eq!(roundtrip(&f, 0), 1);
    }

    // ── Lower (ASCII case fold) ─────────────────────────────────────

    #[test]
    fn lower_lowercases_ascii_string() {
        let f = Func::Compose(
            Box::new(Func::Lower),
            Box::new(Func::Constant(Object::atom("HeLLo WoRLD"))),
        );
        assert_eq!(roundtrip_string(&f, 0), "hello world");
    }

    #[test]
    fn lower_preserves_already_lowercase_and_non_letters() {
        // Punctuation, digits, and lowercase letters pass through.
        let f = Func::Compose(
            Box::new(Func::Lower),
            Box::new(Func::Constant(Object::atom("abc123-_.!@#"))),
        );
        assert_eq!(roundtrip_string(&f, 0), "abc123-_.!@#");
    }

    #[test]
    fn lower_of_empty_string_is_empty_string() {
        let f = Func::Compose(
            Box::new(Func::Lower),
            Box::new(Func::Constant(Object::atom(""))),
        );
        assert_eq!(roundtrip_string(&f, 0), "");
    }

    #[test]
    fn lower_preserves_non_ascii_utf8_bytes() {
        // Multi-byte UTF-8 sequences should pass through unchanged
        // because their bytes (all ≥ 0x80) fall outside the [A-Z]
        // ASCII case-fold range. Happens to produce valid UTF-8
        // output for any valid UTF-8 input — a freebie of byte-level
        // ASCII-only folding.
        let f = Func::Compose(
            Box::new(Func::Lower),
            Box::new(Func::Constant(Object::atom("RÉSUMÉ"))),
        );
        // The leading R → r, but É bytes (0xC3 0x89) are untouched,
        // so the result is "rÉsumÉ" not "résumé". That's the
        // documented ASCII-only contract.
        assert_eq!(roundtrip_string(&f, 0), "rÉsumÉ");
    }

    #[test]
    fn lower_composes_with_contains() {
        // contains:<lower:"Hello", "hello"> = 1 — case-insensitive
        // substring check built from two primitives.
        let f = Func::Compose(
            Box::new(Func::Contains),
            Box::new(Func::Construction(vec![
                Func::Compose(
                    Box::new(Func::Lower),
                    Box::new(Func::Constant(Object::atom("Hello"))),
                ),
                Func::Constant(Object::atom("hello")),
            ])),
        );
        assert_eq!(roundtrip(&f, 0), 1);
    }

    #[test]
    fn lower_selector_rejects_zero_index() {
        let f = Func::Selector(0);
        let err = lower_to_wasm(&f).expect_err("Selector(0) must fail at emit time");
        assert!(err.contains("≥ 1"));
    }

    // ── Compose ───────────────────────────────────────────────────

    #[test]
    fn lower_compose_emits_chained_body() {
        let f = Func::Compose(
            Box::new(Func::Constant(Object::atom("7"))),
            Box::new(Func::Id),
        );
        assert_eq!(roundtrip(&f, 42), 7);
        assert_eq!(roundtrip(&f, -1), 7);
    }

    #[test]
    fn lower_compose_of_two_ids_is_identity() {
        let f = Func::Compose(Box::new(Func::Id), Box::new(Func::Id));
        assert_eq!(roundtrip(&f, 42), 42);
        assert_eq!(roundtrip(&f, -7), -7);
    }

    #[test]
    fn lower_compose_constant_over_constant_returns_outer() {
        let f = Func::Compose(
            Box::new(Func::Constant(Object::atom("9"))),
            Box::new(Func::Constant(Object::atom("11"))),
        );
        assert_eq!(roundtrip(&f, 0), 9);
    }

    // ── Condition ─────────────────────────────────────────────────

    #[test]
    fn lower_condition_with_constant_true_predicate_takes_f_branch() {
        let f = Func::Condition(
            Box::new(Func::Constant(Object::atom("1"))),
            Box::new(Func::Constant(Object::atom("42"))),
            Box::new(Func::Constant(Object::atom("99"))),
        );
        assert_eq!(roundtrip(&f, 0), 42);
        assert_eq!(roundtrip(&f, -7), 42);
        assert_eq!(roundtrip(&f, 1_000_000), 42);
    }

    #[test]
    fn lower_condition_with_constant_false_predicate_takes_g_branch() {
        let f = Func::Condition(
            Box::new(Func::Constant(Object::atom("0"))),
            Box::new(Func::Constant(Object::atom("42"))),
            Box::new(Func::Constant(Object::atom("99"))),
        );
        assert_eq!(roundtrip(&f, 0), 99);
        assert_eq!(roundtrip(&f, 42), 99);
    }

    #[test]
    fn lower_condition_with_id_predicate_branches_on_input() {
        let f = Func::Condition(
            Box::new(Func::Id),
            Box::new(Func::Constant(Object::atom("1"))),
            Box::new(Func::Constant(Object::atom("0"))),
        );
        assert_eq!(roundtrip(&f, 42), 1);
        assert_eq!(roundtrip(&f, -1), 1);
        assert_eq!(roundtrip(&f, 0), 0);
    }

    #[test]
    fn lower_condition_restores_x_for_branch_body() {
        let f = Func::Condition(
            Box::new(Func::Id),
            Box::new(Func::Id),
            Box::new(Func::Constant(Object::atom("-1"))),
        );
        assert_eq!(roundtrip(&f, 7), 7);
        assert_eq!(roundtrip(&f, 999), 999);
        assert_eq!(roundtrip(&f, 0), -1);
    }

    #[test]
    fn lower_condition_nests_without_scratch_collision() {
        let inner = Func::Condition(
            Box::new(Func::Id),
            Box::new(Func::Constant(Object::atom("11"))),
            Box::new(Func::Constant(Object::atom("22"))),
        );
        let f = Func::Condition(
            Box::new(Func::Id),
            Box::new(inner),
            Box::new(Func::Constant(Object::atom("33"))),
        );
        assert_eq!(roundtrip(&f, 5), 11);
        assert_eq!(roundtrip(&f, -3), 11);
        assert_eq!(roundtrip(&f, 0), 33);
    }

    #[test]
    fn lower_condition_over_compose_chains_cleanly() {
        let f = Func::Compose(
            Box::new(Func::Condition(
                Box::new(Func::Id),
                Box::new(Func::Constant(Object::atom("100"))),
                Box::new(Func::Constant(Object::atom("200"))),
            )),
            Box::new(Func::Id),
        );
        assert_eq!(roundtrip(&f, 1), 100);
        assert_eq!(roundtrip(&f, 0), 200);
    }

    // ── Construction ──────────────────────────────────────────────

    #[test]
    fn lower_construction_of_two_constants_builds_seq_of_atoms() {
        // <CONS, 10, 20>:x = <10, 20> for any x.
        let f = Func::Construction(vec![
            Func::Constant(Object::atom("10")),
            Func::Constant(Object::atom("20")),
        ]);
        assert_eq!(roundtrip_seq(&f, 0), vec![10, 20]);
        assert_eq!(roundtrip_seq(&f, 42), vec![10, 20]);
    }

    #[test]
    fn lower_construction_of_id_id_pairs_x_with_itself() {
        // <CONS, id, id>:x = <x, x> — both children see the same x.
        let f = Func::Construction(vec![Func::Id, Func::Id]);
        assert_eq!(roundtrip_seq(&f, 7), vec![7, 7]);
        assert_eq!(roundtrip_seq(&f, -99), vec![-99, -99]);
    }

    #[test]
    fn lower_construction_empty_returns_empty_seq() {
        // <CONS>:x = <> — Seq of length 0. Valid but atypical.
        let f = Func::Construction(vec![]);
        assert_eq!(roundtrip_seq(&f, 42), Vec::<i64>::new());
    }

    #[test]
    fn lower_construction_mixed_id_and_constant() {
        // <CONS, id, 100, id>:x = <x, 100, x>.
        let f = Func::Construction(vec![
            Func::Id,
            Func::Constant(Object::atom("100")),
            Func::Id,
        ]);
        assert_eq!(roundtrip_seq(&f, 7), vec![7, 100, 7]);
    }

    #[test]
    fn lower_construction_nested_builds_seq_of_seqs() {
        // <CONS, <CONS, id, id>, id>:x outer is a Seq whose first
        // element is itself a Seq. The outer roundtrip helper only
        // decodes two levels of Atoms, so this test just verifies the
        // tag layout directly.
        let inner = Func::Construction(vec![Func::Id, Func::Id]);
        let f = Func::Construction(vec![inner, Func::Id]);
        let (ptr, data) = invoke(&f, 5);
        assert_eq!(read_u32(&data, ptr), TAG_SEQ as u32);
        assert_eq!(read_u32(&data, ptr + 4), 2, "outer length");
        let inner_ptr = read_u32(&data, ptr + 8);
        assert_eq!(read_u32(&data, inner_ptr), TAG_SEQ as u32, "inner tag");
        assert_eq!(read_u32(&data, inner_ptr + 4), 2, "inner length");
        let elem0_ptr = read_u32(&data, inner_ptr + 8);
        assert_eq!(read_i64(&data, elem0_ptr + 8), 5);
    }

    #[test]
    fn lower_compose_of_construction_and_id_builds_seq() {
        // (<CONS, id, id> ∘ id):x = <x, x>.
        let f = Func::Compose(
            Box::new(Func::Construction(vec![Func::Id, Func::Id])),
            Box::new(Func::Id),
        );
        assert_eq!(roundtrip_seq(&f, 3), vec![3, 3]);
    }

    // ── ApplyToAll ────────────────────────────────────────────────

    #[test]
    fn lower_apply_to_all_constant_produces_uniform_seq() {
        // (α(Constant 1) ∘ <id, id>):x = <1, 1> for any x.
        let f = Func::Compose(
            Box::new(Func::ApplyToAll(Box::new(Func::Constant(Object::atom("1"))))),
            Box::new(Func::Construction(vec![Func::Id, Func::Id])),
        );
        assert_eq!(roundtrip_seq(&f, 42), vec![1, 1]);
        assert_eq!(roundtrip_seq(&f, 0), vec![1, 1]);
    }

    #[test]
    fn lower_apply_to_all_id_is_identity_on_seq() {
        // (α(id) ∘ <id, id>):x = <x, x>. Verifies the loop faithfully
        // threads element ptrs through without corruption.
        let f = Func::Compose(
            Box::new(Func::ApplyToAll(Box::new(Func::Id))),
            Box::new(Func::Construction(vec![Func::Id, Func::Id])),
        );
        assert_eq!(roundtrip_seq(&f, 7), vec![7, 7]);
        assert_eq!(roundtrip_seq(&f, -3), vec![-3, -3]);
    }

    #[test]
    fn lower_apply_to_all_empty_seq_short_circuits() {
        // α(f):<> = <> — br_if fires on the first iteration with i=0
        // and len=0, exiting cleanly.
        let f = Func::Compose(
            Box::new(Func::ApplyToAll(Box::new(Func::Constant(Object::atom("99"))))),
            Box::new(Func::Construction(vec![])),
        );
        assert_eq!(roundtrip_seq(&f, 42), Vec::<i64>::new());
    }

    #[test]
    fn lower_apply_to_all_over_four_elements() {
        // α(Constant 5) ∘ <id, id, id, id>:x = <5, 5, 5, 5>.
        // Confirms the loop iterates the correct number of times —
        // if br_if fires too early we'd see a shorter Seq; if too
        // late we'd trap on out-of-bounds load.
        let f = Func::Compose(
            Box::new(Func::ApplyToAll(Box::new(Func::Constant(Object::atom("5"))))),
            Box::new(Func::Construction(vec![
                Func::Id, Func::Id, Func::Id, Func::Id,
            ])),
        );
        assert_eq!(roundtrip_seq(&f, 42), vec![5, 5, 5, 5]);
    }

    #[test]
    fn lower_apply_to_all_preserves_seq_shape() {
        // α(id) ∘ <10, 20, 30>:x — result must be a Seq, tag=1,
        // length=3, elements are Atoms [10, 20, 30] regardless of x.
        let f = Func::Compose(
            Box::new(Func::ApplyToAll(Box::new(Func::Id))),
            Box::new(Func::Construction(vec![
                Func::Constant(Object::atom("10")),
                Func::Constant(Object::atom("20")),
                Func::Constant(Object::atom("30")),
            ])),
        );
        let (ptr, data) = invoke(&f, 0);
        assert_eq!(read_u32(&data, ptr), TAG_SEQ as u32, "result is a Seq");
        assert_eq!(read_u32(&data, ptr + 4), 3, "length=3");
        assert_eq!(roundtrip_seq(&f, 0), vec![10, 20, 30]);
    }

    // ── Filter ────────────────────────────────────────────────────

    #[test]
    fn lower_filter_constant_true_predicate_keeps_all() {
        // Filter(Constant 1) ∘ <10, 20, 30>:x = <10, 20, 30>.
        let f = Func::Compose(
            Box::new(Func::Filter(Box::new(Func::Constant(Object::atom("1"))))),
            Box::new(Func::Construction(vec![
                Func::Constant(Object::atom("10")),
                Func::Constant(Object::atom("20")),
                Func::Constant(Object::atom("30")),
            ])),
        );
        assert_eq!(roundtrip_seq(&f, 0), vec![10, 20, 30]);
    }

    #[test]
    fn lower_filter_constant_false_predicate_keeps_none() {
        // Filter(Constant 0) ∘ <10, 20>:x = <>. length patches to 0.
        let f = Func::Compose(
            Box::new(Func::Filter(Box::new(Func::Constant(Object::atom("0"))))),
            Box::new(Func::Construction(vec![
                Func::Constant(Object::atom("10")),
                Func::Constant(Object::atom("20")),
            ])),
        );
        assert_eq!(roundtrip_seq(&f, 0), Vec::<i64>::new());
    }

    #[test]
    fn lower_filter_id_keeps_nonzero_atoms() {
        // Filter(Id) ∘ <0, 7, 0, 5, 0>:x = <7, 5>.
        // Id passes the Atom through to truthy, which checks value != 0.
        // Exercises partial-keep + length patch-back to 2.
        let f = Func::Compose(
            Box::new(Func::Filter(Box::new(Func::Id))),
            Box::new(Func::Construction(vec![
                Func::Constant(Object::atom("0")),
                Func::Constant(Object::atom("7")),
                Func::Constant(Object::atom("0")),
                Func::Constant(Object::atom("5")),
                Func::Constant(Object::atom("0")),
            ])),
        );
        assert_eq!(roundtrip_seq(&f, 0), vec![7, 5]);
    }

    #[test]
    fn lower_filter_empty_seq() {
        // Filter(p):<> = <> regardless of p.
        let f = Func::Compose(
            Box::new(Func::Filter(Box::new(Func::Id))),
            Box::new(Func::Construction(vec![])),
        );
        assert_eq!(roundtrip_seq(&f, 0), Vec::<i64>::new());
    }

    // ── Selector ──────────────────────────────────────────────────

    #[test]
    fn lower_selector_first_of_pair() {
        // s₁:<x, y> = x. Compose with Construction to produce the
        // pair from the apply's i64 input.
        let f = Func::Compose(
            Box::new(Func::Selector(1)),
            Box::new(Func::Construction(vec![
                Func::Constant(Object::atom("42")),
                Func::Constant(Object::atom("99")),
            ])),
        );
        assert_eq!(roundtrip(&f, 0), 42);
    }

    #[test]
    fn lower_selector_second_of_pair() {
        // s₂:<x, y> = y. Exercises the non-zero offset path.
        let f = Func::Compose(
            Box::new(Func::Selector(2)),
            Box::new(Func::Construction(vec![
                Func::Constant(Object::atom("42")),
                Func::Constant(Object::atom("99")),
            ])),
        );
        assert_eq!(roundtrip(&f, 0), 99);
    }

    #[test]
    fn lower_selector_threads_input_via_id() {
        // s₁:<x, x> — both components echo the apply input, selector
        // picks the first. Result = x itself.
        let f = Func::Compose(
            Box::new(Func::Selector(1)),
            Box::new(Func::Construction(vec![Func::Id, Func::Id])),
        );
        assert_eq!(roundtrip(&f, 7), 7);
        assert_eq!(roundtrip(&f, -13), -13);
    }

    #[test]
    fn lower_selector_out_of_triple() {
        // s₃:<a, b, c> = c. Confirms the offset math scales past
        // the fixed-pair case.
        let f = Func::Compose(
            Box::new(Func::Selector(3)),
            Box::new(Func::Construction(vec![
                Func::Constant(Object::atom("100")),
                Func::Constant(Object::atom("200")),
                Func::Constant(Object::atom("300")),
            ])),
        );
        assert_eq!(roundtrip(&f, 0), 300);
    }

    // ── Arithmetic (binary on pair) ───────────────────────────────

    /// Build a pair Seq of two constant Atoms. Used by arithmetic tests
    /// to feed Add/Sub/Mul/Div without relying on the apply's i64 input.
    fn pair(a: i64, b: i64) -> Func {
        Func::Construction(vec![
            Func::Constant(Object::atom(&a.to_string())),
            Func::Constant(Object::atom(&b.to_string())),
        ])
    }

    #[test]
    fn lower_add_pair_returns_sum_atom() {
        // +:<3, 4> = 7
        let f = Func::Compose(Box::new(Func::Add), Box::new(pair(3, 4)));
        assert_eq!(roundtrip(&f, 0), 7);
        let f2 = Func::Compose(Box::new(Func::Add), Box::new(pair(-10, 3)));
        assert_eq!(roundtrip(&f2, 0), -7);
    }

    #[test]
    fn lower_sub_pair_returns_difference_atom() {
        // -:<10, 3> = 7. Order matters — confirms we read pair[0] as
        // the LHS and pair[1] as the RHS, not the other way around.
        let f = Func::Compose(Box::new(Func::Sub), Box::new(pair(10, 3)));
        assert_eq!(roundtrip(&f, 0), 7);
        let f2 = Func::Compose(Box::new(Func::Sub), Box::new(pair(3, 10)));
        assert_eq!(roundtrip(&f2, 0), -7);
    }

    #[test]
    fn lower_mul_pair_returns_product_atom() {
        // ×:<6, 7> = 42
        let f = Func::Compose(Box::new(Func::Mul), Box::new(pair(6, 7)));
        assert_eq!(roundtrip(&f, 0), 42);
        let f2 = Func::Compose(Box::new(Func::Mul), Box::new(pair(-3, -4)));
        assert_eq!(roundtrip(&f2, 0), 12);
    }

    #[test]
    fn lower_div_pair_returns_quotient_atom() {
        // ÷:<100, 4> = 25 ; signed.
        let f = Func::Compose(Box::new(Func::Div), Box::new(pair(100, 4)));
        assert_eq!(roundtrip(&f, 0), 25);
        let f2 = Func::Compose(Box::new(Func::Div), Box::new(pair(-20, 5)));
        assert_eq!(roundtrip(&f2, 0), -4);
    }

    #[test]
    fn lower_div_by_zero_returns_phi_not_trap() {
        // ÷:<10, 0> must return phi (ptr 0), not trap. Backus's ÷
        // plus AREST's Object::Bottom propagation require this — a
        // naive I64DivS would abort the instance.
        let f = Func::Compose(Box::new(Func::Div), Box::new(pair(10, 0)));
        let (ptr, _) = invoke(&f, 0);
        assert_eq!(ptr, 0, "divide by zero must return phi");
    }

    #[test]
    fn lower_arithmetic_composes_with_itself() {
        // +:<+:<1, 2>, 3> = 6. Exercises arithmetic inside a pair
        // slot — one arithmetic op produces an Atom that becomes
        // element 0 of another pair, which another arithmetic op
        // consumes. Pure computation through the alloc arena.
        let inner = Func::Compose(Box::new(Func::Add), Box::new(pair(1, 2)));
        let outer_pair = Func::Construction(vec![
            inner,
            Func::Constant(Object::atom("3")),
        ]);
        let f = Func::Compose(Box::new(Func::Add), Box::new(outer_pair));
        assert_eq!(roundtrip(&f, 0), 6);
    }

    // ── Comparisons (binary on pair) ──────────────────────────────

    #[test]
    fn lower_eq_returns_one_for_equal_pair_zero_otherwise() {
        let eq_pair = |a: i64, b: i64| Func::Compose(Box::new(Func::Eq), Box::new(pair(a, b)));
        assert_eq!(roundtrip(&eq_pair(5, 5), 0), 1);
        assert_eq!(roundtrip(&eq_pair(5, 6), 0), 0);
        assert_eq!(roundtrip(&eq_pair(-3, -3), 0), 1);
    }

    #[test]
    fn lower_gt_is_signed_strictly_greater_than() {
        let gt = |a: i64, b: i64| Func::Compose(Box::new(Func::Gt), Box::new(pair(a, b)));
        assert_eq!(roundtrip(&gt(10, 3), 0), 1);
        assert_eq!(roundtrip(&gt(3, 10), 0), 0);
        assert_eq!(roundtrip(&gt(5, 5), 0), 0);       // strict
        assert_eq!(roundtrip(&gt(-1, -2), 0), 1);     // signed compare
    }

    #[test]
    fn lower_lt_is_signed_strictly_less_than() {
        let lt = |a: i64, b: i64| Func::Compose(Box::new(Func::Lt), Box::new(pair(a, b)));
        assert_eq!(roundtrip(&lt(3, 10), 0), 1);
        assert_eq!(roundtrip(&lt(10, 3), 0), 0);
        assert_eq!(roundtrip(&lt(5, 5), 0), 0);
        assert_eq!(roundtrip(&lt(-2, -1), 0), 1);
    }

    #[test]
    fn lower_ge_is_signed_greater_or_equal() {
        let ge = |a: i64, b: i64| Func::Compose(Box::new(Func::Ge), Box::new(pair(a, b)));
        assert_eq!(roundtrip(&ge(10, 3), 0), 1);
        assert_eq!(roundtrip(&ge(5, 5), 0), 1);       // equal is true
        assert_eq!(roundtrip(&ge(3, 10), 0), 0);
    }

    #[test]
    fn lower_le_is_signed_less_or_equal() {
        let le = |a: i64, b: i64| Func::Compose(Box::new(Func::Le), Box::new(pair(a, b)));
        assert_eq!(roundtrip(&le(3, 10), 0), 1);
        assert_eq!(roundtrip(&le(5, 5), 0), 1);
        assert_eq!(roundtrip(&le(10, 3), 0), 0);
    }

    #[test]
    fn lower_comparison_feeds_condition_naturally() {
        // Condition(Gt, Constant 100, Constant 200) ∘ <input, threshold>
        // Returns 100 if input > threshold else 200. Proves the {0,1}
        // Atom produced by Gt flows through Condition's truthy check.
        let f = Func::Compose(
            Box::new(Func::Condition(
                Box::new(Func::Gt),
                Box::new(Func::Constant(Object::atom("100"))),
                Box::new(Func::Constant(Object::atom("200"))),
            )),
            Box::new(pair(7, 5)),
        );
        assert_eq!(roundtrip(&f, 0), 100);
        let f2 = Func::Compose(
            Box::new(Func::Condition(
                Box::new(Func::Gt),
                Box::new(Func::Constant(Object::atom("100"))),
                Box::new(Func::Constant(Object::atom("200"))),
            )),
            Box::new(pair(3, 5)),
        );
        assert_eq!(roundtrip(&f2, 0), 200);
    }

    // ── While (bounded iteration) ─────────────────────────────────

    #[test]
    fn lower_while_with_constant_false_predicate_is_identity() {
        // (while (Constant 0; f)) : x = x — pred never fires, body
        // never runs.
        let f = Func::While(
            Box::new(Func::Constant(Object::atom("0"))),
            Box::new(Func::Id),
        );
        assert_eq!(roundtrip(&f, 42), 42);
        assert_eq!(roundtrip(&f, -7), -7);
    }

    #[test]
    fn lower_while_iterates_until_predicate_falsifies() {
        // (while (x → f; x)) where f = pair<x, -1> → Add = x - 1.
        // Starts truthy (Atom(n) with n ≠ 0), decrements each
        // iteration, stops when acc reaches 0. For n=5 → 0 in 5
        // iterations.
        //
        // The body: pair<id, Constant(-1)> → Add = x + (-1) = x - 1.
        // The pred: Id — truthy iff acc is non-zero Atom.
        let decrement = Func::Compose(
            Box::new(Func::Add),
            Box::new(Func::Construction(vec![
                Func::Id,
                Func::Constant(Object::atom("-1")),
            ])),
        );
        let f = Func::While(Box::new(Func::Id), Box::new(decrement));
        assert_eq!(roundtrip(&f, 5), 0);
        assert_eq!(roundtrip(&f, 0), 0);      // already zero → skip
        assert_eq!(roundtrip(&f, 1), 0);
    }

    #[test]
    fn lower_while_cap_prevents_runaway() {
        // Pathological case: predicate never falsifies AND body is
        // identity. Without the safety counter, this would infinite-
        // loop. With the counter, we bail out at 1M iterations and
        // return whatever acc holds. Using Id for both pred and body
        // means no per-iteration heap allocation — the single 64 KB
        // memory page is enough even at full cap.
        //
        // Pred = Id : truthy iff acc is a non-null non-zero Atom.
        // Body = Id : acc never changes.
        //   => 1M iterations, then exit, acc = Atom(42).
        let f = Func::While(Box::new(Func::Id), Box::new(Func::Id));
        assert_eq!(roundtrip(&f, 42), 42);
    }

    // ── Distribution (DistL, DistR, Trans) ────────────────────────

    #[test]
    fn lower_distl_pairs_scalar_with_each_element() {
        // distl:<9, <1, 2, 3>> = <<9,1>, <9,2>, <9,3>>.
        // Decode manually since roundtrip_seq is flat; we need
        // Seq-of-Seqs.
        let f = Func::Compose(
            Box::new(Func::DistL),
            Box::new(Func::Construction(vec![
                Func::Constant(Object::atom("9")),
                seq_of_atoms(&[1, 2, 3]),
            ])),
        );
        let (ptr, data) = invoke(&f, 0);
        assert_eq!(read_u32(&data, ptr), TAG_SEQ as u32);
        assert_eq!(read_u32(&data, ptr + 4), 3);
        for (i, expected_tail) in [1i64, 2, 3].iter().enumerate() {
            let pair_ptr = read_u32(&data, ptr + 8 + 4 * i as u32);
            assert_eq!(read_u32(&data, pair_ptr), TAG_SEQ as u32);
            assert_eq!(read_u32(&data, pair_ptr + 4), 2);
            let head_ptr = read_u32(&data, pair_ptr + 8);
            let tail_ptr = read_u32(&data, pair_ptr + 12);
            assert_eq!(read_i64(&data, head_ptr + 8), 9, "pair {i} head = 9");
            assert_eq!(read_i64(&data, tail_ptr + 8), *expected_tail,
                "pair {i} tail = {expected_tail}");
        }
    }

    #[test]
    fn lower_distl_over_empty_inner_is_empty() {
        let f = Func::Compose(
            Box::new(Func::DistL),
            Box::new(Func::Construction(vec![
                Func::Constant(Object::atom("9")),
                seq_of_atoms(&[]),
            ])),
        );
        let (ptr, data) = invoke(&f, 0);
        assert_eq!(read_u32(&data, ptr), TAG_SEQ as u32);
        assert_eq!(read_u32(&data, ptr + 4), 0);
    }

    #[test]
    fn lower_distr_pairs_each_element_with_scalar() {
        // distr:<<1, 2, 3>, 9> = <<1,9>, <2,9>, <3,9>>.
        let f = Func::Compose(
            Box::new(Func::DistR),
            Box::new(Func::Construction(vec![
                seq_of_atoms(&[1, 2, 3]),
                Func::Constant(Object::atom("9")),
            ])),
        );
        let (ptr, data) = invoke(&f, 0);
        assert_eq!(read_u32(&data, ptr), TAG_SEQ as u32);
        assert_eq!(read_u32(&data, ptr + 4), 3);
        for (i, expected_head) in [1i64, 2, 3].iter().enumerate() {
            let pair_ptr = read_u32(&data, ptr + 8 + 4 * i as u32);
            let head_ptr = read_u32(&data, pair_ptr + 8);
            let tail_ptr = read_u32(&data, pair_ptr + 12);
            assert_eq!(read_i64(&data, head_ptr + 8), *expected_head);
            assert_eq!(read_i64(&data, tail_ptr + 8), 9);
        }
    }

    #[test]
    fn lower_trans_transposes_uniform_seq_of_seqs() {
        // trans:<<1,2,3>, <4,5,6>> = <<1,4>, <2,5>, <3,6>>.
        let f = Func::Compose(
            Box::new(Func::Trans),
            Box::new(Func::Construction(vec![
                seq_of_atoms(&[1, 2, 3]),
                seq_of_atoms(&[4, 5, 6]),
            ])),
        );
        let (ptr, data) = invoke(&f, 0);
        assert_eq!(read_u32(&data, ptr), TAG_SEQ as u32);
        assert_eq!(read_u32(&data, ptr + 4), 3, "output length = inner length 3");
        let expected = [[1i64, 4], [2, 5], [3, 6]];
        for (i, row) in expected.iter().enumerate() {
            let pair_ptr = read_u32(&data, ptr + 8 + 4 * i as u32);
            assert_eq!(read_u32(&data, pair_ptr + 4), 2, "each row length = outer length 2");
            for (j, &expected_val) in row.iter().enumerate() {
                let elem_ptr = read_u32(&data, pair_ptr + 8 + 4 * j as u32);
                assert_eq!(read_i64(&data, elem_ptr + 8), expected_val,
                    "row {i} col {j} = {expected_val}");
            }
        }
    }

    #[test]
    fn lower_trans_is_its_own_inverse() {
        // trans ∘ trans on a 2×3 matrix returns a 2×3 matrix.
        // Concretely: <<1,2,3>, <4,5,6>> → <<1,4>,<2,5>,<3,6>>
        //          → <<1,2,3>, <4,5,6>>.
        let matrix = Func::Construction(vec![
            seq_of_atoms(&[1, 2, 3]),
            seq_of_atoms(&[4, 5, 6]),
        ]);
        let f = Func::Compose(
            Box::new(Func::Trans),
            Box::new(Func::Compose(Box::new(Func::Trans), Box::new(matrix))),
        );
        let (ptr, data) = invoke(&f, 0);
        assert_eq!(read_u32(&data, ptr + 4), 2);
        let row0 = read_u32(&data, ptr + 8);
        let row1 = read_u32(&data, ptr + 12);
        // Row 0 should be <1,2,3>
        for (j, &v) in [1i64, 2, 3].iter().enumerate() {
            let elem = read_u32(&data, row0 + 8 + 4 * j as u32);
            assert_eq!(read_i64(&data, elem + 8), v);
        }
        // Row 1 should be <4,5,6>
        for (j, &v) in [4i64, 5, 6].iter().enumerate() {
            let elem = read_u32(&data, row1 + 8 + 4 * j as u32);
            assert_eq!(read_i64(&data, elem + 8), v);
        }
    }

    #[test]
    fn lower_trans_on_empty_outer_returns_empty() {
        let f = Func::Compose(Box::new(Func::Trans), Box::new(seq_of_atoms(&[])));
        let (ptr, data) = invoke(&f, 0);
        assert_eq!(read_u32(&data, ptr), TAG_SEQ as u32);
        assert_eq!(read_u32(&data, ptr + 4), 0);
    }

    // ── Binary Seq builders (ApndL, ApndR, Concat) ────────────────

    #[test]
    fn lower_apndl_prepends_head_to_inner_seq() {
        // apndl:<99, <1, 2, 3>> = <99, 1, 2, 3>.
        let f = Func::Compose(
            Box::new(Func::ApndL),
            Box::new(Func::Construction(vec![
                Func::Constant(Object::atom("99")),
                seq_of_atoms(&[1, 2, 3]),
            ])),
        );
        assert_eq!(roundtrip_seq(&f, 0), vec![99, 1, 2, 3]);
    }

    #[test]
    fn lower_apndl_onto_empty_seq_produces_singleton() {
        // apndl:<99, <>> = <99>.
        let f = Func::Compose(
            Box::new(Func::ApndL),
            Box::new(Func::Construction(vec![
                Func::Constant(Object::atom("99")),
                seq_of_atoms(&[]),
            ])),
        );
        assert_eq!(roundtrip_seq(&f, 0), vec![99]);
    }

    #[test]
    fn lower_apndr_appends_tail_to_inner_seq() {
        // apndr:<<1, 2, 3>, 99> = <1, 2, 3, 99>.
        let f = Func::Compose(
            Box::new(Func::ApndR),
            Box::new(Func::Construction(vec![
                seq_of_atoms(&[1, 2, 3]),
                Func::Constant(Object::atom("99")),
            ])),
        );
        assert_eq!(roundtrip_seq(&f, 0), vec![1, 2, 3, 99]);
    }

    #[test]
    fn lower_apndr_onto_empty_seq_produces_singleton() {
        // apndr:<<>, 99> = <99>.
        let f = Func::Compose(
            Box::new(Func::ApndR),
            Box::new(Func::Construction(vec![
                seq_of_atoms(&[]),
                Func::Constant(Object::atom("99")),
            ])),
        );
        assert_eq!(roundtrip_seq(&f, 0), vec![99]);
    }

    #[test]
    fn lower_concat_flattens_seq_of_seqs() {
        // concat:<<1,2>, <3,4,5>, <6>> = <1,2,3,4,5,6>.
        let f = Func::Compose(
            Box::new(Func::Concat),
            Box::new(Func::Construction(vec![
                seq_of_atoms(&[1, 2]),
                seq_of_atoms(&[3, 4, 5]),
                seq_of_atoms(&[6]),
            ])),
        );
        assert_eq!(roundtrip_seq(&f, 0), vec![1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn lower_concat_handles_empty_inner_seqs() {
        // concat:<<>, <1>, <>, <2, 3>, <>> = <1, 2, 3>.
        // Empty inner seqs contribute nothing to total length and the
        // inner-loop exit condition handles length-0 cleanly.
        let f = Func::Compose(
            Box::new(Func::Concat),
            Box::new(Func::Construction(vec![
                seq_of_atoms(&[]),
                seq_of_atoms(&[1]),
                seq_of_atoms(&[]),
                seq_of_atoms(&[2, 3]),
                seq_of_atoms(&[]),
            ])),
        );
        assert_eq!(roundtrip_seq(&f, 0), vec![1, 2, 3]);
    }

    #[test]
    fn lower_concat_of_empty_outer_is_empty() {
        // concat:<> = <>.
        let f = Func::Compose(Box::new(Func::Concat), Box::new(seq_of_atoms(&[])));
        assert_eq!(roundtrip_seq(&f, 0), Vec::<i64>::new());
    }

    // ── Unary Seq transformers (Tail, Reverse, RotL, RotR) ────────

    /// Small helper to build a Seq literal from an i64 list for test setup.
    fn seq_of_atoms(values: &[i64]) -> Func {
        Func::Construction(
            values.iter()
                .map(|v| Func::Constant(Object::atom(&v.to_string())))
                .collect()
        )
    }

    #[test]
    fn lower_tail_drops_first_element() {
        // tl:<10, 20, 30> = <20, 30>.
        let f = Func::Compose(
            Box::new(Func::Tail),
            Box::new(seq_of_atoms(&[10, 20, 30])),
        );
        assert_eq!(roundtrip_seq(&f, 0), vec![20, 30]);
    }

    #[test]
    fn lower_tail_of_single_element_returns_empty_seq() {
        // tl:<x> = <>.
        let f = Func::Compose(
            Box::new(Func::Tail),
            Box::new(seq_of_atoms(&[99])),
        );
        assert_eq!(roundtrip_seq(&f, 0), Vec::<i64>::new());
    }

    #[test]
    fn lower_tail_of_empty_is_empty() {
        // tl:<> = <> via saturated subtract on in_len.
        let f = Func::Compose(
            Box::new(Func::Tail),
            Box::new(seq_of_atoms(&[])),
        );
        assert_eq!(roundtrip_seq(&f, 0), Vec::<i64>::new());
    }

    #[test]
    fn lower_reverse_flips_element_order() {
        // reverse:<1, 2, 3, 4> = <4, 3, 2, 1>.
        let f = Func::Compose(
            Box::new(Func::Reverse),
            Box::new(seq_of_atoms(&[1, 2, 3, 4])),
        );
        assert_eq!(roundtrip_seq(&f, 0), vec![4, 3, 2, 1]);
    }

    #[test]
    fn lower_reverse_of_empty_is_empty() {
        let f = Func::Compose(Box::new(Func::Reverse), Box::new(seq_of_atoms(&[])));
        assert_eq!(roundtrip_seq(&f, 0), Vec::<i64>::new());
    }

    #[test]
    fn lower_reverse_of_single_is_self() {
        let f = Func::Compose(Box::new(Func::Reverse), Box::new(seq_of_atoms(&[7])));
        assert_eq!(roundtrip_seq(&f, 0), vec![7]);
    }

    #[test]
    fn lower_rotl_shifts_head_to_tail() {
        // rotl:<1, 2, 3> = <2, 3, 1>.
        let f = Func::Compose(Box::new(Func::RotL), Box::new(seq_of_atoms(&[1, 2, 3])));
        assert_eq!(roundtrip_seq(&f, 0), vec![2, 3, 1]);
    }

    #[test]
    fn lower_rotl_preserves_empty_and_singleton() {
        assert_eq!(roundtrip_seq(&Func::Compose(Box::new(Func::RotL), Box::new(seq_of_atoms(&[]))), 0),
            Vec::<i64>::new());
        assert_eq!(roundtrip_seq(&Func::Compose(Box::new(Func::RotL), Box::new(seq_of_atoms(&[7]))), 0),
            vec![7]);
    }

    #[test]
    fn lower_rotr_shifts_tail_to_head() {
        // rotr:<1, 2, 3> = <3, 1, 2>.
        let f = Func::Compose(Box::new(Func::RotR), Box::new(seq_of_atoms(&[1, 2, 3])));
        assert_eq!(roundtrip_seq(&f, 0), vec![3, 1, 2]);
    }

    #[test]
    fn lower_rotr_is_inverse_of_rotl() {
        // rotr ∘ rotl:<1,2,3,4,5> = <1,2,3,4,5>.
        let f = Func::Compose(
            Box::new(Func::RotR),
            Box::new(Func::Compose(
                Box::new(Func::RotL),
                Box::new(seq_of_atoms(&[1, 2, 3, 4, 5])),
            )),
        );
        assert_eq!(roundtrip_seq(&f, 0), vec![1, 2, 3, 4, 5]);
    }

    // ── Structural predicates (AtomTest, NullTest, Length) ────────

    #[test]
    fn lower_atom_test_on_atom_returns_one() {
        // atom:x=1 when x is a non-null Atom. The boxed i64 input
        // becomes an Atom at apply entry, so Id preserves atomness.
        let f = Func::Compose(Box::new(Func::AtomTest), Box::new(Func::Id));
        assert_eq!(roundtrip(&f, 42), 1);
        assert_eq!(roundtrip(&f, 0), 1);      // Atom(0) is still an Atom
        assert_eq!(roundtrip(&f, -1), 1);
    }

    #[test]
    fn lower_atom_test_on_seq_returns_zero() {
        // atom:<a, b> = 0 — Construction produces a Seq.
        let f = Func::Compose(
            Box::new(Func::AtomTest),
            Box::new(Func::Construction(vec![Func::Id, Func::Id])),
        );
        assert_eq!(roundtrip(&f, 42), 0);
    }

    #[test]
    fn lower_atom_test_on_null_returns_zero() {
        // atom:φ = 0 — Insert on empty Seq returns null ptr, which
        // must classify as non-Atom despite potential memory-zero
        // masquerade at ptr=0.
        let f = Func::Compose(
            Box::new(Func::AtomTest),
            Box::new(Func::Compose(
                Box::new(Func::Insert(Box::new(Func::Id))),
                Box::new(Func::Construction(vec![])),
            )),
        );
        assert_eq!(roundtrip(&f, 0), 0);
    }

    #[test]
    fn lower_null_test_on_atom_returns_zero() {
        // null:Atom = 0 — an Atom (even Atom(0)) is not φ.
        let f = Func::Compose(Box::new(Func::NullTest), Box::new(Func::Id));
        assert_eq!(roundtrip(&f, 42), 0);
        assert_eq!(roundtrip(&f, 0), 0);    // critical: Atom(0) ≠ φ
    }

    #[test]
    fn lower_null_test_on_empty_seq_returns_one() {
        // null:<> = 1 — an empty Seq matches φ.
        let f = Func::Compose(
            Box::new(Func::NullTest),
            Box::new(Func::Construction(vec![])),
        );
        assert_eq!(roundtrip(&f, 0), 1);
    }

    #[test]
    fn lower_null_test_on_null_ptr_returns_one() {
        // null:φ = 1 — null-ptr form of φ from /Id:<> (fold on empty).
        let f = Func::Compose(
            Box::new(Func::NullTest),
            Box::new(Func::Compose(
                Box::new(Func::Insert(Box::new(Func::Id))),
                Box::new(Func::Construction(vec![])),
            )),
        );
        assert_eq!(roundtrip(&f, 0), 1);
    }

    #[test]
    fn lower_null_test_on_nonempty_seq_returns_zero() {
        // null:<x> = 0 for any non-empty Seq.
        let f = Func::Compose(
            Box::new(Func::NullTest),
            Box::new(Func::Construction(vec![Func::Id])),
        );
        assert_eq!(roundtrip(&f, 42), 0);
    }

    #[test]
    fn lower_length_of_seq_returns_element_count_atom() {
        // length:<a, b, c> = 3.
        let f = Func::Compose(
            Box::new(Func::Length),
            Box::new(Func::Construction(vec![
                Func::Constant(Object::atom("10")),
                Func::Constant(Object::atom("20")),
                Func::Constant(Object::atom("30")),
            ])),
        );
        assert_eq!(roundtrip(&f, 0), 3);
    }

    #[test]
    fn lower_length_of_empty_seq_is_zero() {
        // length:<> = 0.
        let f = Func::Compose(
            Box::new(Func::Length),
            Box::new(Func::Construction(vec![])),
        );
        assert_eq!(roundtrip(&f, 0), 0);
    }

    #[test]
    fn lower_length_of_atom_returns_phi() {
        // length:Atom = φ per Backus bottom-on-type-error.
        let f = Func::Compose(Box::new(Func::Length), Box::new(Func::Id));
        let (ptr, _) = invoke(&f, 42);
        assert_eq!(ptr, 0, "length on Atom must return null ptr (φ)");
    }

    // ── Logic (And/Or pair, Not unary) ────────────────────────────

    #[test]
    fn lower_and_returns_logical_conjunction_atom() {
        // and:<y, z>: {0,0}→0, {0,1}→0, {1,0}→0, {1,1}→1. Exercises
        // both-zero, one-zero, both-non-zero paths. Nonzero counts
        // as truthy regardless of magnitude.
        let and = |a: i64, b: i64| Func::Compose(Box::new(Func::And), Box::new(pair(a, b)));
        assert_eq!(roundtrip(&and(0, 0), 0), 0);
        assert_eq!(roundtrip(&and(1, 0), 0), 0);
        assert_eq!(roundtrip(&and(0, 1), 0), 0);
        assert_eq!(roundtrip(&and(1, 1), 0), 1);
        assert_eq!(roundtrip(&and(42, -7), 0), 1);  // both nonzero → truthy
    }

    #[test]
    fn lower_or_returns_logical_disjunction_atom() {
        let or = |a: i64, b: i64| Func::Compose(Box::new(Func::Or), Box::new(pair(a, b)));
        assert_eq!(roundtrip(&or(0, 0), 0), 0);
        assert_eq!(roundtrip(&or(1, 0), 0), 1);
        assert_eq!(roundtrip(&or(0, 1), 0), 1);
        assert_eq!(roundtrip(&or(-5, 0), 0), 1);    // negative nonzero → truthy
    }

    #[test]
    fn lower_not_flips_truthiness_of_unary_atom() {
        // not:x = 1 if x is falsy, 0 if truthy. Unary input, no pair.
        let f = Func::Compose(Box::new(Func::Not), Box::new(Func::Id));
        assert_eq!(roundtrip(&f, 0), 1);    // Atom 0 is falsy
        assert_eq!(roundtrip(&f, 42), 0);   // Atom 42 is truthy
        assert_eq!(roundtrip(&f, -1), 0);   // nonzero is truthy
    }

    #[test]
    fn lower_double_negation_restores_truthiness() {
        // not ∘ not = truthy-indicator. Identity on {0, 1} inputs,
        // coerces any nonzero to 1.
        let f = Func::Compose(Box::new(Func::Not), Box::new(Func::Not));
        assert_eq!(roundtrip(&f, 0), 0);
        assert_eq!(roundtrip(&f, 1), 1);
        assert_eq!(roundtrip(&f, 42), 1);   // coerces
    }

    #[test]
    fn lower_logic_composes_with_comparisons() {
        // (Gt ∘ <id, 5>) and (Lt ∘ <id, 10>) then And:
        // Accept input iff 5 < input < 10.
        // The chain:
        //   pair = <Gt(x, 5), Lt(x, 10)>
        //   result = And(pair)
        let in_range = Func::Compose(
            Box::new(Func::And),
            Box::new(Func::Construction(vec![
                Func::Compose(
                    Box::new(Func::Gt),
                    Box::new(Func::Construction(vec![
                        Func::Id,
                        Func::Constant(Object::atom("5")),
                    ])),
                ),
                Func::Compose(
                    Box::new(Func::Lt),
                    Box::new(Func::Construction(vec![
                        Func::Id,
                        Func::Constant(Object::atom("10")),
                    ])),
                ),
            ])),
        );
        assert_eq!(roundtrip(&in_range, 7), 1);
        assert_eq!(roundtrip(&in_range, 5), 0);    // boundary (Gt is strict)
        assert_eq!(roundtrip(&in_range, 10), 0);
        assert_eq!(roundtrip(&in_range, 42), 0);
    }

    // ── Insert ────────────────────────────────────────────────────

    #[test]
    fn lower_insert_of_single_element_returns_that_element() {
        // /f:<x> = x regardless of f. Loop body never executes
        // because i starts at 1 and len == 1.
        let f = Func::Compose(
            Box::new(Func::Insert(Box::new(Func::Id))),
            Box::new(Func::Construction(vec![
                Func::Constant(Object::atom("42")),
            ])),
        );
        assert_eq!(roundtrip(&f, 0), 42);
    }

    #[test]
    fn lower_insert_of_empty_seq_returns_phi() {
        // /f:<> = phi (null ptr) in the PoC. Verify the return is
        // pointer 0 — a meaningful sentinel the caller can detect.
        let f = Func::Compose(
            Box::new(Func::Insert(Box::new(Func::Id))),
            Box::new(Func::Construction(vec![])),
        );
        let (ptr, _data) = invoke(&f, 0);
        assert_eq!(ptr, 0, "Insert over empty Seq must return phi (0)");
    }

    #[test]
    fn lower_insert_constant_ignores_accumulator_and_returns_const() {
        // Constant(99):<acc, elem> = atom 99 for every fold step, so
        // the final acc = 99 regardless of sequence contents.
        let f = Func::Compose(
            Box::new(Func::Insert(Box::new(Func::Constant(Object::atom("99"))))),
            Box::new(Func::Construction(vec![
                Func::Constant(Object::atom("5")),
                Func::Constant(Object::atom("10")),
                Func::Constant(Object::atom("20")),
            ])),
        );
        assert_eq!(roundtrip(&f, 0), 99);
    }

    #[test]
    fn lower_insert_id_over_pair_returns_the_pair_itself() {
        // /Id:<x, y> runs one fold step: pair = <x, y>, Id(pair) = pair.
        // Final acc = <x, y>.
        let f = Func::Compose(
            Box::new(Func::Insert(Box::new(Func::Id))),
            Box::new(Func::Construction(vec![
                Func::Constant(Object::atom("5")),
                Func::Constant(Object::atom("10")),
            ])),
        );
        assert_eq!(roundtrip_seq(&f, 0), vec![5, 10]);
    }

    #[test]
    fn lower_insert_id_over_three_nests_left_associatively() {
        // /Id:<5, 10, 20> = <<5, 10>, 20> with Id-as-fold-step.
        // Verifies the fold is left-associative (acc rebinds each
        // iteration) and that nested Seqs survive the pair-alloc
        // path without corruption.
        let f = Func::Compose(
            Box::new(Func::Insert(Box::new(Func::Id))),
            Box::new(Func::Construction(vec![
                Func::Constant(Object::atom("5")),
                Func::Constant(Object::atom("10")),
                Func::Constant(Object::atom("20")),
            ])),
        );
        let (ptr, data) = invoke(&f, 0);
        assert_eq!(read_u32(&data, ptr), TAG_SEQ as u32);
        assert_eq!(read_u32(&data, ptr + 4), 2, "outer length = 2");
        // Outer[0] is <5, 10>
        let inner = read_u32(&data, ptr + 8);
        assert_eq!(read_u32(&data, inner), TAG_SEQ as u32);
        assert_eq!(read_u32(&data, inner + 4), 2);
        let inner0 = read_u32(&data, inner + 8);
        assert_eq!(read_i64(&data, inner0 + 8), 5);
        let inner1 = read_u32(&data, inner + 12);
        assert_eq!(read_i64(&data, inner1 + 8), 10);
        // Outer[1] is atom 20
        let outer1 = read_u32(&data, ptr + 12);
        assert_eq!(read_i64(&data, outer1 + 8), 20);
    }

    #[test]
    fn lower_filter_patches_length_field_correctly() {
        // After Filter, tag must be SEQ and length must equal kept
        // count. Direct layout check so a bug in the length
        // patch-back doesn't hide behind roundtrip_seq.
        let f = Func::Compose(
            Box::new(Func::Filter(Box::new(Func::Id))),
            Box::new(Func::Construction(vec![
                Func::Constant(Object::atom("0")),
                Func::Constant(Object::atom("1")),
                Func::Constant(Object::atom("2")),
            ])),
        );
        let (ptr, data) = invoke(&f, 0);
        assert_eq!(read_u32(&data, ptr), TAG_SEQ as u32);
        assert_eq!(read_u32(&data, ptr + 4), 2, "exactly two truthy atoms");
    }

    #[test]
    fn lower_apply_to_all_nested_does_not_collide_scratches() {
        // α(α(Constant 7)) over <<id, id>, <id>> — inner ApplyToAll
        // claims scratch slots at next_scratch+4 (inside outer's
        // body), so the outer's i/len/in/out at slots 0..3 must not
        // overlap. Confirms the scratch math is correct.
        let inner_alpha = Func::ApplyToAll(Box::new(Func::Constant(Object::atom("7"))));
        let f = Func::Compose(
            Box::new(Func::ApplyToAll(Box::new(inner_alpha))),
            Box::new(Func::Construction(vec![
                Func::Construction(vec![Func::Id, Func::Id]),
                Func::Construction(vec![Func::Id]),
            ])),
        );
        // Outer result: <<7,7>, <7>>. Decode manually.
        let (ptr, data) = invoke(&f, 42);
        assert_eq!(read_u32(&data, ptr), TAG_SEQ as u32);
        assert_eq!(read_u32(&data, ptr + 4), 2);
        let first = read_u32(&data, ptr + 8);
        assert_eq!(read_u32(&data, first), TAG_SEQ as u32);
        assert_eq!(read_u32(&data, first + 4), 2);
        let first_first = read_u32(&data, first + 8);
        assert_eq!(read_i64(&data, first_first + 8), 7);
        let second = read_u32(&data, ptr + 12);
        assert_eq!(read_u32(&data, second + 4), 1);
    }

    // ── Semantic equivalence with Rust apply() ────────────────────

    #[test]
    fn wasm_result_matches_rust_apply_for_supported_variants() {
        // For the scalar-returning subset, the emitted WASM must
        // return an Atom holding the same i64 that ast::apply
        // produces. Construction results live in a Seq — tested
        // separately above since the Rust side uses Object::Seq of
        // Atom objects, not raw i64s.
        let cases: Vec<(Func, i64, i64)> = vec![
            (Func::Id, 99, 99),
            (Func::Id, -99, -99),
            (Func::Constant(Object::atom("42")), 7, 42),
            (Func::Constant(Object::atom("-5")), 123, -5),
            (
                Func::Compose(
                    Box::new(Func::Constant(Object::atom("77"))),
                    Box::new(Func::Id),
                ),
                11, 77,
            ),
        ];
        for (f, input, expected) in cases {
            let wasm_result = roundtrip(&f, input);
            assert_eq!(wasm_result, expected,
                "WASM result diverges for {:?} input={}", f, input);
        }
    }

    // ── Composed-primitives smoke (fast, always runs) ─────────────

    #[test]
    fn wasm_composed_primitives_smoke() {
        // Exercise a rich Func tree that mixes ~15 lowered primitives in
        // one pipeline. If any single variant regresses such that its
        // interaction with composition breaks, this one tree catches it
        // cheaper than the full per-variant suite.
        //
        //   f(x) = sum(map(doubled_if_positive, <1, -2, 3, -4, 5>))
        //
        //   doubled_if_positive(y) = Condition(Gt(y, 0), 2*y, 0)
        //   sum = /Add
        //
        // For the Seq <1, -2, 3, -4, 5>:
        //   doubled_if_positive mapped → <2, 0, 6, 0, 10>
        //   sum                         → 18
        let pred_positive = Func::Compose(
            Box::new(Func::Gt),
            Box::new(Func::Construction(vec![
                Func::Id,
                Func::Constant(Object::atom("0")),
            ])),
        );
        let doubled = Func::Compose(
            Box::new(Func::Mul),
            Box::new(Func::Construction(vec![
                Func::Id,
                Func::Constant(Object::atom("2")),
            ])),
        );
        let doubled_if_positive = Func::Condition(
            Box::new(pred_positive),
            Box::new(doubled),
            Box::new(Func::Constant(Object::atom("0"))),
        );
        let input_seq = seq_of_atoms(&[1, -2, 3, -4, 5]);
        let pipeline = Func::Compose(
            Box::new(Func::Insert(Box::new(Func::Add))),
            Box::new(Func::Compose(
                Box::new(Func::ApplyToAll(Box::new(doubled_if_positive))),
                Box::new(input_seq),
            )),
        );
        assert_eq!(roundtrip(&pipeline, 0), 18,
            "composed-primitives smoke: sum(map(doubled_if_positive, <1,-2,3,-4,5>)) = 18");
    }

    // ── Benchmark + fixture writer (ignored by default) ───────────

    /// `#[ignore]`'d so normal cargo test doesn't pay the runtime
    /// cost; run explicitly:
    ///
    ///   cargo test --features wasm-lower --release --lib \
    ///     bench_wasm_fixtures_and_write -- --ignored --nocapture
    ///
    /// Writes emitted WASM modules to `target/wasm_fixtures/` so the
    /// companion Bun script (`scripts/bench_bun.ts`) can load them
    /// and time the identical loop in V8. Prints ns/op for the Rust
    /// native apply() path AND the wasmi-interpreted WASM path so
    /// you can see the interpreter tax locally. Bun's V8 JIT result
    /// comes from `scripts/bench_bun.ts`.
    ///
    /// Note: apply returns an i32 ptr. The bench sums raw ptrs as a
    /// black-box; the ns/op comparison is unchanged in meaning. The
    /// `_and_write` in the name is a warning — the test has a file-
    /// write side effect, so it's intentionally ignored by default.
    #[test]
    #[ignore = "benchmark; run with --release --ignored --nocapture"]
    fn bench_wasm_fixtures_and_write() {
        use std::fs;
        use std::path::Path;
        use std::time::Instant;

        const ITERATIONS: u64 = 10_000_000;

        let fixtures_dir = Path::new("target/wasm_fixtures");
        fs::create_dir_all(fixtures_dir).expect("create fixtures dir");

        let cases: Vec<(&str, Func, i64)> = vec![
            ("id",              Func::Id, 42),
            ("constant_seven",  Func::Constant(Object::atom("7")), 42),
        ];

        eprintln!("\n=== WASM lowering benchmark — {} iterations ===", ITERATIONS);
        eprintln!("{:<18} {:>12} {:>12} {:>12}", "case", "rust-ns/op", "wasmi-ns/op", "ratio");

        for (name, func, input) in &cases {
            let bytes = lower_to_wasm(func).expect("lower must succeed");
            let path = fixtures_dir.join(format!("{}.wasm", name));
            fs::write(&path, &bytes).expect("write fixture");

            // Rust native path.
            let x = Object::atom(&input.to_string());
            let d = Object::phi();
            let t = Instant::now();
            let mut acc: i64 = 0;
            for _ in 0..ITERATIONS {
                let r = crate::ast::apply(func, &x, &d);
                if let Object::Atom(s) = &r {
                    acc = acc.wrapping_add(s.len() as i64);
                }
            }
            let rust_ns = t.elapsed().as_nanos() as f64 / ITERATIONS as f64;
            std::hint::black_box(acc);

            // wasmi interpreter path.
            let engine = Engine::default();
            let module = WiModule::new(&engine, &bytes[..]).expect("wasmi validate");
            let mut store: Store<()> = Store::new(&engine, ());
            let linker: Linker<()> = Linker::new(&engine);
            let instance = linker
                .instantiate_and_start(&mut store, &module)
                .expect("instantiate and start");
            let apply = instance.get_typed_func::<i64, i32>(&store, "apply").unwrap();
            let t = Instant::now();
            let mut wacc: i64 = 0;
            for _ in 0..ITERATIONS {
                wacc = wacc.wrapping_add(apply.call(&mut store, *input).unwrap() as i64);
            }
            let wasmi_ns = t.elapsed().as_nanos() as f64 / ITERATIONS as f64;
            std::hint::black_box(wacc);

            let ratio = wasmi_ns / rust_ns;
            eprintln!("{:<18} {:>12.1} {:>12.1} {:>11.1}x",
                name, rust_ns, wasmi_ns, ratio);
        }

        eprintln!("\nFixtures written to {}", fixtures_dir.display());
        eprintln!("Run `bun scripts/bench_bun.ts` for the V8-JIT (production) comparison.");
    }
}
