# 11 · SYSTEM as an Operating System Kernel

This is a re-frame, not a rewrite. AREST's algebra was designed without OS concepts in mind. It turns out that every primitive maps directly onto a familiar kernel concept, and the mapping clarifies what scheduling, concurrency, multi-user, and live-update should look like — concerns that have been awkward to fit into the algebra-first framing.

The lens: **SYSTEM IS a kernel.** Every entity is a process. Every cell is a named memory region. Every Platform function is a syscall. Every derivation rule is an event-driven daemon. The reframe doesn't change anything that runs today; it gives names to structures we already have, and surfaces structures we are missing.

## The kernel equation, OS-typed

```
SYSTEM:x = (ρ(↑entity(x):D)):↑op(x)
```

| Algebra term | OS analog |
|---|---|
| `↑entity(x):D` | process-table lookup (entity name → control block) |
| `ρ(...)` | dynamic loader / linker — name to executable code |
| `:↑op(x)` | syscall dispatch — code applied to operation args |
| `D` | the kernel address space — every cell, every def |

That's the whole thesis in one row. The rest of this doc maps the supporting machinery.

## Object hierarchy ↔ kernel data structures

| AREST | OS analog | Notes |
|---|---|---|
| `Object::Map` | page table | O(1) keyed access, exactly the cell lookup we just adopted in #150 |
| `Object::Seq` | linear page (or syscall-arg vector) | O(n) scan; appropriate for ordered traversal |
| `Object::Atom` | scalar / immediate value | string-tagged for serialization |
| `Object::Bottom` | uninitialized memory / EFAULT | propagates through composition like a fault |
| `⟨CELL, name, contents⟩` | inode + name + data | the named-region triple |

## Func variants ↔ kernel primitives

| Func | OS analog |
|---|---|
| `Func::Platform(name)` | **syscall** — hardwired entry into the kernel |
| `Func::Def(name)` | **process executable** — looked up dynamically by name |
| `Func::Native(closure)` | kernel-builtin callback (C function pointer) |
| `Func::Compose(f, g)` | function call (push frame) |
| `Func::Construction([...])` | parallel evaluation → tuple build (like SIMD lanes) |
| `Func::Condition(p, f, g)` | branch / `if` |
| `Func::ApplyToAll(f)` | `map` — embarrassingly parallel |
| `Func::Filter(p)` | filtered iteration |
| `Func::Insert(f)` | reduce / fold |
| `Func::While(p, f)` | loop with bounded iteration cap (no infinite loops by design) |
| `Func::Fetch` / `Func::FetchOrPhi` / `Func::Store` | mmap-style read/write syscalls |

## Operations ↔ syscalls

The current `Func::Platform` table is the syscall table. Concretely:

| Platform name | Syscall family |
|---|---|
| `compile` | `exec()` / `dlopen()` — load new code into DEFS |
| `apply_command` | `ioctl()` — generic command dispatcher |
| `create:{Noun}` | `creat()` — instantiate a new process of type Noun |
| `update:{Noun}` | `write()` — mutate process state |
| `transition:{Noun}` | `kill()` with a signal — fire an event on the SM |
| `list_noun:{Noun}` | `readdir()` — enumerate processes of a type |
| `verify_signature` | crypto syscall — security check |
| `audit` | `auditd` — append to the audit log |
| `project` / `join` / `restrict` / `tie` | relational syscalls — Codd θ₁ as kernel ops |

Adding a new `Func::Platform` name = adding a new syscall = recompiling the kernel. User code can't extend the syscall table at runtime; it can compose existing ones.

## Cell isolation ↔ process isolation

Definition 2 of AREST.tex says: *"For each cell ⟨CELL, n, c⟩ in D, at most one μ application that writes to n may be in progress at any time. Concurrent μ applications over disjoint cells are permitted."*

This is **per-cell mutex semantics**. Today the runtime takes one global lock on the entire `DOMAINS` Vec — the SERIAL fix in `48a7cd0` removed test-side over-locking but the engine still serializes through that single Vec mutex. The paper licenses **fine-grained per-cell locks**. With those, two `create:Order` and `create:Customer` commands run in parallel on different cores.

This is also what enables genuine multi-tenant isolation: each handle (tenant) is a separate `D`; per-handle isolation is automatic, per-cell isolation within a handle is the next layer.

