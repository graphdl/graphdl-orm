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
    docker build -t arest-kernel-uefi -f "$repoRoot\crates\arest-kernel\Dockerfile.uefi" $repoRoot
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
        # -p 8080:8080: bridge container's exposed 8080 to the host so the
        # post-launcher curl step (#361) can reach the kernel's HTTP listener
        # via QEMU's hostfwd=tcp::8080-:80.
        & docker run --rm --name $containerName -d -p 8080:8080 arest-kernel-uefi | Out-Null
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
            # Beacon: the launcher print is the LAST thing
            # entry_uefi.rs writes before yielding to Slint's event
            # loop (#431 UUU). When this appears the boot is fully
            # quiesced and every preceding banner is in the log.
            if ($log -match "ui:\s+launcher running") { break }
            # Defensive: if the kernel panics before the launcher line,
            # surface that early instead of waiting out the cap.
            if ($log -match "UEFI kernel panic") { break }
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
            "dma:      pool live",
            "pci:      walk OK (virtio-net:",
            "virtio-net: driver online, MAC",
            "virtio-blk: driver online,",
            "block:    checkpoint round-trip OK",
            "post-EBS heap live (sum 0..16 = 120)",
            "system::init() completed (arest engine live on UEFI)",
            "tiny module executed, main() = 42",
            "gop:      ",
            "gop-mmio: wrote 320x200, readback sum=0xffff8300",
            "fb:       paint smoke OK, presents=2",
            "doom-blit: synthetic 640x400 BGRA frame blitted",
            # virtio-gpu surface (#371 Track III). The driver-online line
            # proves the virtio-drivers VirtIOGpu init succeeded against
            # the discovered PCI device + carved DMA pool; the install
            # line proves framebuffer::install_virtio_gpu picked it up
            # as the front-buffer (preferred over GOP per #382).
            "virtio-gpu: driver online,",
            "fb:       virtio-gpu surface installed",
            # virtio-input wire-up (#464 Track EEEE). Dockerfile.uefi
            # CMD adds `-device virtio-keyboard-pci -device
            # virtio-tablet-pci`, so PCI scan finds two slots at
            # vendor 0x1AF4 / device 0x1052 in QEMU enumeration order.
            # The linuxkpi wire-up at the bottom of `entry_uefi.rs`
            # iterates them, drives each through the shim's
            # device-register / driver-probe path, and prints one
            # banner line per slot. Discrimination on the foundation
            # slice rides on the QEMU CMD line ordering (keyboard
            # before tablet); when the linuxkpi shim's virtio
            # transport is fully wired (post-#464), this flips to a
            # real EV_BITS config-space read at probe time. The
            # `(slot ` substring matches the PCI coordinate format
            # `XX:YY.Z` the wire-up emits.
            "virtio-input: keyboard online (slot ",
            "virtio-input: tablet online (slot ",
            # Doom is gated behind --features doom (#456 / VVV). The
            # default build prints "doom: skipped" so the AGPL-only
            # binary is observable in the boot log. With the feature
            # on, the line would be "doom: module instantiated" /
            # "doom: calling initGame" instead — assert tolerantly.
            "doom:     skipped (build without --features doom",
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
            "kbd:      poll ",
            # #360 NNN — register_http on UEFI x86_64.
            "http:     handler registered on :80",
            # #359 DDD — net::init under UEFI x86_64 (smoltcp phy bind).
            "net:      smoltcp interface live",
            # #365 GGG — REPL on UEFI; line-buffered keystrokes feed
            # crate::repl::process_key (kept by VVV alongside the new
            # Slint REPL app).
            "repl:     line-buffered keyboard REPL online",
            # #431 UUU — Slint launcher takes over the boot screen and
            # hosts HATEOAS + REPL (and Doom under --features doom).
            # This is the smoke beacon — the very last line entry_uefi
            # writes before yielding to Slint's event loop. Polling
            # breaks on this string, so its presence here is partly
            # belt + suspenders (the loop already broke on it).
            "ui:       launcher running"
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

        # #361: curl the kernel's HTTP listener from the host. Soft check —
        # smoke PASSES on the banner observation alone (the in-kernel
        # network/handler path is already verified there). The curl seals
        # the host-reachable half (host -> docker:8080 -> QEMU hostfwd
        # -> guest:80) when it works; if it doesn't, surface a WARNING
        # rather than failing because:
        #   - Slint's event loop pauses when idle, so net::poll() may
        #     fire only every few hundred ms, slowing DHCPv4 settle.
        #   - QEMU SLiRP DHCPv4 typically resolves in <1 s but UEFI
        #     boot timing variance can push past 30 s on a cold container.
        # The kernel banner stack already covers what register_http +
        # net::init being live needs to prove; the host-reachability
        # piece is bonus. Promote to hard-fail once the timing is
        # consistently < 30 s.
        $curlOk = $false
        $curlBody = $null
        $curlDeadline = (Get-Date).AddSeconds(45)
        while ((Get-Date) -lt $curlDeadline) {
            try {
                $resp = Invoke-WebRequest -Uri "http://localhost:8080/" -TimeoutSec 3 -ErrorAction Stop
                if ($resp.StatusCode -ge 200 -and $resp.StatusCode -lt 500) {
                    $curlOk = $true
                    $curlBody = $resp.StatusCode
                    break
                }
            } catch {
                Start-Sleep -Milliseconds 500
            }
        }

        if (-not $curlOk) {
            Write-Host "WARN: banner OK but http://localhost:8080/ unreachable from host within 45 s (#361)." -ForegroundColor Yellow
            Write-Host "      Network path: host -> docker -p 8080 -> QEMU hostfwd -> guest :80." -ForegroundColor Yellow
            Write-Host "      Banner-level smoke still PASSES; the in-kernel side is verified." -ForegroundColor Yellow
        } else {
            Write-Host "PASS: http://localhost:8080/ reachable (HTTP $curlBody) — full host curl path verified." -ForegroundColor Green
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
