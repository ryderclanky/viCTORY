# Windows setup and repeatable runs

This guide records the Windows build path and the observed startup-only run on this PC. The run reached the opening intro/title card but did not reach gameplay; these measurements do not establish gameplay compatibility or a gameplay baseline.

## PC and prerequisites

PC inventory observed on 2026-11-25: Acer Nitro ANV15-52, Intel Core i7-13620H (10 cores / 16 logical processors), 16 GB system RAM, NVIDIA GeForce RTX 5060 Laptop GPU, and Intel UHD Graphics. The Windows display driver reported for the NVIDIA adapter was `32.0.16.1742`. These inventory values do not verify the GPU features the runtime needs.

The project requires Windows 10/11 x64, an AVX2-capable CPU, a Direct3D 12 GPU supporting Shader Model 6.6 and mesh shaders for world rendering, and the Rust version pinned in `rust-toolchain.toml` (Rust `1.98.1`, target `x86_64-pc-windows-msvc`). Rustup, that toolchain and target are present on this PC. The MSVC toolchain and linker were usable for the successful release and diagnostics builds recorded below. The successful startup run also confirms the program launched through its intro/title-card path on this PC; it is not a gameplay compatibility result.

The runtime may need several GiB of available Windows commit at startup. Check **Task Manager → Performance → Memory → Committed**. If `crashes.log` reports Windows error 1455, close other applications or increase the Windows paging file size and retry.

## Build and repository checks

Run these commands from the repository root in PowerShell:

```powershell
rustup show
cargo xtask install-hooks
cargo test --workspace --locked -j 1 -- --test-threads=1
cargo clippy --workspace --all-targets --locked -j 1
cargo build --locked --release -p victory -j 1
cargo build --locked --profile diag -p victory --features diagnostics -j 1
```

The release executable is `target\release\victory.exe`; the diagnostics executable is `target\diag\victory.exe`. Diagnostics builds include debug logging and additional runtime instrumentation. Build outputs stay under the ignored `target` directory.

Clippy may report existing dead-code warnings. The strict variant with `-- -D warnings` has not been verified clean for this checkout and should not be treated as a passing gate.

## Observed startup-only run on this PC

The local run used a diagnostics build and the authorized executable path resolved during setup. It reached the opening intro/title card and never reached gameplay. These are observations from an instrumented startup sequence, not a gameplay baseline or an upstream-equivalent performance benchmark:

| Startup scene / event | Observed rate and frame time | Draws | Notes |
|---|---:|---:|---|
| Initial account picker | 30 fps / 33.3 ms | 21 | Initial reading. |
| Account picker later in the same sequence | 6 fps / 167 ms | 5 | Severe slowdown despite fewer draws. |
| Switch-profile splash | About 22 fps / 45.7 ms | 6 | Diagnostics run. |
| Transition into title card | Below 1 fps; multi-second stalls | Not recorded | Never advanced into gameplay. |
| Follow-up run, intro artwork around 87 s | Renderer meter: 20.1 fps / 49.6 ms per frame | 10 | Same switch profile plus `VICTORY_CARD_BUFFERS_MB=256`; fully rendered intro artwork. Nearby frame-log windows had 33–39 ms medians and 213–3,110 ms long frames. |
| Follow-up black transition window | Median 424 ms; max 3,902 ms | Not recorded | Followed the intro artwork. |
| Follow-up next five-frame window | Median 424 ms; max 3,378 ms | Not recorded | Transition remained black. |
| Later passive observation, credits artwork | About 3 fps / 319.5 ms per frame | 3,342 | Settings and build unknown. The user reported a crash while saw-like audio remained audible; no crash report or dump was collected, so the failure path is unknown. |

Frame rates and draws above are title/runtime instrumentation. They are scene-local observations and must not be used as evidence of world rendering, playability, or a performance improvement. The memory readings below are approximate snapshots during startup/loading, not controlled steady-state samples:

| Run / elapsed time | Process RSS / private bytes | System available | dGPU memory | Scope / note |
|---|---:|---:|---:|---|
| Default diagnostics, 223 s | 3.74 / 10.68 GiB | About 0.4 GiB | About 3.94 GiB | dGPU value is adapter/system-wide, not process-specific. |
| Switch-profile diagnostics, 31 s | 4.65 / 8.89 GiB | 2.75 GiB | 2.39 GiB | Snapshot during startup. |
| Switch-profile diagnostics, 83 s | 5.18 / 9.83 GiB | 0.52 GiB | 2.97 GiB | Process RSS peak by this point: 6.52 GiB. |
| Switch-profile diagnostics, about 155 s | 4.09 / 17.89 GiB | 0.41 GiB | 6,279 MiB | Process RSS peak: 7.19 GiB. dGPU value is adapter-level. |
| Final diagnostics instrumentation, about 21:52:10 wall time | 3,288 MiB resident / 19,968 MiB private | 403 MiB free; 1,892 MiB commit headroom | 6,020 / 6,879 MiB card use/budget; 5,225 MiB shared | Renderer-reported figures, not process-specific GPU use. |

