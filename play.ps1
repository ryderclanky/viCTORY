# Nitro Series S preset (16 GB RAM, 8 GB VRAM) and a shader cache that
# survives reboots.
#
# From the repository root:
#   .\play.ps1 run <title exe> --execute
#
# The cache is %LOCALAPPDATA%\victory\cache on every launch. The script does
# not delete it. VICTORY_SHADER_CACHE_KEY is left unset so a rebuild that does
# not change the translator keeps its translations.
#
# Windows stay 4 MiB. A 1 MiB grid, tails, and shared buffer heaps raised
# private RAM from 17.3 GB to 18.5 GB and stalled frames for 5-8 seconds.
# VICTORY_LIGHT_REFRESH, VICTORY_JOIN_COPIES, VICTORY_DIRTY_4K,
# VICTORY_CARD_REHASH_RUNS, and VICTORY_WIDE_MERGE stay unset: that preset
# dropped prison-cell gameplay from 4 fps to 2 fps and removed the device.
#
# One line of stage times per presented frame (off here; set it in this
# window before the script if the next run should say where the milliseconds
# went):
#   $env:VICTORY_FRAME_TIMINGS = '1'

$ErrorActionPreference = 'Stop'

if (-not $env:LOCALAPPDATA) {
    Write-Error 'LOCALAPPDATA is not set; set VICTORY_CACHE_DIR to a folder outside the repository.'
}

$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
$Cache = Join-Path $env:LOCALAPPDATA 'victory\cache'
$Data = Join-Path $env:LOCALAPPDATA 'victory\data'
New-Item -ItemType Directory -Force -Path $Cache, $Data | Out-Null

$env:VICTORY_CACHE_DIR = $Cache
$env:VICTORY_DATA_DIR = $Data
$env:VICTORY_CONSOLE = 'series-s'
$env:VICTORY_PROFILE = 'small'
$env:VICTORY_TEXTURE_DROP_MIPS = '2'
$env:VICTORY_TEXTURE_HEAPS = '1'
$env:VICTORY_TEXTURE_HEAP_MB = '64'
$env:VICTORY_AMBIENT = '0.25'
$env:VICTORY_TEXTURE_BACK = '1'
$env:VICTORY_HEAP_WORKER = '1'
$env:VICTORY_LOOKUP_CACHE = '1'
$env:VICTORY_SETTLE_ROOM = '1'
$env:VICTORY_PARALLEL_REFRESH = '1'
# A title copy no longer flushes every pending refresh. A range already
# brought up to date is not copied again until title memory is written.
# Copies into host buffers are the processor's, with the kernel behind a
# missing page.
$env:VICTORY_NEAR_WRITES = '1'
$env:VICTORY_REFRESH_ONCE = '1'
$env:VICTORY_COPY_PLAIN = '1'
$env:VICTORY_INPLACE = '0'
$env:VICTORY_CARD_FETCH_INFLIGHT = '0'
Remove-Item Env:VICTORY_SHADER_CACHE_KEY -ErrorAction SilentlyContinue
Remove-Item Env:VICTORY_BUFFER_HEAPS -ErrorAction SilentlyContinue
Remove-Item Env:VICTORY_WINDOW_KB -ErrorAction SilentlyContinue
Remove-Item Env:VICTORY_WINDOW_TAILS -ErrorAction SilentlyContinue
Remove-Item Env:VICTORY_LIGHT_REFRESH -ErrorAction SilentlyContinue
Remove-Item Env:VICTORY_JOIN_COPIES -ErrorAction SilentlyContinue
Remove-Item Env:VICTORY_DIRTY_4K -ErrorAction SilentlyContinue
Remove-Item Env:VICTORY_CARD_REHASH_RUNS -ErrorAction SilentlyContinue
Remove-Item Env:VICTORY_WIDE_MERGE -ErrorAction SilentlyContinue

$Release = Join-Path $Root 'target\release\victory.exe'
$Diag = Join-Path $Root 'target\diag\victory.exe'
if (Test-Path $Release) {
    $Victory = $Release
} elseif (Test-Path $Diag) {
    $Victory = $Diag
} else {
    Write-Error "no victory.exe under target\release or target\diag. Build it first (cargo build --locked --release -p victory)."
}

& $Victory @args
exit $LASTEXITCODE
