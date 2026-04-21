# CLAUDE.md — project instructions for AREST

## Concurrency: multiple Claude agents may be working on this repo simultaneously

Background agents (UI bootstrap, FPGA constraint modules, FORML 2 stage-2
refactors, E3 provenance work, and ad-hoc sessions) are spawned and worked
in parallel. Each runs in its own shell against the same working tree and
the same git index.

This has two concrete failure modes that have already happened in session
history:

1. **Shared staging index contamination.** `git add <file>` appends to
   the index, it does not scope it. If another agent has already staged
   unrelated files, `git commit` will sweep them into your commit. Seen
   in `49d332c` (UI work captured a concurrent E3 diff).

2. **Mid-refactor breakage visible to other sessions.** If an agent is
   mid-way through a cross-file rename (e.g., a struct field is removed
   from `types.rs` but the callers in `parse_forml2.rs` haven't caught
   up yet), every other session sees a broken build.

## Staging discipline (required)

**Always commit by explicit path, bypassing the shared index:**

```
git commit -m "your message" -- <file1> <file2>
```

The `--` + paths form tells `git commit` to take *exactly* those paths,
regardless of what is staged. Use this over `git add <file> && git commit`
whenever you're sharing the repo with other agents.

When you need to stage *and* commit separately (e.g., a pre-commit hook
needs index state), verify what is actually staged right before commit:

```
git diff --cached --stat          # list only what this commit will touch
git diff --cached <file>          # verify the intended diff for <file>
```

If `--cached --stat` shows files you did not intend to include, run
`git restore --staged <unwanted-file>` before committing.

## If you find the build broken

`cargo test --lib` should pass on `main`. If it doesn't and the error is
in a file you didn't touch, another agent is mid-refactor. Options:

- Pivot to work that doesn't depend on the broken file.
- Run `git log --oneline -20 -- <broken-file>` to see who touched it
  last. Their commit message may say "WIP" or cite an issue you can look
  up.
- Do NOT "fix" it by reverting the other agent's work. They may have a
  follow-up queued.

If you must unblock yourself, comment-out the broken callers temporarily
and leave a TODO with your session's timestamp so the other agent can
find it.

## Working directory ownership map

| Directory                          | Typical owner |
|------------------------------------|---------------|
| `crates/arest/src/ast.rs`          | Engine / E3   |
| `crates/arest/src/lib.rs`          | Engine        |
| `crates/arest/src/compile.rs`      | Compile       |
| `crates/arest/src/parse_forml2*.rs` | Parser        |
| `crates/arest/src/generators/fpga.rs` | FPGA (#303)   |
| `crates/arest/src/generators/solidity.rs` | Solidity (#304) |
| `src/mcp/*.ts`                     | MCP server    |
| `apps/ui.do/*`                     | UI (#121-126) |
| `readings/*.md`                    | Schema        |
| `docs/*.md`                        | Docs          |
| `_reports/*.md`                    | Transient — never commit |

If your task touches files in someone else's column, check the open
issues list first to see if that file is actively being refactored.

## Do NOT touch

- Another session's `_reports/*.md` files (they are scratch; not committed).
- `Cargo.lock` unless you are explicitly upgrading a dependency.
- `target/`, `node_modules/`, `dist/`, `pkg/` — all generated.

## Things to remember beyond this file

Per user's global CLAUDE.md: no memory system; use AREST MCP in local
mode. Use test-driven development. Commit early and often. Use PowerShell
in place of Bash when on Windows. Ask one question per request.