## Process scheduling

Today: every `system_impl` call runs to completion under a single mutex. There's no scheduler — first-come, first-served, blocking.

Under the kernel lens, the obvious shape is:

- A queue of pending commands (processes-to-run).
- A scheduler that picks the next command and dispatches it on a worker.
- Per-cell locks ensure correctness when commands touch the same cell.
- Priority levels: alethic-constraint commands at higher priority than deontic-only or read-only commands.

For browser/Worker/FPGA targets the scheduler degenerates to "the next request", but the abstraction stays.

## Multi-user ↔ POSIX permissions

Every `User` entity in the population is a process owner. The metamodel's `User accesses Domain` derivation IS the POSIX permission check — `chmod` rules expressed as ORM 2 readings rather than a separate `/etc/passwd`.

When the user's identity reaches `resolve`, the runtime pushes a `created by User` fact. Subsequent `validate` runs see whoever the requesting user is and any deontic constraint that says *"It is forbidden that a User lacking access reads a Domain"* fires automatically. The "permission system" is just the constraint set.

Per-process credential propagation = `sender` argument flowing through `apply_command_defs`.

## Multi-tasking and concurrency

Three orthogonal axes, all naturally expressed:

1. **Across handles (tenants)**: full isolation, can run in parallel today even under the global Vec mutex if the lock is moved per-slot.
2. **Across cells within a handle**: per-cell locks (per Definition 2) let `create:Order` and `create:Customer` run concurrently.
3. **Within a single command pipeline**: resolve → derive → validate → emit can pipeline with multiple commands in flight (classic 4-stage CPU pipeline). Plus: derive's forward-chain over independent rules is parallelizable per rule.

The interpreter already has a `parallel` Cargo feature that pulls in rayon. The full payoff requires the per-cell locks.

## Live update — `compile` is `dlopen`

`compile` adds new readings → new defs in DEFS. In OS terms this is loading a new process executable into the kernel's process table without restart. The system stays live; new requests can call into the new code.

`#146 incremental compile` is exactly the `dlopen` optimization: don't relink the entire image to add one new symbol. Compile just the delta and merge.

## Boot / init

`metamodel_state()` is the kernel image. It loads once at process start (via `OnceLock`) and never changes. The metamodel **is** the kernel — its readings define the syscall ABI, the constraint kernel, the SM machinery. User Apps are processes loaded on top.

## Per-App namespacing

Apps are containers (Docker / cgroups). The recently-landed `openapi:{app-slug}` cell is per-App namespacing. The pattern generalizes:

- `app:{slug}:openapi`
- `app:{slug}:env`
- `app:{slug}:audit_log`
- `app:{slug}:resource_quota`

This becomes the contract for adding container-style isolation.

## Memory management

The population grows monotonically by `assert`; `retract` removes specific facts. This is **append-only with explicit free**. Like a generational GC where the only collection pass is a manual `compact` operation.

What's missing under the kernel lens:

- **Compaction**: rewrite the population to physically reclaim retracted facts. Reduces working-set size.
- **Page eviction**: unload rarely-touched cells (maybe entire app tenants) from RAM to disk.
- **Snapshots / rollback**: `D₀ → compile(d) → D₁`; if validation rejects, revert to `D₀`. The current code already does this — it's just not framed as snapshot/rollback.

## Signals → state-machine transitions

A process receives a signal; if it has a handler, the handler runs and transitions state. AREST: a fact arrives → if it's a transition trigger → the SM advances. Same shape, different name. The "machine fold" (Eq. 11 in the paper) is the signal handler dispatch loop.

External events from a queue, webhook, or peer enter the same fold. Cross-process IPC is just facts entering D.

## What this reframe makes obvious

A series of decisions and missing pieces become natural under the kernel lens:

1. **Per-cell locks** (not the global Vec mutex). Definition 2 directly licenses this. Currently blocking SMP scaling.
2. **Scheduler with priorities** for queued commands. Makes batch ingestion safe alongside interactive requests.
3. **Per-App namespacing** as the unit of resource accounting (CPU, memory, audit log).
4. **Snapshot/rollback** as a first-class operation, not implicit in the validate-and-revert flow.
5. **Page eviction** for very large multi-tenant deployments.
6. **The OS surface as the wire surface**: an external API that exposes "syscall dispatch" directly to LLMs/agents. Already 80% true via the MCP verb set.
7. **FPGA**: synthesizing this kernel as hardware. Backus §15 is exactly this proposal; #154 lays out the path.