The displayed GPU memory numbers come from different observations: some are system/adapter-level, and the final figures are D3D instrumentation reports. They are not interchangeable with per-process GPU memory. The falling free-memory readings and rising private-byte totals show memory pressure during this instrumented startup run, but do not by themselves identify a leak or a single cause.

### Follow-up diagnostics run, 2026-11-25 17:01:46 local

This run used the same switch profile plus `VICTORY_CARD_BUFFERS_MB=256`. Frame captures were written under `%LOCALAPPDATA%`, outside the repository. The different buffer cap and capture instrumentation make its performance readings non-comparable as an improvement over the prior run.

The heap-capacity probe rejected requests for 2,097,152 and 1,572,864 with HRESULT `0x80070057`, then fell back to 1,000,000. The title still requests 1,245,184. The fallback is below the title's request, so this probe did not establish additional capacity or a rendering improvement on this PC.

Approximate memory snapshots from this run:

| Elapsed time | Process RSS / private bytes | System available | System-wide dGPU memory |
|---|---:|---:|---:|
| About 21 s | 2.58 / 6.13 GiB | 0.64 GiB | 1.49 GiB |
| About 118 s | 4.68 / 12.29 GiB | 0.58 GiB | 4,277 MiB (4.18 GiB) |
| About 120 s | 4.23 / 17.58 GiB | 0.32 GiB | 5,890 MiB (5.75 GiB) |

These are approximate startup/loading snapshots; GPU memory is system-wide, not process-specific. At about 120 seconds the process was not responding and was stopped manually. This was not recorded as an automatic crash or a device-removal event. The run still did not reach gameplay.

## Configuration and launch

Keep the user's authorized dump, config, cache, saves, and logs outside the checkout. `victory.toml` is git-ignored, but it is simplest to keep only paths there and leave the default cache and data directories under `%LOCALAPPDATA%\victory`. Start from the example and edit its game path:

```powershell
Copy-Item victory.example.toml victory.toml
notepad victory.toml
```

Set `[paths].game_dir` to the authorized game directory outside this repository. On this PC, the confirmed executable is under `%USERPROFILE%\Documents\GTA6\GTA6`; set the directory and executable paths in the current PowerShell session as follows. The game files and license material do not belong in this repository.

```powershell
$env:VICTORY_GAME_DIR = Join-Path $env:USERPROFILE 'Documents\GTA6\GTA6'
$TitleExe = Join-Path $env:USERPROFILE 'Documents\GTA6\GTA6\game_scarlett_master.exe'
& .\target\release\victory.exe run $TitleExe --execute
```

For an instrumented run, use the diagnostics executable. `VICTORY_KEYS` is supported only in diagnostics builds; each entry presses a key for 0.2 seconds at the given number of seconds after input starts. Supported keys are letters, digits, `Enter`, `Esc`, `Tab`, `Back`, `Up`, `Down`, `Left`, and `Right`.

```powershell
$env:VICTORY_FRAME_LOG = '1'
$env:VICTORY_KEYS = '30:Enter,35:X'
& .\target\diag\victory.exe run $TitleExe --execute
Remove-Item Env:VICTORY_KEYS, Env:VICTORY_FRAME_LOG -ErrorAction SilentlyContinue
```

Three switches are worth knowing before a run:

- `VICTORY_AMBIENT=<level>` (off unless set): the indirect diffuse light the title's rays would
  have filled is replaced with a constant right after the program that resolves it. `=off`
  restores the title's output, `=0` gives no indirect light, and a number sets the floor.
  `VICTORY_AMBIENT_TINT=r,g,b` and `VICTORY_AMBIENT_SHADE=strength,radius,near,tan` tune
  colour and depth-only occlusion. This is experimental and its visual result is unverified on
  this PC because the run did not reach gameplay.
- `VICTORY_BVH_EMPTY=1` (off): a program that tests rays against the ray-tracing structures is
  translated so every test misses. The traversal leaves after one node and the sun lights what
  would have been shadowed; it is the other candidate for lit interiors and needs its own run.
- `VICTORY_DISPATCH_SPLIT=1` (off): a console dispatch of more than 65,535 thread groups is run
  in slices, including a single indirect dispatch record whose shader reads the X group id.
  Multi-record indirect windows are not split.
- `VICTORY_CLAMP_AHEAD=1` (off): eligible single indirect records are clamped in a command
  list's prelude as a batch instead of adding a clamp pass before each draw. The batch behavior
  has a WARP test; game timing and rendering effects are unmeasured.

Memory controls with no value set:

