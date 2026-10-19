$ErrorActionPreference = 'Stop'
$project = Split-Path $PSScriptRoot -Parent
$metadata = (& (Join-Path $PSScriptRoot 'cargo.ps1') metadata --format-version 1 --locked) | ConvertFrom-Json
if (!$metadata.packages) { throw 'Cargo metadata unavailable.' }
$text = [Text.StringBuilder]::new()
[void]$text.AppendLine('STRAJA SQL — THIRD-PARTY NOTICES')
[void]$text.AppendLine('Dependency inventory from Cargo.lock. Includes target-specific and development dependencies that may not be linked into the Windows binary.')
foreach ($package in ($metadata.packages | Sort-Object name,version)) {
    if ($package.name -eq 'straja-sql') { continue }
    [void]$text.AppendLine("`n============================================================")
    [void]$text.AppendLine("$($package.name) $($package.version) — $($package.license)")
    [void]$text.AppendLine("Source: $($package.repository)")
    $directory = Split-Path $package.manifest_path -Parent
    $files = Get-ChildItem -LiteralPath $directory -File | Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE)' }
    foreach ($file in $files) { [void]$text.AppendLine("`n$($file.Name)"); [void]$text.AppendLine([IO.File]::ReadAllText($file.FullName)) }
}
[void]$text.AppendLine("`nInstaller: Inno Setup, copyright Jordan Russell and contributors. See https://jrsoftware.org/isinfo.php. The local compiler reports non-commercial use only; commercial packaging requires the appropriate compiler license.")
[IO.File]::WriteAllText((Join-Path $project 'THIRD-PARTY-NOTICES.txt'),$text.ToString(),[Text.UTF8Encoding]::new($false))