## Renames worth considering (not now, but soon)

These names carry today and we could keep them. Renaming would make the code read like an OS:

| Today | Could be |
|---|---|
| `DOMAINS: Mutex<Vec<Option<CompiledState>>>` | `process_table` |
| `CompiledState` | `Process` or `Tenant` |
| `create_impl()` | `fork()` |
| `release_impl(handle)` | `exit(pid)` |
| `system_impl(handle, key, input)` | `syscall(pid, op, args)` |
| `Func::Platform(name)` | `Func::Syscall(name)` |
| `apply_command_defs` | `dispatch_syscall` |
| `compile_to_defs_state` | `link_process_image` |

Worth the diff once the OS lens has proven itself in scheduling/concurrency work. Until then the algebra-first names stay.

## Tasks under this lens

Existing tasks that gain definition from the kernel lens:

- **#112–#116 BroadcastDO streaming** — IPC primitives. A subscriber is `select(2)` on cell-change events.
- **#119–#120 MCP streaming** — same: process-level event subscribe/unsubscribe via the kernel.
- **#146 incremental compile** — `dlopen` partial-link.
- **#149 envelope alignment** — kernel ABI conformance: every syscall returns the same shape.
- **#151 Arc-backed Object** — copy-on-write pages.
- **#152 bytecode VM** — kernel instruction set; replaces the recursive interpreter with a flat instruction stream.
- **#154 FPGA** — synthesize the kernel in silicon.

New tasks that emerge:

- **Per-cell locking**: replace `Mutex<Vec<Option<CompiledState>>>` with `Vec<Option<Arc<RwLock<CompiledState>>>>` so per-tenant work parallelizes.
- **Per-cell locking inside a tenant**: per-cell mutex inside CompiledState so two `create:`s on different nouns parallelize.
- **Command queue + scheduler**: a per-tenant work queue with priority lanes (alethic > deontic > read-only).
- **Snapshot/rollback as a verb**: explicit `snapshot:{handle}` and `rollback:{handle, snapshot_id}` ops backed by COW cells.
- **Resource quotas per App**: CPU time, memory bytes, audit-log entries per `app:{slug}`.

## Don't reinvent the VM — WASM IS the bytecode

Observation: the Rust crate compiles to WASM. WASM is bytecode that runs in a VM. Writing an in-process bytecode interpreter inside `apply()` is re-implementing something that's already there.

**Convergence**: lower `Func` trees directly to WASM functions at AREST compile time. Store compiled `WebAssembly.Module` references in `D` instead of `Func` data. `apply(Def, x, d)` becomes `instance.exports.f(x, d)` — a direct VM dispatch with zero AREST-level interpretation overhead. Algebraic normalize (Backus §12) still runs before lowering.

Pipeline:

```
Func (normalized)  →  WASM  →  { V8, wasmer, wasmtime, FPGA silicon }
```

This unifies three things we used to model separately:

- **JIT interpreter speedup** (formerly #152 in-process bytecode) — runtime WASM compilation in Workers is supported via `new WebAssembly.Module(bytes)`. V8 + wasmer + wasmtime all expose the same contract.
- **FPGA kernel** (formerly #154 hand-rolled hardware compiler) — WASM-to-silicon pipelines exist (Wasmachine, FACE-WASM). AREST emits WASM; the silicon backend is someone else's problem.
- **Multi-backend deployment** — latency-critical tenants run on silicon, batch tenants on a server VM, dev runs in V8. Same IR.

Under the kernel lens, WASM IS our "machine code". Platform funcs are syscalls. Def-keyed WASM modules are user processes. `compile` is `exec()`. `fetch/store` on cells are memory-mapped I/O.

Tasks #152 and #154 are rescoped around this convergence.

## What's next

Start with the lens, not the code. Frame the next perf, concurrency, and multi-tenant work in OS terms — the right abstractions become obvious, the wrong ones reveal themselves quickly. Concrete code lands under tasks #146, #149, #151, #152, #154 and the new concurrency tasks above.

The algebra is the spec. The kernel is the implementation. WASM is the machine code.
