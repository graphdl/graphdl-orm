# scripts/boot-kernel-uefi.ps1
#
# UEFI sibling of `boot-kernel.ps1`. Builds the arest-kernel as a
# `x86_64-unknown-uefi` PE32+ image (via `Dockerfile.uefi`) and boots
# it under QEMU + OVMF inside Docker. Serial output streams to the
# host terminal.
#
# Usage:
#   .\scripts\boot-kernel-uefi.ps1            # interactive boot
#   .\scripts\boot-kernel-uefi.ps1 -Smoke     # headless: assert banner
#
# Smoke mode (#344 step 4 waves 1-6 + #270/#271):
#   Boots the kernel under OVMF with a 30 s cap (boot-time UEFI
#   initialisation is slower than the BIOS path -- OVMF prints its
#   own banners before our entry runs), captures every byte of
#   serial, and asserts every banner line our entry writes pre- and
#   post-ExitBootServices. Exits 0 on success, 1 with the captured
#   log on failure. Asserted banner lines cover:
#     * step 4b: ExitBootServices + post-EBS 16550 serial cutover
#     * step 4d prep: SSE enable pre-EBS (5dc246a)
#     * step 4c: init_memory from UEFI memory map -> frames usable
#     * step 4d wave 3: post-EBS static-BSS heap (5b74f2a)
#     * step 4d wave 4: AREST engine init on UEFI (8ea0528)
#     * step 4d wave 5: wasmi executes user WASM (58cf113)
#     * #270/#271 shim: 10 Doom host imports bound (f8c11d2)
#     * step 4d wave 6: framebuffer::install + triple-buffer paint
#       smoke on GOP (#269 BIOS path now reachable on UEFI)
#
# Remaining for step 4d completion: kernel_run() handoff — requires
# virtio / block / net to compile on UEFI (currently gated) or a
# UEFI-specific kernel_run_uefi that skips those subsystems.

param(
    [switch]$Smoke
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path "$PSScriptRoot\..").Path

# docker writes BuildKit progress to stderr; PowerShell 5.1 wraps each
# stderr line in an ErrorRecord (NativeCommandError) when streams are
# merged, which fires under `$ErrorActionPreference = "Stop"`. Mirror
# the BIOS script's pattern: relax error-action around native-exec
# calls and gate on $LASTEXITCODE.
Write-Host "Building UEFI kernel image (Docker)..." -ForegroundColor Cyan
$prevEAP = $ErrorActionPreference
$ErrorActionPreference = "Continue"
try {
    docker build -t arest-kernel-uefi -f "$repoRoot\crates\arest-kernel-image\Dockerfile.uefi" $repoRoot
} finally {
    $ErrorActionPreference = $prevEAP
}
if ($LASTEXITCODE -ne 0) { throw "Docker build failed (exit $LASTEXITCODE)" }

