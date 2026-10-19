param([string]$Version)
$ErrorActionPreference = 'Stop'
$project = [IO.Path]::GetFullPath((Split-Path $PSScriptRoot -Parent))
if (!$Version) {
    $Version = [regex]::Match((Get-Content (Join-Path $project 'Cargo.toml') -Raw),'(?m)^version = "([^"]+)"').Groups[1].Value
}
if ($Version -notmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$') { throw 'Use a stable semantic version such as 1.2.3.' }
$destination = Join-Path $project 'dist\update-feed\updates\strajabackupsql\windows-x64'
New-Item -ItemType Directory -Path $destination -Force | Out-Null
$feed = [ordered]@{
    schema_version=1
    product='strajabackupsql'
    platform='windows-x64'
    channel='stable'
    version=$Version
    release_url="https://github.com/weblikero/straja-backup-sql/releases/tag/v$Version"
}
# UTF-8 without BOM: the application parses JSON bytes, not a text editor document.
[IO.File]::WriteAllText((Join-Path $destination 'stable.json'), ($feed | ConvertTo-Json) + "`n", [Text.UTF8Encoding]::new($false))
Write-Host "Prepared locally: $destination\stable.json"
Write-Host 'Publish this file only after the matching stable release and its installer are public and tested.'
Write-Host 'Upload to https://strajabackup.com/updates/strajabackupsql/windows-x64/stable.json with application/json and Cache-Control: max-age=300.'
