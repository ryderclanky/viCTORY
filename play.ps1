# Laptop preset (16 GB RAM, 8 GB VRAM) and a shader cache that survives reboots.
#
# From the repository root:
#   .\play.ps1 run <title exe> --execute
#
# The cache is %LOCALAPPDATA%\victory\cache on every launch. The script does
# not delete it. VICTORY_SHADER_CACHE_KEY is left unset so a rebuild that does
# not change the translator keeps its translations.

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
$env:VICTORY_INPLACE = '0'
$env:VICTORY_CARD_FETCH_INFLIGHT = '0'
Remove-Item Env:VICTORY_SHADER_CACHE_KEY -ErrorAction SilentlyContinue

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