- On a machine with less than 8,192 MiB of DXGI video-memory budget, or at most 20 GiB of
  physical RAM, `VICTORY_PROFILE` selects smaller-card defaults: target reserve is a quarter of
  the budget (512–3,072 MiB), texture room is 92%, on-demand texture mips are enabled, and the
  staging pool is 64 MiB when RAM is at most 20 GiB. `VICTORY_PROFILE=0` or `=large` keeps the
  shipped defaults; `=small` forces the small profile. These defaults have not been validated
  in a gameplay run on this PC.
- `VICTORY_TEXTURE_REFILL=<count>` limits new copies made through the indexed-texture queue per
  frame; existing host copies can still be refreshed. Copies created outside that queue are
  counted but not deferred by this limit. The default is off.
- `VICTORY_TEXTURE_EVICT_AFTER=<frames>` releases the oldest eligible texture copies only when
  measured VRAM use exceeds the queue's budget line. `VICTORY_TARGETS_ROOM_SHRINK=<frames>`
  releases unused target reserve after target creation has gone quiet. Both are off by default.
- `VICTORY_VRAM_IDLE_BY_ROOM=0` restores the former idle timeout based only on VRAM share. By
  default, the timeout follows whether the texture queue itself can accept another copy.

The memory defaults are sized from the machine at startup (`VICTORY_PROFILE=0` keeps the
shipped ones; see the README section "Memory, sized to the machine it runs on").

`VICTORY_FRAME_LOG=1` prints five-second summaries including frame count, shortest/median/longest frame times, individual frame times, the usual-frame breakdown, and separate long-frame breakdowns. This logging is enabled by default in diagnostics builds; the environment variable makes the measurement intent explicit. It adds instrumentation, so record that the run used a diagnostics build and compare like-for-like builds.

For diagnostics screenshots, create the output folder before launch; the capture code does not create it automatically:

```powershell
$CaptureDir = Join-Path $env:LOCALAPPDATA 'victory\logs\frames'
New-Item -ItemType Directory -Force -Path $CaptureDir | Out-Null
$env:VICTORY_CAPTURE = $CaptureDir
$env:VICTORY_CAPTURE_WORLD = '1'
$env:VICTORY_CAPTURE_WORLD_EVERY = '256'
```

On this 16 GB RAM / 8 GB VRAM laptop, the follow-up run used `VICTORY_CARD_BUFFERS_MB=256` and `VICTORY_STAGING_POOL=0`; optional card windows, upload windows and in-place pinning remained off. This conservative profile still became memory pressured and did not reach gameplay. The tried switch profile is the README's run 253 environment set, plus the 256 MiB card-buffer cap; it is an experiment, not a recommended performance preset. The 16 GB system memory and descriptor-heap limit remain blockers for this title on this PC. The newer adaptive-memory defaults and other pending worktree changes have not been measured in the same scene, so they do not yet establish an improvement over those observations.

## Record a comparable run

Use one game run at a time. For each run, save the transcript outside the checkout and record the exact build, command, settings, scene description, and elapsed time. Separate initial shader/pipeline compilation and loading from a steady-state sample. For comparisons, repeat the same route/view, display resolution and settings, use the same build profile, and compare similar draw counts. Do not infer a speedup from FPS alone; retain median and slow-frame times from the frame log.

```powershell
$LogDir = Join-Path $env:LOCALAPPDATA 'victory\logs'
New-Item -ItemType Directory -Force -Path $LogDir | Out-Null
$Stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
Start-Transcript -Path (Join-Path $LogDir "run-$Stamp.txt")
# Run the selected launch command here; stop it with Ctrl+C when finished.
Stop-Transcript
```

At the start and end of the same steady-state interval, also record:

- System **Committed** and **Available** memory from Task Manager → Performance → Memory.
- `victory.exe` working set/private memory in Task Manager → Details (enable the memory columns if needed).
- Dedicated and shared GPU memory use in Task Manager → Performance → GPU, identifying whether the figure is process-specific or system-wide.
- GPU adapter, driver version, display resolution, scene/view, settings, draw count if available, and any visible rendering faults, hangs, or crashes.

These system memory observations are separate from the renderer's per-frame timing log. Do not combine readings from different scenes or runs as though they were one sample.

## Current verification limits

On the current working tree, `cargo test --workspace --locked -j 1 -- --test-threads=1` passed with 616 tests passed, 12 ignored and 0 failed. Both `cargo build --locked --profile diag -p victory --features diagnostics -j 1` and `cargo build --locked --release -p victory -j 1` passed. `git diff --check` passed. Clippy and a clean `cargo fmt --all -- --check` result were not established.

The release and diagnostics builds passed, and the local diagnostics run reached the opening intro/title card. Gameplay and world rendering were not reached; their compatibility, stability, FPS and frame times remain unverified. The startup performance readings are instrumented, incomplete and memory pressured; they are not a gameplay baseline. The README's gameplay measurements describe a different test machine and are not a baseline for this PC. The later 3 fps credits-artwork sample had unknown settings and is not comparable to those earlier runs.
