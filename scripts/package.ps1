param([switch]$SkipBuild,[switch]$DebugBuild,[switch]$TestVariant,[switch]$DownloadTools,[string]$Python = 'python')
$ErrorActionPreference = 'Stop'
$project = Split-Path $PSScriptRoot -Parent
& $Python (Join-Path $PSScriptRoot 'locales.py')
if ($LASTEXITCODE -ne 0) { throw 'Translation catalog validation failed.' }
$toolArgs = @((Join-Path $PSScriptRoot 'bundle_tools.py'), '--sources')
if ($DownloadTools) { $toolArgs += '--download' }
& $Python @toolArgs
if ($LASTEXITCODE -ne 0) { throw 'Pregătirea/verificarea utilitarelor SQL a eșuat.' }
& (Join-Path $PSScriptRoot 'assets.ps1')
if (!$SkipBuild) {
    $arguments = @('build','--locked')
    if (!$DebugBuild) { $arguments += '--release' }
    & (Join-Path $PSScriptRoot 'cargo.ps1') @arguments
    if ($LASTEXITCODE -ne 0) { throw 'Compilarea a eșuat.' }
}
$profile = if ($DebugBuild) { 'debug' } else { 'release' }
$dist = Join-Path $project 'dist\StrajaBackupSQL'
New-Item -ItemType Directory -Path $dist -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $project "target\$profile\StrajaBackupSQL.exe") -Destination $dist -Force
Copy-Item -LiteralPath (Join-Path $project "target\$profile\StrajaBackupSQL-Agent.exe") -Destination $dist -Force
Copy-Item -LiteralPath (Join-Path $project 'README.md') -Destination $dist -Force
Copy-Item -LiteralPath (Join-Path $project 'THIRD-PARTY-NOTICES.txt') -Destination $dist -Force
Copy-Item -LiteralPath (Join-Path $project 'SQL-TOOLS-NOTICES.txt') -Destination $dist -Force
Copy-Item -LiteralPath (Join-Path $project 'docs') -Destination $dist -Recurse -Force
$toolsTarget = Join-Path $dist 'tools'
if (Test-Path -LiteralPath $toolsTarget) {
    $resolved = (Resolve-Path -LiteralPath $toolsTarget).Path
    if ($resolved -ne [System.IO.Path]::GetFullPath($toolsTarget) -or (Get-Item -LiteralPath $toolsTarget).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Unsafe tools staging directory.' }
    Remove-Item -LiteralPath $toolsTarget -Recurse -Force
}
Copy-Item -LiteralPath (Join-Path $project 'vendor-tools') -Destination $toolsTarget -Recurse
# App-local runtime for the Rust GUI and agent too; never install a system-wide runtime.
Get-ChildItem -LiteralPath (Join-Path $toolsTarget 'mariadb\bin') -File | Where-Object { $_.Name -match '^(vcruntime140|msvcp140|concrt140).*\.dll$' } | Copy-Item -Destination $dist -Force
& $Python (Join-Path $PSScriptRoot 'bundle_tools.py') --verify-only $toolsTarget --app-dir $dist
if ($LASTEXITCODE -ne 0) { throw 'Verificarea kitului a eșuat.' }
$inno = Join-Path (Split-Path $project -Parent) 'rust-backup\.tools\inno\ISCC.exe'
if (!(Test-Path -LiteralPath $inno)) { $inno = (Get-Command ISCC.exe -ErrorAction Stop).Source }
$version = [regex]::Match((Get-Content (Join-Path $project 'Cargo.toml') -Raw),'(?m)^version = "([^"]+)"').Groups[1].Value
$arguments = @("/DAppVersion=$version")
if ($TestVariant) { $arguments += '/DInstallerTest=1' }
$arguments += (Join-Path $project 'installer\straja-sql.iss')
& $inno @arguments
if ($LASTEXITCODE -ne 0) { throw 'Instalatorul nu a putut fi compilat.' }
$suffix = if ($TestVariant) { '-test' } else { '' }
$setup = Join-Path $project "dist\installer\StrajaBackupSQL-$version-windows-x64$suffix-setup.exe"
$hash = (Get-FileHash -LiteralPath $setup -Algorithm SHA256).Hash.ToLowerInvariant()
Set-Content -LiteralPath ($setup + '.sha256') -Value ($hash + '  ' + (Split-Path $setup -Leaf)) -Encoding ascii
Write-Host "Instalator: $setup"
