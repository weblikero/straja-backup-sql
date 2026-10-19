"""Build app-private SQL clients from hash-pinned official archives. No installers/servers run.

Python 3.12+ standard library only. Downloads are an explicit build-time step.
Only two EXEs per engine, selected client plugins, their DLL dependency closure,
charset resources and upstream notices are copied. Never reads installed SQL products.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parent.parent
LOCK = json.loads((ROOT / "scripts/sql-tools.lock.json").read_text(encoding="utf-8"))
CACHE = ROOT / ".vendor-cache"
DEST = ROOT / "vendor-tools"
# OS components on the supported Windows 10/11 x64 platform. Runtime libraries
# (MSVCP/VCRUNTIME) deliberately excluded: those MUST be shipped app-local.
SYSTEM = set("""kernel32.dll kernelbase.dll ntdll.dll advapi32.dll user32.dll gdi32.dll
ws2_32.dll secur32.dll crypt32.dll bcrypt.dll bcryptprimitives.dll ncrypt.dll
shell32.dll shlwapi.dll ole32.dll oleaut32.dll rpcrt4.dll comdlg32.dll comctl32.dll
version.dll winmm.dll wldap32.dll dnsapi.dll iphlpapi.dll netapi32.dll userenv.dll
normaliz.dll msvcrt.dll ucrtbase.dll dbghelp.dll psapi.dll wintrust.dll powrprof.dll
setupapi.dll cfgmgr32.dll cryptbase.dll authz.dll credui.dll winhttp.dll
sspicli.dll cryptui.dll dhcpcsvc.dll winspool.drv wsock32.dll hid.dll
dwmapi.dll imm32.dll uxtheme.dll""".split())


def digest(path):
    with path.open("rb") as f:
        return hashlib.file_digest(f, "sha256").hexdigest()


def imports(data):
    """Read regular and delay-load DLL imports from an x64 PE image."""
    def u16(at): return struct.unpack_from("<H", data, at)[0]
    def u32(at): return struct.unpack_from("<I", data, at)[0]
    pe = u32(0x3C)
    if data[pe:pe+4] != b"PE\0\0" or u16(pe+4) != 0x8664:
        raise ValueError("Expected an x64 PE image")
    opt = pe + 24
    if u16(opt) != 0x20B:
        raise ValueError("Expected PE32+")
    sections = opt + u16(pe+20)
    def offset(rva):
        for i in range(u16(pe+6)):
            s = sections + i*40
            size, va, rawsize, raw = struct.unpack_from("<IIII", data, s+8)
            if va <= rva < va + max(size, rawsize):
                return raw + rva - va
        raise ValueError(f"Invalid RVA {rva:x}")
    found = set()
    for index, stride, name_index in [(1, 20, 12), (13, 32, 4)]:
        rva, size = struct.unpack_from("<II", data, opt+112+index*8)
        if not rva:
            continue
        start = offset(rva)
        for pos in range(start, start+size, stride):
            if not any(data[pos:pos+stride]):
                break
            if index == 13 and not u32(pos) & 1:
                raise ValueError("Unsupported VA-based delay import")
            n = offset(u32(pos+name_index))
            found.add(data[n:data.index(b"\0", n)].decode("ascii").lower())
    return sorted(found)


def system_dll(name):
    return name in SYSTEM or name.startswith(("api-ms-win-", "ext-ms-win-"))


def download(name, item, allow):
    path = CACHE / (name + ".zip")
    if not path.is_file():
        if not allow:
            raise RuntimeError(f"Missing {path}; run with --download first")
        partial = path.with_suffix(".partial")
        print(f"Downloading {item['url']}", flush=True)
        with urllib.request.urlopen(item["url"], timeout=90) as response, partial.open("wb") as out:
            shutil.copyfileobj(response, out)
        if digest(partial) != item["sha256"]:
            raise RuntimeError(f"Checksum mismatch: {name}")
        partial.replace(path)
    if digest(path) != item["sha256"]:
        raise RuntimeError(f"Checksum mismatch: {name}")
    return zipfile.ZipFile(path)


def bundle(allow_download):
    CACHE.mkdir(exist_ok=True)
    archives = {n: download(n, spec, allow_download) for n, spec in LOCK.items()}
    # The pinned MariaDB distribution supplies the Microsoft 14.44 x64 CRT files.
    runtime = {}
    maria = archives["mariadb"]
    for n in maria.namelist():
        if n.startswith(LOCK["mariadb"]["prefix"] + "bin/") and Path(n).name.lower().startswith(("vcruntime140", "msvcp140", "concrt140")) and n.endswith(".dll"):
            runtime[Path(n).name.lower()] = maria.read(n)
    if not runtime:
        raise RuntimeError("App-local Visual C++ runtime not found")
    # Clear only this tool's own generated output; never a supplied path.
    assert DEST.parent == ROOT and DEST.name == "vendor-tools"
    if DEST.exists():
        if DEST.is_symlink() or DEST.is_junction():
            raise RuntimeError("Refusing linked staging directory")
        shutil.rmtree(DEST)
    DEST.mkdir()
    for name, spec in LOCK.items():
        z = archives[name]
        prefix = spec["prefix"]
        files = {p[len(prefix):]: p for p in z.namelist() if p.startswith(prefix) and not p.endswith("/")}
        target = DEST / name
        selected = {}
        def add(relative):
            if relative not in selected:
                selected[relative] = z.read(files[relative])
        for exe in spec["executables"]:
            add("bin/" + exe)
        for plugin in spec["plugins"]:
            add("lib/plugin/" + plugin)
        for p in files:
            if p.startswith("share/charsets/") or (name == "mysql" and p.startswith("bin/sasl2/") and p.endswith(".dll")):
                add(p)
        for notice in spec["licenses"]:
            add(notice)
        for n, data in runtime.items():
            selected["bin/" + n] = data
        candidates = {Path(p).name.lower(): p for p in files if p.startswith("bin/") and p.count("/") == 1 and p.endswith(".dll")}
        checked = set()
        while True:
            pending = [p for p in selected if p.endswith((".exe", ".dll")) and p not in checked]
            if not pending:
                break
            for p in pending:
                checked.add(p)
                for dep in imports(selected[p]):
                    if system_dll(dep) or "bin/" + dep in selected:
                        continue
                    if dep not in candidates:
                        raise RuntimeError(f"Unresolved dependency: {name}/{p} -> {dep}")
                    add(candidates[dep])
        for p, data in selected.items():
            out = target / p
            if not out.resolve().is_relative_to(target.resolve()):
                raise RuntimeError("Invalid archive path")
            out.parent.mkdir(parents=True, exist_ok=True)
            out.write_bytes(data)
        print(f"{name}: {len(selected)} files, {sum(map(len, selected.values()))/1024**2:.1f} MiB", flush=True)
    manifest = {"archives": LOCK, "runtime_source": "Unmodified Microsoft CRT 14.44.35211.0 from pinned MariaDB archive", "files": {str(p.relative_to(DEST)).replace('\\', '/'): digest(p) for p in sorted(DEST.rglob('*')) if p.is_file()}}
    (DEST / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")


def sources(allow_download):
    directory = ROOT / 'dist/sources'
    directory.mkdir(parents=True, exist_ok=True)
    hashes = []
    for item in LOCK.values():
        if 'source_sha256' not in item:
            continue
        path = directory / item['source_url'].rsplit('/', 1)[1]
        if not path.is_file():
            if not allow_download:
                raise RuntimeError(f'Missing source archive: {path}; use --download --sources')
            partial = path.with_suffix('.partial')
            with urllib.request.urlopen(item['source_url'], timeout=90) as response, partial.open('wb') as out:
                shutil.copyfileobj(response, out)
            if digest(partial) != item['source_sha256']:
                raise RuntimeError('Source checksum mismatch')
            partial.replace(path)
        if digest(path) != item['source_sha256']:
            raise RuntimeError(f'Source checksum mismatch: {path}')
        hashes.append(f"{item['source_sha256']}  {path.name}")
    (directory / 'SHA256SUMS.txt').write_text('\n'.join(hashes) + '\n', encoding='ascii')
    shutil.copyfile(ROOT / 'SQL-TOOLS-NOTICES.txt', directory / 'README.txt')


def verify(root, app_dir=None):
    """No server/network access: validate hashes, imports and --version only."""
    manifest = json.loads((root / "manifest.json").read_text(encoding="utf-8"))
    expected = manifest["files"]
    actual = {p.relative_to(root).as_posix() for p in root.rglob('*') if p.is_file() and p != root / 'manifest.json'}
    if actual != set(expected):
        raise RuntimeError(f"Unexpected or missing tools: {actual ^ set(expected)}")
    for relative, sha in expected.items():
        p = root / relative
        if digest(p) != sha:
            raise RuntimeError(f"Modified tool: {relative}")
        if p.suffix.lower() in ['.exe', '.dll']:
            for dep in imports(p.read_bytes()):
                if not system_dll(dep) and not (root / relative.split('/')[0] / 'bin' / dep).is_file():
                    raise RuntimeError(f"Missing app-local dependency: {relative} -> {dep}")
    if app_dir:
        for exe in ['StrajaBackupSQL.exe', 'StrajaBackupSQL-Agent.exe']:
            for dep in imports((app_dir / exe).read_bytes()):
                if not system_dll(dep):
                    library = app_dir / dep
                    if not library.is_file() or digest(library) != expected.get('mariadb/bin/' + dep):
                        raise RuntimeError(f'Missing/incorrect application runtime: {exe} -> {dep}')
    # No user profile SQL config or PATH dependency is used. --no-defaults is first.
    env = {k: v for k, v in os.environ.items() if k.upper() in ['SYSTEMROOT', 'WINDIR', 'TEMP', 'TMP']}
    env['PATH'] = str(Path(os.environ['SYSTEMROOT']) / 'System32')
    report = []
    for name, spec in manifest['archives'].items():
        for exe in spec['executables']:
            program = (root / name / 'bin' / exe).resolve()
            args = [str(program)]
            if name != 'postgresql':
                args += ['--no-defaults']
                if name == 'mysql': args += ['--no-login-paths']
                args += ['--plugin-dir=' + str(program.parent.parent / 'lib/plugin'), '--character-sets-dir=' + str(program.parent.parent / 'share/charsets')]
            args += ['--version']
            result = subprocess.run(args, cwd=program.parent, env=env, capture_output=True, text=True, timeout=20, creationflags=0x08000000)
            output = (result.stdout + result.stderr).strip()
            if result.returncode != 0 or spec['version'] not in output:
                raise RuntimeError(f"Client smoke test failed: {exe}: {result.returncode} {output}")
            report.append({'exe': str(program), 'version': output, 'exit_code': result.returncode})
            print(output, flush=True)
    output = ROOT / 'test-output/bundled-tools.json'
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps({'file_count': len(expected), 'path': env['PATH'], 'checks': report}, indent=2), encoding='utf-8')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--download', action='store_true')
    parser.add_argument('--verify-only', type=Path)
    parser.add_argument('--app-dir', type=Path)
    parser.add_argument('--sources', action='store_true')
    options = parser.parse_args()
    if options.verify_only:
        verify(options.verify_only.resolve(), options.app_dir)
    else:
        bundle(options.download)
        verify(DEST)
    if options.sources:
        sources(options.download)
