param(
    [Parameter(Mandatory=$true)][string]$NotesFile,
    [ValidateSet('preview','stable')][string]$Channel = 'preview',
    [switch]$Publish,
    [switch]$DryRun,
    [string]$GitHubCli = 'gh.exe'
)
$ErrorActionPreference = 'Stop'
$project = [IO.Path]::GetFullPath((Split-Path $PSScriptRoot -Parent))
$repo = 'weblikero/straja-backup-sql'
$version = [regex]::Match((Get-Content (Join-Path $project 'Cargo.toml') -Raw),'(?m)^version = "([^"]+)"').Groups[1].Value
if ($version -notmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$') { throw 'Expected a stable numeric package version.' }
$tag = "v$version"
$kit = Join-Path $project "dist\release\$version"
$manifestPath = Join-Path $kit 'release-manifest.json'
$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
if ($manifest.Product -ne 'StrajaBackupSQL' -or $manifest.Version -ne $version -or $manifest.Platform -ne 'windows-x64') { throw 'Release kit product/version mismatch.' }
if ($manifest.SourceCommit -notmatch '^[0-9a-f]{40}$') { throw 'Rebuild the release kit with a recorded source commit.' }
$notes = (Resolve-Path -LiteralPath $NotesFile).Path
if (!(Get-Content -LiteralPath $notes -Raw).Trim()) { throw 'Release notes must not be empty.' }
$assets = @()
$names = @{}
foreach ($entry in $manifest.Files) {
    # Only the curated release kit can become a public asset, never workspace files.
    if ($entry.Path -notmatch '^(docs/[^/]+\.(txt|json)|sources/[^/]+|README\.md|THIRD-PARTY-NOTICES\.txt|SQL-TOOLS-NOTICES\.txt|StrajaBackupSQL-[0-9.]+-windows-x64-setup\.exe(?:\.sha256)?)$') { throw "Unexpected release asset: $($entry.Path)" }
    $path = [IO.Path]::GetFullPath((Join-Path $kit $entry.Path))
    if (!$path.StartsWith($kit + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Asset escapes the release kit.' }
    $item = Get-Item -LiteralPath $path
    if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw "Invalid release asset: $path" }
    $hash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($item.Length -ne $entry.Bytes -or $hash -ne $entry.Sha256) { throw "Release asset has changed: $path" }
    if ($names.ContainsKey($item.Name)) { throw "Duplicate asset name: $($item.Name)" }
    $names[$item.Name] = $hash
    $assets += $path
}
$installer = "StrajaBackupSQL-$version-windows-x64-setup.exe"
foreach ($required in @($installer, "$installer.sha256", 'SQL-TOOLS-NOTICES.txt','THIRD-PARTY-NOTICES.txt')) {
    if (!$names.ContainsKey($required)) { throw "Missing required asset: $required" }
}
if (@($manifest.Files | Where-Object { $_.Path -match '^sources/.*\.(zip|gz|xz)$' }).Count -lt 2) { throw 'Corresponding SQL utility source archives are missing.' }
$assets += $manifestPath
Write-Host "Release: $repo / $tag ($Channel), source $($manifest.SourceCommit)"
Write-Host "Validated $($assets.Count) assets. Publish: $Publish"
if ($DryRun) { Write-Host 'Dry run: no GitHub requests or uploads.'; return }
$gh = (Get-Command $GitHubCli -ErrorAction Stop).Source
function Invoke-Gh([string[]]$Arguments) {
    $output = & $gh @Arguments
    if ($LASTEXITCODE -ne 0) { throw "GitHub CLI failed: $($Arguments[0]) $($Arguments[1]). Any incomplete release stays a draft; rerun to resume." }
    return $output
}
# The exact source revision must already exist in the product's public repository.
$remoteCommit = Invoke-Gh @('api', "repos/$repo/commits/$($manifest.SourceCommit)", '--jq', '.sha')
if ($remoteCommit -ne $manifest.SourceCommit) { throw 'Source commit is not published in the expected repository.' }
$refs = Invoke-Gh @('api', "repos/$repo/git/matching-refs/tags/$tag") | Out-String | ConvertFrom-Json
$tagRef = @($refs | Where-Object { $_.ref -eq "refs/tags/$tag" })
if ($tagRef.Count -gt 0 -and ($tagRef[0].object.type -ne 'commit' -or $tagRef[0].object.sha -ne $manifest.SourceCommit)) { throw 'Existing tag does not directly identify the release source commit.' }
$releases = (Invoke-Gh @('api', "repos/$repo/releases?per_page=100") | Out-String | ConvertFrom-Json)
$existing = @($releases | Where-Object { $_.tag_name -eq $tag })
if ($Publish -and $Channel -eq 'stable') {
    foreach ($release in $releases) {
        if (!$release.draft -and !$release.prerelease -and $release.tag_name -match '^v([0-9]+\.[0-9]+\.[0-9]+)$' -and [version]$Matches[1] -gt [version]$version) { throw 'A newer stable version already exists; refusing to move Latest backwards.' }
    }
}
if ($existing.Count -gt 1) { throw 'Ambiguous release tag.' }
if ($existing.Count -eq 1 -and !$existing[0].draft) { throw 'This version is already public. Publish a new version; public assets are immutable.' }
if ($existing.Count -eq 1 -and $existing[0].target_commitish -ne $manifest.SourceCommit) { throw 'Draft belongs to another source revision.' }
if ($existing.Count -eq 0) {
    Invoke-Gh @('release','create',$tag,'--repo',$repo,'--target',$manifest.SourceCommit,'--title',"StrajaBackupSQL $version",'--notes-file',$notes,'--draft') | Out-Host
}
# Draft tags may not exist yet; the REST endpoint by tag can return 404 until publication.
$drafts = Invoke-Gh @('api', "repos/$repo/releases?per_page=100") | Out-String | ConvertFrom-Json
$draft = @($drafts | Where-Object { $_.tag_name -eq $tag })
if ($draft.Count -ne 1 -or !$draft[0].draft -or $draft[0].target_commitish -ne $manifest.SourceCommit) { throw 'Cannot identify the expected draft release.' }
$releaseId = $draft[0].id
# Upload while hidden. Resume only drafts; never replace files in a public release.
foreach ($asset in $assets) {
    $item = Get-Item -LiteralPath $asset
    $digest = 'sha256:' + (Get-FileHash -LiteralPath $asset -Algorithm SHA256).Hash.ToLowerInvariant()
    $previous = @($draft[0].assets | Where-Object { $_.name -eq $item.Name })
    if ($previous.Count -eq 1 -and $previous[0].state -eq 'uploaded' -and $previous[0].size -eq $item.Length -and $previous[0].digest -eq $digest) {
        Write-Host "Already verified: $($item.Name)"
        continue
    }
    Write-Host "Uploading: $($item.Name)"
    Invoke-Gh @('release','upload',$tag,$asset,'--repo',$repo,'--clobber') | Out-Host
}
$remote = Invoke-Gh @('api', "repos/$repo/releases/$releaseId") | Out-String | ConvertFrom-Json
if (!$remote.draft) { throw 'Release is no longer a draft; stopped.' }
foreach ($asset in $assets) {
    $local = Get-Item -LiteralPath $asset
    $uploaded = @($remote.assets | Where-Object { $_.name -eq $local.Name })
    $digest = 'sha256:' + (Get-FileHash -LiteralPath $asset -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($uploaded.Count -ne 1 -or $uploaded[0].state -ne 'uploaded' -or $uploaded[0].size -ne $local.Length -or $uploaded[0].digest -ne $digest) { throw "Uploaded asset verification failed: $($local.Name). Release stays a draft." }
}
if ($remote.assets.Count -ne $assets.Count) { throw 'Draft contains unexpected assets. Review the draft before publishing.' }
$edit = @('release','edit',$tag,'--repo',$repo,'--notes-file',$notes)
if ($Channel -eq 'stable') { $edit += @('--prerelease=false','--latest') }
else { $edit += @('--prerelease','--latest=false') }
if ($Publish) { $edit += '--draft=false' }
Invoke-Gh $edit | Out-Host
Write-Host "https://github.com/$repo/releases/tag/$tag"
if ($Publish -and $Channel -eq 'stable') { Write-Host 'Stable update is public. Installed apps detect it on their next check.' }
else { Write-Host 'Drafts and previews are not offered as stable application updates.' }
