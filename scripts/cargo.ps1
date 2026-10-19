param([Parameter(ValueFromRemainingArguments = $true)][string[]]$CargoArgs)
$ErrorActionPreference = 'Stop'
$project = Split-Path $PSScriptRoot -Parent
$buildTemp = Join-Path $project 'target\tmp'
New-Item -ItemType Directory -Path $buildTemp -Force | Out-Null
$env:TEMP = $buildTemp
$env:TMP = $buildTemp
$localTools = Join-Path (Split-Path $project -Parent) 'rust-backup\.tools'
if (Test-Path (Join-Path $localTools 'cargo\bin\cargo.exe')) {
    $env:CARGO_HOME = Join-Path $localTools 'cargo'
    $env:RUSTUP_HOME = Join-Path $localTools 'rustup'
    $cargo = Join-Path $env:CARGO_HOME 'bin\cargo.exe'
    # Reuse compiled dependencies, without changing the other application's sources.
    $env:CARGO_TARGET_DIR = Join-Path $project 'target'
} else { $cargo = 'cargo' }
Push-Location $project
try { & $cargo @CargoArgs; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE } }
finally { Pop-Location }
