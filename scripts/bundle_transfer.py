"""Install the pinned transfer client app-locally; never alter PATH or other installations."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parent.parent

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--download', action='store_true')
    args = parser.parse_args()
    lock = json.loads((ROOT/'scripts/transfer-tool.lock.json').read_text())
    archive = ROOT/'.vendor-cache'/lock['url'].rsplit('/',1)[1]
    if not archive.exists():
        if not args.download:
            raise SystemExit('Transfer tool archive missing. Run bundle_transfer.py --download.')
        archive.parent.mkdir(exist_ok=True)
        urllib.request.urlretrieve(lock['url'],archive)
    with archive.open('rb') as stream:
        assert hashlib.file_digest(stream,'sha256').hexdigest() == lock['sha256'], 'Transfer tool checksum mismatch'
    target=ROOT/'vendor-tools/transfer'
    target.mkdir(parents=True,exist_ok=True)
    prefix=f"rclone-v{lock['version']}-windows-amd64/"
    with zipfile.ZipFile(archive) as bundle:
        for name in ('rclone.exe','README.txt'):
            (target/name).write_bytes(bundle.read(prefix+name))
    (target/'COPYING').write_bytes((ROOT/'docs/RCLONE-LICENSE.txt').read_bytes())
    manifest_path=ROOT/'vendor-tools/manifest.json'
    if manifest_path.exists():
        manifest=json.loads(manifest_path.read_text(encoding='utf-8'))
        manifest['transfer_tool']=lock
        for file in target.iterdir():
            with file.open('rb') as stream:
                manifest['files']['transfer/'+file.name]=hashlib.file_digest(stream,'sha256').hexdigest()
        manifest_path.write_text(json.dumps(manifest,indent=2),encoding='utf-8')
    output=subprocess.check_output([str(target/'rclone.exe'),'version'],creationflags=0x08000000,text=True)
    assert output.splitlines()[0] == 'rclone v'+lock['version']
    print(output.splitlines()[0]+' verified and staged app-locally')

if __name__ == '__main__':
    main()
