// arest-cli — thin bin shim. The CLI dispatcher lives in
// `arest::cli::entry::main_entry` so it compiles inside the lib's
// rlib once, not once per (lib, bin) compilation pair. See
// crates/arest/src/cli/entry.rs for the full pre-extract context.
//
// Pre-extract (#684/#650b), src/main.rs declared `mod ast; mod
// compile; …` for all 31 lib modules independently of lib.rs's
// `pub mod ast; …`, forcing cargo to recompile each source file
// twice. cargo-timing 2026-05-01 measured ~120s of duplicate
// cumulative compile across `arest-cli "bin"` and
// `arest-cli "bin" (test)`. Now this shim is the entire bin's source.

fn main() {
    // The MSVC main thread reserves only 1 MiB of stack, which overflows mid
    // forward-chain fixpoint on large apps (support.auto.dev: 670 rules over
    // 6461 fact types — STATUS_STACK_OVERFLOW after compile, before the chain
    // completes; the same workload finishes cleanly on a 1 GiB stack). The
    // engine's apply/metacompose evaluator is inherently recursive over Func
    // trees, so we give it adequate headroom by running the real entry on a
    // worker thread with a large stack (512 MiB default, AREST_STACK_MB env
    // override). Spawned-thread stacks are honored on every platform, unlike
    // the linker-fixed main-thread reserve.
    let stack = arest::cli::entry::desired_stack_bytes();
    let worker = std::thread::Builder::new()
        .name("arest-main".to_string())
        .stack_size(stack)
        .spawn(arest::cli::entry::main_entry)
        .expect("failed to spawn arest-main worker thread");
    // `main_entry` returns () on success and signals all error paths via
    // std::process::exit (which tears down the whole process from the worker,
    // preserving the intended code). A panic is the only thing join() surfaces
    // here — re-raise it as Rust's conventional 101 so the shell / MCP see a
    // non-zero status (the default hook already printed the message).
    if worker.join().is_err() {
        std::process::exit(101);
    }
}
