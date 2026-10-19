"""Offline release-publisher integration tests; fake files and fake GitHub CLI only."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
MOCK = r'''
$global:LASTEXITCODE = 0
$statePath = Join-Path $PSScriptRoot 'state.json'
$state = Get-Content $statePath -Raw | ConvertFrom-Json
Add-Content (Join-Path $PSScriptRoot 'calls.txt') ($args -join '|')
if ($args[0] -eq 'api') {
    if ($args[1] -like '*/commits/*') { 'a' * 40; return }
    if ($args[1] -like '*/matching-refs/*') { '[]'; return }
    if ($args[1] -like '*per_page*') {
        if ($state.release) { '[' + ($state.release | ConvertTo-Json -Depth 8 -Compress) + ']' } else { '[]' }
        return
    }
    $result = $state.release
    if ($state.badDigest) { $result.assets[0].digest = 'sha256:wrong' }
    $result | ConvertTo-Json -Depth 8 -Compress
    return
}
if ($args[1] -eq 'create') {
    $state.release = [pscustomobject]@{tag_name='v1.2.3';draft=$true;target_commitish=('a'*40);assets=@()}
}
if ($args[1] -eq 'upload') {
    if ($state.failUpload) { $global:LASTEXITCODE = 1; return }
    $item = Get-Item $args[3]
    $asset = [pscustomobject]@{name=$item.Name;size=$item.Length;state='uploaded';digest=('sha256:' + (Get-FileHash $item.FullName -Algorithm SHA256).Hash.ToLowerInvariant())}
    $state.release.assets = @($state.release.assets | Where-Object { $_.name -ne $item.Name }) + @($asset)
}
if ($args[1] -eq 'edit' -and $args -contains '--draft=false') { $state.release.draft=$false }
$state | ConvertTo-Json -Depth 8 | Set-Content $statePath
'''

class PublishTests(unittest.TestCase):
    def setUp(self):
        temp = ROOT / 'target/tmp'
        temp.mkdir(parents=True, exist_ok=True)
        self.tmp = tempfile.TemporaryDirectory(dir=temp)
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        (self.root/'scripts').mkdir()
        shutil.copyfile(ROOT/'scripts/publish-release.ps1',self.root/'scripts/publish-release.ps1')
        (self.root/'Cargo.toml').write_text('[package]\nversion = "1.2.3"\n')
        self.kit=self.root/'dist/release/1.2.3'
        self.kit.mkdir(parents=True)
        names=['StrajaBackupSQL-1.2.3-windows-x64-setup.exe','StrajaBackupSQL-1.2.3-windows-x64-setup.exe.sha256','SQL-TOOLS-NOTICES.txt','THIRD-PARTY-NOTICES.txt','sources/mysql.tar.gz','sources/mariadb.tar.gz']
        files=[]
        for name in names:
            file=self.kit/name
            file.parent.mkdir(exist_ok=True)
            file.write_bytes(b'offline fixture, not executable')
            files.append(dict(Path=name,Bytes=file.stat().st_size,Sha256=hashlib.sha256(file.read_bytes()).hexdigest()))
        (self.kit/'release-manifest.json').write_text(json.dumps(dict(Product='StrajaBackupSQL',Version='1.2.3',Platform='windows-x64',SourceCommit='a'*40,Files=files)))
        (self.root/'notes.txt').write_text('Fixture release notes')
        (self.root/'gh.ps1').write_text(MOCK)
        self.set_state(release=None,badDigest=False,failUpload=False)

    def set_state(self, **values):
        (self.root/'state.json').write_text(json.dumps(values))

    def state(self):
        return json.loads((self.root/'state.json').read_text(encoding='utf-8-sig'))

    def calls(self):
        path=self.root/'calls.txt'
        return path.read_text() if path.exists() else ''

    def run_script(self,*args,ok=True):
        result=subprocess.run([shutil.which('pwsh') or 'powershell.exe','-NoProfile','-ExecutionPolicy','Bypass','-File',str(self.root/'scripts/publish-release.ps1'),'-NotesFile',str(self.root/'notes.txt'),'-GitHubCli',str(self.root/'gh.ps1'),*args],capture_output=True,text=True,timeout=30)
        self.assertEqual(result.returncode==0,ok,result.stdout+result.stderr)
        return result

    def test_dry_run_is_offline(self):
        self.run_script('-DryRun')
        self.assertEqual(self.calls(),'')

    def test_stable_publishes_only_after_upload_verification(self):
        self.run_script('-Channel','stable','-Publish')
        calls=self.calls()
        self.assertIn('--draft',calls)
        self.assertLess(calls.index('releases/tags/'),calls.index('release|edit'))
        self.assertIn('--prerelease=false|--latest|--draft=false',calls)
        self.assertFalse(self.state()['release']['draft'])

    def test_preview_and_draft_do_not_offer_stable_updates(self):
        self.run_script()
        self.assertTrue(self.state()['release']['draft'])
        self.run_script('-Publish')
        self.assertIn('--prerelease|--latest=false|--draft=false',self.calls())

    def test_public_version_cannot_be_overwritten(self):
        self.run_script('-Publish')
        before=self.calls().count('release|upload')
        self.run_script('-Publish',ok=False)
        self.assertEqual(self.calls().count('release|upload'),before)

    def test_bad_remote_digest_never_publishes(self):
        self.set_state(release=None,badDigest=True,failUpload=False)
        self.run_script('-Channel','stable','-Publish',ok=False)
        self.assertNotIn('release|edit',self.calls())
        self.assertTrue(self.state()['release']['draft'])

    def test_failed_upload_can_resume(self):
        self.set_state(release=None,badDigest=False,failUpload=True)
        self.run_script('-Publish',ok=False)
        state=self.state();state['failUpload']=False;self.set_state(**state)
        self.run_script('-Publish')
        self.assertEqual(self.calls().count('release|create'),1)

    def test_changed_local_installer_stops_before_network(self):
        (self.kit/'StrajaBackupSQL-1.2.3-windows-x64-setup.exe').write_text('tampered')
        self.run_script('-Publish',ok=False)
        self.assertEqual(self.calls(),'')

if __name__=='__main__':
    unittest.main()
