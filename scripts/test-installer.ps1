param([ValidateSet('en','de','es','pt','fr','it')][string[]]$Languages = @('en','de','es','pt','fr','it'))
$ErrorActionPreference = 'Stop'
function Invoke-TestInstaller([string]$Executable, [string[]]$Arguments) {
    $child = Start-Process -FilePath $Executable -ArgumentList $Arguments -WindowStyle Hidden -PassThru
    if (!$child.WaitForExit(60000)) {
        # The process tree was created by this test; silent installs never launch the app.
        $child.Kill($true)
        $child.WaitForExit()
        throw 'Installer initialization/execution exceeded 60 seconds; its test process tree was stopped. See the install log.'
    }
    return $child.ExitCode
}
$project = [IO.Path]::GetFullPath((Split-Path $PSScriptRoot -Parent))
$version = [regex]::Match((Get-Content (Join-Path $project 'Cargo.toml') -Raw),'(?m)^version = "([^"]+)"').Groups[1].Value
$setup = Join-Path $project "dist\installer\StrajaBackupSQL-$version-windows-x64-test-setup.exe"
if (!(Test-Path -LiteralPath $setup)) { throw "Build the isolated test installer first: $setup" }
$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{34CA39E7-0EBA-42C2-9042-AD11A00DF38C}_is1'
$startupKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$startupName = 'StrajaBackupSQLInstallerTest.Agent'
if (Test-Path -LiteralPath $key) { throw 'A test installation already exists. It will not be overwritten.' }
if (Get-ItemProperty -LiteralPath $startupKey -Name $startupName -ErrorAction SilentlyContinue) { throw 'A test startup entry already exists. It will not be overwritten.' }
$root = [IO.Path]::GetFullPath((Join-Path $project ('test-output\installer-' + $version + '-' + [guid]::NewGuid().ToString('N'))))
if (!$root.StartsWith($project + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Test directory is outside the workspace.' }
$target = Join-Path $root 'installed'
New-Item -ItemType Directory -Path $root -Force | Out-Null
$marker = Join-Path $root 'backup-preserve.txt'
Set-Content -LiteralPath $marker -Value 'External backup must survive installation and uninstall.'
$markerHash = (Get-FileHash -LiteralPath $marker).Hash
$checks = [Collections.Generic.List[object]]::new()
$failure = $null
$cleanupFailure = $null
$uninstalled = $false
try {
    foreach ($language in $Languages) {
        $arguments = @('/SP-','/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/TASKS="agentlogin"',"/LANG=$language","/DIR=`"$target`"","/LOG=`"$(Join-Path $root ('install-' + $language + '.log'))`"")
        # All [Run] entries skip silent installs: neither GUI nor agent is started.
        $exitCode = Invoke-TestInstaller $setup $arguments
        if ($exitCode -ne 0) { throw "Install ($language) exit code: $exitCode" }
        $entry = Get-ItemProperty -LiteralPath $key
        if ($entry.DisplayName -ne 'StrajaBackupSQL Installer Test' -or $entry.DisplayVersion -ne $version) { throw 'Installed Apps identity/version mismatch.' }
        foreach ($name in @('StrajaBackupSQL.exe','StrajaBackupSQL-Agent.exe')) {
            if ((Get-FileHash -LiteralPath (Join-Path $target $name)).Hash -ne (Get-FileHash -LiteralPath (Join-Path $project ('dist\StrajaBackupSQL\' + $name))).Hash) { throw "Installed binary hash mismatch: $name" }
        }
        $installedAgent = Join-Path $target 'StrajaBackupSQL-Agent.exe'
        if ((Get-Item -LiteralPath $installedAgent).VersionInfo.FileDescription -ne 'StrajaBackupSQL Agent') { throw 'SQL agent identity mismatch.' }
        if ((Get-ItemPropertyValue -LiteralPath $startupKey -Name $startupName) -ne ('"' + $installedAgent + '"')) { throw 'Test startup entry does not target its isolated SQL agent.' }
        if ((Get-Content -LiteralPath (Join-Path $target 'default-language.txt') -Raw).Trim() -ne $language) { throw "Installed language mismatch: $language" }
        foreach ($code in @('en','de','es','pt','fr','it')) {
            if (!(Test-Path -LiteralPath (Join-Path $target "docs\Guide.$code.txt"))) { throw "Missing guide: $code" }
        }
        foreach ($client in @('mariadb\bin\mariadb-dump.exe','mysql\bin\mysqldump.exe','postgresql\bin\pg_dump.exe')) {
            if (!(Test-Path -LiteralPath (Join-Path $target ('tools\' + $client)))) { throw "Missing bundled utility: $client" }
        }
        $userFile = Join-Path $target 'user-created-backup.sql'
        if (!(Test-Path -LiteralPath $userFile)) { Set-Content -LiteralPath $userFile -Value 'User-created backup fixture; never execute.' }
        if ((Get-Content -LiteralPath $userFile -Raw).Trim() -ne 'User-created backup fixture; never execute.') { throw 'Upgrade changed user-created data.' }
        $checks.Add([pscustomobject]@{Language=$language;InstallOrUpgrade='Passed';Binaries='Passed';BundledTools='Passed';StartupIdentity='Passed';UserData='Preserved'})
    }
} catch { $failure = $_.Exception.Message }
finally {
    # Only the uninstaller belonging to this invocation's isolated target may run.
    $uninstaller = Join-Path $target 'unins000.exe'
    if (Test-Path -LiteralPath $uninstaller) {
        try {
            $arguments = @('/SP-','/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART',"/LOG=`"$(Join-Path $root 'uninstall.log')`"")
            $exitCode = Invoke-TestInstaller $uninstaller $arguments
            if ($exitCode -ne 0) { throw "Uninstall exit code: $exitCode" }
            if (Test-Path -LiteralPath $key) { throw 'Test uninstall registration remains.' }
            if (Get-ItemProperty -LiteralPath $startupKey -Name $startupName -ErrorAction SilentlyContinue) { throw 'Test startup entry remains.' }
            $remaining = @(Get-ChildItem -LiteralPath $target -File -Recurse -ErrorAction SilentlyContinue)
            if ($remaining.Count -ne 1 -or $remaining[0].Name -ne 'user-created-backup.sql') { throw 'Uninstall did not leave exactly the user-created file.' }
            if ((Get-FileHash -LiteralPath $marker).Hash -ne $markerHash) { throw 'External backup preservation failed.' }
            $uninstalled = $true
        } catch { $cleanupFailure = $_.Exception.Message }
    }
    [pscustomobject]@{Version=$version;Checks=$checks.ToArray();UninstallPassed=$uninstalled;Failure=$failure;CleanupFailure=$cleanupFailure;Root=$root;Timestamp=(Get-Date).ToUniversalTime().ToString('o')} | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $root 'result.json') -Encoding utf8
}
if ($failure -or $cleanupFailure) { throw "Test failure: $failure; cleanup: $cleanupFailure. Report: $root\result.json" }
Write-Host "Six-language installation/upgrade checks and uninstall passed. Test registration removed; user data preserved. Report: $root\result.json"
