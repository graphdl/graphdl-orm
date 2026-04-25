#!/usr/bin/env bash
# crates/arest-kernel/src/post_link_armv7.sh
#
# Linker shim for the `arest-kernel-armv7-uefi.json` custom target.
# Sourced by `bash` via the JSON's `link-env: ["BASH_ENV=..."]` entry,
# so it runs *before* bash tries to interpret its positional arguments
# as a script. The shim:
#
#   1. Locates `rust-lld` in the active rustup toolchain.
#   2. Re-invokes it with `-flavor link` and the original args. On
#      Linux (Docker), rustc passes those args directly via `$@`
#      (with `$0` being the first positional rustc handed to bash —
#      typically `/NOLOGO` from the JSON's pre-link-args). On Windows,
#      rustc bundles the args into a `@response_file`; we forward
#      that as a single arg to `rust-lld`, which expands `@file`
#      natively (it is link.exe's response-file convention).
#   3. Patches the COFF Machine field at PE-header-offset + 4 from
#      0x01c4 (`IMAGE_FILE_MACHINE_ARMNT`, Microsoft Windows-on-ARM
#      convention) to 0x01c2 (`IMAGE_FILE_MACHINE_ARMTHUMB_MIXED`,
#      the EDK2-recognised constant for 32-bit ARM UEFI). EDK2's
#      PE-COFF loader rejects 0x01c4 with `EFI_UNSUPPORTED` —
#      `MdePkg/Include/IndustryStandard/PeImage.h` only lists 0x01c2
#      among supported 32-bit ARM machines. Without this patch,
#      ArmVirtPkg's `LoadImage()` fails on `/EFI/BOOT/BOOTARM.EFI`
#      and the kernel never starts (#441, follow-up to #438).
#   4. `exit`s before bash reaches its own argv processing — bash
#      would otherwise try to open `@<response-file>` (or `/NOLOGO`,
#      etc.) as a script and abort with `exit 127`.
#
# Why the `BASH_ENV` indirection rather than putting the script in
# `pre-link-args`:
#   * pre-link-args entries become positional arguments to bash, but
#     when the command line is long enough rustc bundles ALL args
#     (including pre-link-args) into a `@response_file` that gets
#     passed as `$1` to bash. Bash then tries to open `@<path>` as a
#     script and aborts with "No such file or directory" before any
#     pre-link-args wrapper ever runs.
#   * `BASH_ENV` is read on bash startup (before script processing)
#     for non-interactive shells. Sourcing this script via BASH_ENV
#     runs it *before* bash touches its argv, sidestepping the
#     response-file issue entirely. We then `exit` to skip bash's
#     normal script-execution phase.
#
# Why a one-byte patch and not a linker-flavor swap (option 3 from
# the bisect path documented in `crates/arest-kernel-image/
# Dockerfile.uefi-armv7`):
#   * `rust-lld -flavor link /machine:thumb` -> `error: unknown
#     /machine argument: thumb`. lld-link only accepts `arm` for
#     32-bit ARM, and `arm` writes 0x01c4. Verified via
#     `rust-lld -flavor link /? | grep machine` — the accepted
#     values are arm, arm64, arm64ec, arm64x, x64, x86 only. (The
#     bisect path's option 2 is dead.)
#   * Microsoft's `*-pc-windows-msvc` triples (`thumbv7a`, `armv7a`,
#     plain `arm`) are all Thumb-2-only; LLVM rejects ARM-mode
#     codegen with "target does not support ARM mode execution" and
#     in some cases crashes with STATUS_STACK_BUFFER_OVERRUN during
#     compiler_builtins. So dropping the `thumbv7a-` prefix doesn't
#     shift LLD's COFF Machine output either. (The bisect path's
#     option 1 is also dead.)
#   * Hence option 3 (post-link byte patch).
#
# Cross-platform notes:
#   * Docker `rust:latest` (Debian-based, Linux): bash is present
#     and rustc passes linker args directly on the command line
#     (POSIX argv limit is large enough that rustc never falls back
#     to a response file for our build). The shim runs end-to-end
#     and produces a correctly-Machine-stamped .efi without any
#     additional setup. The smoke test in `boot-kernel-uefi-armv7.ps1`
#     exercises this path.
#   * Windows dev host (Git Bash / MSYS bash via Git for Windows):
#     bash is on $PATH, but MSYS's argv-conversion layer mangles the
#     `/`-prefixed args coming from a non-MSYS parent process
#     (cargo.exe), so the shim sees an empty argv and aborts with a
#     clear error. Workaround on Windows: build with the upstream
#     rust-lld directly by temporarily reverting this JSON's
#     `link-env`/`linker`/`linker-flavor` overrides locally, then
#     patch the .efi by hand from PowerShell:
#         $efi = "target/arest-kernel-armv7-uefi/release/arest-kernel.efi"
#         $b   = [IO.File]::ReadAllBytes($efi)
#         $pe  = [BitConverter]::ToUInt32($b, 0x3c)
#         $b[$pe + 4] = 0xc2
#         [IO.File]::WriteAllBytes($efi, $b)
#     The Windows-host Machine-field check the brief's verification
#     step 2 asks for follows directly from this snippet.