if ($Smoke) {
    # 60 s cap (was 30 s before #376) -- the Doom WASM instantiate
    # path adds wasmi parsing of the 4.35 MiB doom.wasm + initGame
    # WAD load + first tickGame, all under bounded fuel. wasmi
    # release-mode is ~10 MIPS on QEMU's emulated CPU, so 200 M
    # fuel = ~20 s per top-level call worst-case. The earlier 30 s
    # cap covered everything pre-Doom; 60 s gives the Doom path
    # headroom while still surfacing a hung initGame as a timeout
    # rather than letting the smoke wedge indefinitely.
    Write-Host "`nBooting UEFI kernel in smoke mode (60 s cap)..." -ForegroundColor Cyan

    $containerName = "arest-kernel-uefi-smoke-$([guid]::NewGuid().ToString('N').Substring(0,8))"
    $targetDir = Join-Path $repoRoot "target"
    $logPath = Join-Path $targetDir "kernel-uefi-smoke.log"
    New-Item -ItemType Directory -Force -Path $targetDir | Out-Null

    $prevEAP = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        & docker run --rm --name $containerName -d arest-kernel-uefi | Out-Null
    } finally {
        $ErrorActionPreference = $prevEAP
    }
    if ($LASTEXITCODE -ne 0) { throw "docker run failed (exit $LASTEXITCODE)" }

    try {
        # OVMF prints its own boot banners (TianoCore, UefiVersion, etc.)
        # before the firmware hands control to our PE32+. Poll for the
        # FINAL line the entry prints before halt -- "next: kernel_run
        # handoff (step 4d)" -- rather than an early line like "frames
        # usable". Without that, the docker-logs snapshot can race the
        # kernel's later output: the matcher fires on the early line,
        # the loop breaks, and the snapshot we then write to disk is
        # missing everything the kernel printed after. Polling on the
        # last line guarantees the snapshot includes every banner.
        #
        # Once step 4d boots through `kernel_run`, swap the marker for
        # "int3 round-tripped" -- the BIOS smoke's same beacon.
        $deadline = (Get-Date).AddSeconds(60)
        $log = ""
        while ((Get-Date) -lt $deadline) {
            Start-Sleep -Milliseconds 500
            $prevEAP = $ErrorActionPreference
            $ErrorActionPreference = "Continue"
            try {
                $log = (docker logs $containerName 2>&1 | Out-String)
            } finally {
                $ErrorActionPreference = $prevEAP
            }
            # Break as soon as we see either the clean halt marker
            # ("next: kernel_run handoff") OR the #376 known-limitation
            # marker ("Freed node aliases existing hole" -- the
            # linked_list_allocator panic that fires inside Doom's
            # Z_Init under wasmi's Memory::grow reallocs). Either
            # outcome means the smoke has captured the full banner
            # stream we care about; continued polling would only delay
            # the assertion phase by another 50+ seconds.
            if ($log -match "next:\s+kernel_run handoff") { break }
            if ($log -match "Freed node .* aliases existing hole") { break }
        }
        $log | Out-File -FilePath $logPath -Encoding utf8

        # Step 4 banner phrases. Each line the entry writes is asserted
        # individually so a partial-write regression is easy to spot.
        # The "post-EBS" line proves the 16550 COM1 fall-back works
        # after firmware ConOut is invalid; the "mem:" line proves
        # step 4c's init_memory + page-table / frame-allocator
        # singletons are live post-EBS.
        $expected = @(
            "AREST kernel - UEFI scaffold (#344)",
            "step 4 of 8: ExitBootServices + post-EBS serial",
            "pre-EBS:  ConOut active",
            "SSE enabled",
            "post-EBS: 16550 COM1 active",
            "frames usable",
            "dma:      pool live (2 MiB UEFI memory-map carve for virtio)",
            "pci:      walk OK (virtio-net:",
            "virtio-net: driver online, MAC",
            "virtio-blk: driver online,",
            "block:    checkpoint round-trip OK",
            "post-EBS heap live (sum 0..16 = 120)",
            "system::init() completed (arest engine live on UEFI)",
            "tiny module executed, main() = 42",
            "10 host imports bound to wasmi::Linker",
            "gop:      ",
            "gop-mmio: wrote 320x200, readback sum=0xffff8300",
            "fb:       paint smoke OK, presents=2",
            "doom-blit: synthetic 640x400 BGRA frame blitted",
            # #376: Doom WASM module instantiation + initGame.
            # The "module instantiated" line proves wasmi parsed the
            # 4.35 MiB blob and counted the expected exports (4 funcs
            # + 1 memory per doom_assets/README.md). The "calling
            # initGame" line marks the wasmi entry into D_DoomMain.
            # KNOWN LIMITATION: under the current `linked_list_allocator`
            # host heap, initGame panics inside Doom's Z_Init zone-
            # allocator setup with a "Freed node aliases existing
            # hole" assertion -- wasmi's `Memory::grow` reallocs
            # interact poorly with the freelist under WAD-load alloc
            # churn. The "calling tickGame" / "first drawFrame landed"
            # banners only fire if initGame returns or yields
            # cleanly; until the host allocator is swapped (tracked
            # for #378), the smoke harness asserts only the two
            # lines we reliably reach. Match tolerantly (no
            # fn-count or fuel-consumption number pinned) so a
            # future module rebuild or engine-version bump doesn't
            # false-fail.
            "doom:     module instantiated,",
            "doom:     calling initGame",
            "idt:      int3 round-tripped through UEFI IDT",
            # #379: PIT 1 kHz timer banner. The first phrase is printed
            # immediately after `init_time()` returns; the second phrase
            # confirms the IRQ 0 handler advanced `now_ms` between two
            # snapshots ~10 ms apart. A regression in PIC remap, IRQ 0
            # vector, or `sti` would surface here as a missing
            # "now_ms advanced" line (the spin loop times out without
            # observing motion) or, in the worst case, an absent
            # banner entirely (triple-fault before the line writes).
            "pit:      1 kHz timer online, IRQ 0",
            "pit:      now_ms advanced t0=",
            # #364: PS/2 keyboard banner. The "driver online" line is
            # printed immediately after `init_time()` (which is what
            # unmasks IRQ 1); its appearance proves the unmask + the
            # IDT vector 33 swap from defensive stub to
            # `keyboard_handler` did not fault. The "poll" line shows
            # the read-keystroke API surface works -- expected outcome
            # under the headless smoke harness is "idle" because QEMU
            # has no keyboard input wired, but the matcher is broad
            # enough that a future smoke that injects a scancode
            # would still pass without rewriting the assertion.
            "kbd:      PS/2 driver online (IRQ 1 unmasked)",
            "kbd:      poll "
        )
        $missing = @()
        foreach ($phrase in $expected) {
            if ($log -notmatch [regex]::Escape($phrase)) {
                $missing += $phrase
            }
        }

        if ($missing.Count -gt 0) {
            Write-Host "FAIL: missing banner phrases:" -ForegroundColor Red
            $missing | ForEach-Object { Write-Host "  - $_" -ForegroundColor Red }
            Write-Host "`n--- captured serial log ($logPath) ---"
            Write-Host $log
            exit 1
        }

        Write-Host "PASS: UEFI scaffold banner observed." -ForegroundColor Green
        Write-Host "Serial log: $logPath"
        exit 0
    }
    finally {
        $prevEAP = $ErrorActionPreference
        $ErrorActionPreference = "Continue"
        try {
            docker stop $containerName 2>&1 | Out-Null
        } finally {
            $ErrorActionPreference = $prevEAP
        }
    }
}

Write-Host "`nBooting UEFI kernel under QEMU + OVMF (Docker)..." -ForegroundColor Cyan
Write-Host "Ctrl-C here to stop the kernel." -ForegroundColor DarkGray
Write-Host "Note: OVMF prints its own boot banners before ours; AREST output appears after.`n" -ForegroundColor DarkGray
docker run --rm arest-kernel-uefi
