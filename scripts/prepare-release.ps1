param()
$ErrorActionPreference = 'Stop'
$project = [IO.Path]::GetFullPath((Split-Path $PSScriptRoot -Parent))
$version = [regex]::Match((Get-Content (Join-Path $project 'Cargo.toml') -Raw),'(?m)^version = "([^"]+)"').Groups[1].Value
$name = "StrajaBackupSQL-$version-windows-x64-setup.exe"
$installer = Join-Path $project ('dist\installer\' + $name)
if (!(Test-Path -LiteralPath $installer)) { throw 'Build the normal installer first.' }
$hash = (Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash.ToLowerInvariant()
if ((Get-Content -LiteralPath ($installer + '.sha256') -Raw).Split(' ')[0] -ne $hash) { throw 'Installer checksum mismatch.' }
$destination = [IO.Path]::GetFullPath((Join-Path $project ('dist\release\' + $version)))
if (!$destination.StartsWith($project + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Release destination is outside the workspace.' }
# A versioned release is immutable: a changed build must get a new version/folder.
if (Test-Path -LiteralPath $destination) { throw "Release folder already exists: $destination" }
New-Item -ItemType Directory -Path $destination -Force | Out-Null
Copy-Item -LiteralPath $installer,($installer + '.sha256') -Destination $destination
foreach ($name in @('README.md','THIRD-PARTY-NOTICES.txt','SQL-TOOLS-NOTICES.txt')) {
    Copy-Item -LiteralPath (Join-Path $project $name) -Destination $destination
}
Copy-Item -LiteralPath (Join-Path $project 'docs'),(Join-Path $project 'dist\sources') -Destination $destination -Recurse
$files = @(Get-ChildItem -LiteralPath $destination -File -Recurse | Sort-Object FullName | ForEach-Object {
    [pscustomobject]@{Path=$_.FullName.Substring($destination.Length + 1).Replace('\','/');Bytes=$_.Length;Sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()}
})
$signature = Get-AuthenticodeSignature -LiteralPath $installer
[pscustomobject]@{
    Product='StrajaBackupSQL';Version=$version;Channel='preview';Platform='windows-x64';
    DefaultLanguage='en';Languages=@('en','de','es','pt','fr','it');Website='https://strajabackup.com';
    ReadyForPublicRelease=$false;ValidationReport="docs/VALIDATION-$version.txt";
    InstallerSignatureStatus=$signature.Status.ToString();
    CreatedUtc=(Get-Date).ToUniversalTime().ToString('o');Files=$files
} | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $destination 'release-manifest.json') -Encoding utf8
Write-Host "Local release kit prepared: $destination"
Write-Host 'Nothing was uploaded. Complete clean-machine testing and release review before publication.'