set -euo pipefail

# ── 1. Locate rust-lld in the active toolchain ──────────────────────
SYSROOT="$(rustc --print sysroot)"
HOST="$(rustc -vV | sed -n 's/^host: //p')"
RUST_LLD="${SYSROOT}/lib/rustlib/${HOST}/bin/rust-lld"
if [ ! -x "${RUST_LLD}" ] && [ -x "${RUST_LLD}.exe" ]; then
    RUST_LLD="${RUST_LLD}.exe"
fi

# ── 2. Reconstruct the linker args and re-invoke rust-lld ───────────
# When BASH_ENV sources us:
#   * Linux/short-cmdline: $0 is the script-slot (the first
#     positional rustc passed to bash, e.g. `/NOLOGO`), $@ is the
#     rest of the linker args.
#   * Windows/response-file: $0 is `@<path>` and $@ is empty.
LD_ARGS=()
case "${0:-}" in
    @*|/*|*[/\\]*|-*)
        LD_ARGS+=("$0")
        ;;
esac
LD_ARGS+=("$@")
if [ "${#LD_ARGS[@]}" -eq 0 ]; then
    echo "post_link_armv7.sh: no linker arguments received (likely a Windows-host argv-mangling issue — see header comment for the manual-patch workaround)" >&2
    exit 1
fi
"${RUST_LLD}" -flavor link "${LD_ARGS[@]}"

# ── 3. Find the output file ─────────────────────────────────────────
OUTFILE=""
scan_for_out() {
    for tok in "$@"; do
        case "$tok" in
            /OUT:*|/out:*)
                tok="${tok#\"}"
                tok="${tok%\"}"
                OUTFILE="${tok#*:}"
                ;;
        esac
    done
}
scan_for_out "${LD_ARGS[@]}"
if [ -z "${OUTFILE}" ]; then
    for tok in "${LD_ARGS[@]}"; do
        case "$tok" in
            @*)
                RSP="${tok#@}"
                if [ -f "${RSP}" ]; then
                    while IFS= read -r line_tok; do
                        case "$line_tok" in
                            /OUT:*|/out:*|\"/OUT:*|\"/out:*)
                                line_tok="${line_tok#\"}"
                                line_tok="${line_tok%\"}"
                                OUTFILE="${line_tok#*:}"
                                ;;
                        esac
                    done < <(tr -s ' \r\n\t' '\n' < "${RSP}")
                fi
                ;;
        esac
    done
fi
if [ -z "${OUTFILE}" ]; then
    echo "post_link_armv7.sh: could not find /OUT: in linker args" >&2
    exit 1
fi
OUTFILE="${OUTFILE#\"}"
OUTFILE="${OUTFILE%\"}"
OUTFILE="${OUTFILE//\\/\/}"
if [ ! -f "${OUTFILE}" ]; then
    echo "post_link_armv7.sh: linker output not found at '${OUTFILE}'" >&2
    exit 1
fi

# ── 4. Patch the COFF Machine field ─────────────────────────────────
# PE header layout:
#   * Offset 0x3c (4 bytes, little-endian): pointer to PE header.
#   * At PE+0..3:  "PE\0\0" signature.
#   * At PE+4..5:  IMAGE_FILE_HEADER.Machine (little-endian u16).
PE_OFFSET="$(od -An -tu4 -j 60 -N 4 "${OUTFILE}" | tr -d ' \n')"
MACHINE_OFFSET=$((PE_OFFSET + 4))

# Read the existing two Machine bytes; idempotent re-link is safe.
read -r LO HI <<< "$(od -An -tu1 -j "${MACHINE_OFFSET}" -N 2 "${OUTFILE}")"

if [ "${LO}" = "194" ] && [ "${HI}" = "1" ]; then
    # Already 0x01c2 — nothing to do.
    exit 0
fi
if [ "${LO}" != "196" ] || [ "${HI}" != "1" ]; then
    printf 'post_link_armv7.sh: unexpected Machine field 0x%02x%02x at PE+4 (PE@0x%x); aborting\n' \
        "${HI}" "${LO}" "${PE_OFFSET}" >&2
    exit 1
fi

# Overwrite the low byte: 0xc4 -> 0xc2.
printf '\xc2' | dd of="${OUTFILE}" bs=1 count=1 seek="${MACHINE_OFFSET}" \
    conv=notrunc status=none

# ── 5. Exit so bash never processes its own argv ────────────────────
exit 0
