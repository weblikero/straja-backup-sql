"""Native CI packaging. Never run against the user's database installations.

Linux uses extracted official Ubuntu client packages, not database server installs.
macOS copies clients from the disposable runner's Homebrew kegs and relocates dylibs.
Only client executables are shipped. Sources/notices accompany build artifacts.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import re
import shutil
import subprocess
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parent.parent
WORK = ROOT / "target/native"
OUT = ROOT / "dist/native"
SOURCES = ROOT / "dist/native-sources"
MAC = platform.system() == "Darwin"
ARCH = "arm64" if platform.machine() == "arm64" else "x64"
ID = "macos-" + ARCH if MAC else "linux-x64"
VERSION = re.search(r'^version = "([^"]+)"', (ROOT / "Cargo.toml").read_text(), re.M)[1]
PROVENANCE = []
BREW_SOURCES = set()
BREW_KEGS = set()
APT_SOURCES = set()

def run(*args, **kw):
    return subprocess.check_output([str(x) for x in args], text=True, **kw).strip()

def copy(source, target):
    target = Path(target)
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(Path(source).resolve(), target)
    target.chmod(0o755)
    return target

def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()

def download(url, path, expected=None):
    path = Path(path)
    if not path.exists():
        urllib.request.urlretrieve(url, path)
    if expected and sha(path) != expected:
        raise RuntimeError("Checksum mismatch: " + str(path))
    return path

def brew_source(path):
    parts = Path(path).resolve().parts
    if "Cellar" in parts:
        BREW_SOURCES.add(parts[parts.index("Cellar") + 1])
        BREW_KEGS.add(Path(*parts[:parts.index("Cellar") + 3]))

def macho(source, target, libraries, seen):
    source = Path(source).resolve()
    if source in seen:
        return seen[source]
    target = copy(source, target)
    seen[source] = target
    brew_source(source)
    lines = run("otool", "-L", source).splitlines()[1:]
    for line in lines:
        dependency = line.strip().split(" (", 1)[0]
        if dependency.startswith(("/usr/lib/", "/System/Library/")):
            continue
        if dependency.startswith("@"):
            # Brew bottles normally use absolute paths; resolve their occasional loader paths.
            candidate = dependency.replace("@loader_path", str(source.parent))
            if dependency.startswith("@rpath/"):
                candidates = [source.parent / dependency[7:], source.parent.parent / "lib" / dependency[7:]]
                candidate = next((str(p) for p in candidates if p.exists()), candidate)
            dependency_path = Path(candidate)
        else:
            dependency_path = Path(dependency)
        if not dependency_path.exists():
            raise RuntimeError(f"Unresolved dylib {dependency} in {source}")
        if dependency_path.resolve() == source:
            run("install_name_tool", "-id", "@loader_path/" + target.name, target)
            continue
        dest = macho(dependency_path, libraries / dependency_path.name, libraries, seen)
        relative = os.path.relpath(dest, target.parent)
        run("install_name_tool", "-change", dependency, "@loader_path/" + relative, target)
    # Relocation invalidates upstream signatures. Ad-hoc sign for local execution only.
    run("codesign", "--force", "--sign", "-", target, stderr=subprocess.STDOUT)
    return target

def elf(source, target, libraries, seen):
    source = Path(source).resolve()
    if source in seen:
        return seen[source]
    target = copy(source, target)
    seen[source] = target
    try:
        package = run("dpkg-query", "-S", source).splitlines()[0].split(": ")[0].split(":")[0]
        source_name = run("dpkg-query", "-W", "-f=${source:Package}", package)
        source_version = run("dpkg-query", "-W", "-f=${source:Version}", package)
        APT_SOURCES.add((source_name, source_version))
    except subprocess.CalledProcessError:
        pass  # Extracted client packages are recorded separately.
    for line in run("ldd", source).splitlines():
        if "not found" in line:
            raise RuntimeError(f"Missing dependency for {source}: {line}")
        match = re.search(r"=> (/\S+)", line)
        if not match:
            continue
        dependency = Path(match[1])
        if re.match(r"lib(c|m|pthread|dl|rt|resolv)\.so", dependency.name):
            continue  # Use the target Ubuntu glibc and loader.
        elf(dependency, libraries / dependency.name, libraries, seen)
    run("patchelf", "--set-rpath", "$ORIGIN" if target.parent == libraries else "$ORIGIN/../lib", target)
    return target

def clients(app):
    versions = {}
    linux_packages = {
        "mariadb": ["mariadb-client-core", "mariadb-client", "mariadb-common", "libmariadb3"],
        "mysql": ["mysql-client-core-8.0", "mysql-client-8.0", "mysql-common"],
        "postgresql": ["postgresql-client-18"],
    }
    for family, names, formula in [
        ("mariadb", ["mariadb", "mariadb-dump"], "mariadb@11.4"),
        ("mysql", ["mysql", "mysqldump"], "mysql-client@8.4"),
        ("postgresql", ["psql", "pg_dump"], "libpq"),
    ]:
        target = app / "tools" / family
        (target / "lib/plugin").mkdir(parents=True)
        (target / "share/charsets").mkdir(parents=True)
        seen = {}
        if MAC:
            prefix = Path(run("brew", "--prefix", formula))
            BREW_SOURCES.add(formula)
            for name in names:
                macho(prefix / "bin" / name, target / "bin" / name, target / "lib", seen)
            plugin_roots = [prefix / "lib/plugin", prefix / "lib/mariadb/plugin"]
            charset_roots = [prefix / "share/mysql/charsets", prefix / "share/mariadb/charsets"]
        else:
            folder = WORK / family
            folder.mkdir()
            run("apt-get", "download", *linux_packages[family], cwd=folder)
            prefix = folder / "extract"
            for archive in sorted(folder.glob("*.deb")):
                fields = json.loads(run("python3", "-c",
                    "import json,subprocess,sys;print(json.dumps({x:subprocess.check_output(['dpkg-deb','-f',sys.argv[1],x],text=True).strip() for x in ['Package','Version','Source']}))", archive))
                source = fields["Source"] or fields["Package"]
                source_match = re.fullmatch(r"(\S+)(?: \(([^)]+)\))?", source)
                APT_SOURCES.add((source_match[1], source_match[2] or fields["Version"]))
                PROVENANCE.append({**fields, "archive": archive.name, "sha256": sha(archive)})
                run("dpkg-deb", "-x", archive, prefix)
            binary_dir = prefix / ("usr/lib/postgresql/18/bin" if family == "postgresql" else "usr/bin")
            for name in names:
                elf(binary_dir / name, target / "bin" / name, target / "lib", seen)
            plugin_roots = list(prefix.glob("usr/lib/**/plugin"))
            charset_roots = list(prefix.glob("usr/share/**/charsets"))
            # Carry upstream notices from the packages without any global configuration files.
            if (prefix / "usr/share/doc").exists():
                shutil.copytree(prefix / "usr/share/doc", target / "notices", dirs_exist_ok=True)
        client_plugins = ("client_", "dialog", "mysql_clear_password", "auth_gssapi_client",
                          "caching_sha2_password", "sha256_password", "authentication_")
        for directory in plugin_roots:
            if directory.is_dir():
                for plugin in directory.iterdir():
                    if plugin.is_file() and plugin.name.startswith(client_plugins) and plugin.suffix in (".so", ".dylib"):
                        # Relative lib/plugin => libraries are one directory above.
                        dest = (macho if MAC else elf)(plugin, target / "lib/plugin" / plugin.name, target / "lib", seen)
                        if not MAC:
                            run("patchelf", "--set-rpath", "$ORIGIN/..", dest)
        for directory in charset_roots:
            if directory.is_dir():
                shutil.copytree(directory, target / "share/charsets", dirs_exist_ok=True)
                break
        versions[family] = run(target / "bin" / names[0], "--version").splitlines()[0]
    (app / "tools/versions.json").write_text(json.dumps(versions, indent=2) + "\n")

def transfer(app):
    suffix = "osx-" + ("arm64" if ARCH == "arm64" else "amd64") if MAC else "linux-amd64"
    hashes = {
        "linux-amd64": "349ac8fba6ff65d6247043f1750cdcb518ec5d500ef91463a10d37c0ccdf3702",
        "osx-amd64": "574e1f31cf1c464e5fb0325cb66fcbdeaf1efa1802bd9b17f13c256fd74b6561",
        "osx-arm64": "610ab7d48615135dfc60b763858b11a426a0d3d1160c1bb76fb0ae6011b02edd",
    }
    name = f"rclone-v1.75.2-{suffix}.zip"
    archive = download("https://github.com/rclone/rclone/releases/download/v1.75.2/" + name, WORK / name, hashes[suffix])
    target = app / "tools/transfer"
    target.mkdir()
    with zipfile.ZipFile(archive) as bundle:
        for filename in ("rclone", "README.txt"):
            (target / filename).write_bytes(bundle.read(name[:-4] + "/" + filename))
    (target / "rclone").chmod(0o755)
    shutil.copy2(ROOT / "docs/RCLONE-LICENSE.txt", target / "COPYING")
    PROVENANCE.append({"tool": name, "sha256": hashes[suffix]})

def sources():
    if MAC:
        for formula in sorted(BREW_SOURCES):
            info = json.loads(run("brew", "info", "--json=v2", formula))["formulae"][0]
            SOURCES.joinpath(formula.replace("/", "_") + ".json").write_text(json.dumps(info, indent=2))
            source = info["urls"]["stable"]
            path = SOURCES / (formula.replace("/", "_") + "-" + source["url"].rsplit("/", 1)[1])
            download(source["url"], path, source["checksum"])
            # Preserve the formula/patch references and build metadata too.
            SOURCES.joinpath(formula.replace("/", "_") + ".rb").write_text(run("brew", "cat", formula))
    else:
        for name, version in sorted(APT_SOURCES):
            run("apt-get", "source", "--download-only", "--only-source", name + "=" + version, cwd=SOURCES)
    (SOURCES / "SHA256SUMS.txt").write_text("".join(f"{sha(p)}  {p.name}\n" for p in sorted(SOURCES.iterdir()) if p.is_file() and p.name != "SHA256SUMS.txt"))

def script(path, content):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content)
    path.chmod(0o755)

def process_guard():
    pattern = (r"^/Applications/Straja Backup SQL[.]app/Contents/MacOS/StrajaBackupSQL( |$|-Agent)"
               if MAC else r"^/opt/strajabackupsql/StrajaBackupSQL( |$|-Agent)")
    return f"""#!/bin/sh
set -eu
if /usr/bin/pgrep -f '{pattern}' >/dev/null; then
    echo 'Straja Backup SQL is running. Wait for backups to finish, stop its agent in Settings and close its window, then retry.' >&2
    exit 1
fi
"""

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--sources", action="store_true")
    args = parser.parse_args()
    assert platform.system() in ("Linux", "Darwin"), "Run on a native disposable build runner"
    if WORK.exists():
        raise RuntimeError("Use a clean native staging directory: " + str(WORK))
    WORK.mkdir(parents=True)
    OUT.mkdir(parents=True, exist_ok=True)
    SOURCES.mkdir(parents=True, exist_ok=True)
    payload = WORK / "payload"
    bundle = payload / "Applications/Straja Backup SQL.app"
    app = bundle / "Contents/MacOS" if MAC else payload / "opt/strajabackupsql"
    app.mkdir(parents=True)
    for binary in ("StrajaBackupSQL", "StrajaBackupSQL-Agent"):
        copy(ROOT / "target/release" / binary, app / binary)
    (app / "default-language.txt").write_text("en\n")
    shutil.copytree(ROOT / "docs", app / "docs")
    shutil.copy2(ROOT / "THIRD-PARTY-NOTICES.txt", app)
    clients(app)
    if MAC:
        for keg in sorted(BREW_KEGS):
            notices = app / "tools/notices" / keg.parent.name
            notices.mkdir(parents=True, exist_ok=True)
            for name in ("INSTALL_RECEIPT.json", "LICENSE", "LICENSE.txt", "COPYING", "COPYING.txt", "COPYRIGHT"):
                if (keg / name).is_file():
                    shutil.copy2(keg / name, notices / name)
            if (keg / ".brew").is_dir():
                shutil.copytree(keg / ".brew", notices / "brew", dirs_exist_ok=True)
    transfer(app)
    (app / "tools/provenance.json").write_text(json.dumps(PROVENANCE, indent=2))
    # Move the staged directory to prove client library relocation before packaging.
    moved = app.with_name(app.name + "-relocated")
    app.rename(moved)
    try:
        print(run(moved / "StrajaBackupSQL", "--self-check"))
    finally:
        moved.rename(app)
    name = f"StrajaBackupSQL-{VERSION}-{ID}"
    if MAC:
        resources = bundle / "Contents/Resources"
        resources.mkdir()
        iconset = WORK / "AppIcon.iconset"
        iconset.mkdir()
        for size in (16, 32, 128, 256, 512):
            for scale in (1, 2):
                run("sips", "-z", str(size*scale), str(size*scale), ROOT / "installer/assets/logo.png",
                    "--out", iconset / f"icon_{size}x{size}{'@2x' if scale==2 else ''}.png")
        run("iconutil", "-c", "icns", iconset, "-o", resources / "AppIcon.icns")
        info = {"CFBundleName":"Straja Backup SQL", "CFBundleDisplayName":"Straja Backup SQL",
                "CFBundleIdentifier":"com.strajabackup.sql", "CFBundleExecutable":"StrajaBackupSQL",
                "CFBundleVersion":VERSION, "CFBundleShortVersionString":VERSION,
                "CFBundlePackageType":"APPL", "CFBundleIconFile":"AppIcon", "LSMinimumSystemVersion":"15.0",
                "NSHighResolutionCapable":True}
        (bundle / "Contents/Info.plist").write_bytes(plistlib.dumps(info))
        agents = payload / "Library/LaunchAgents"
        agents.mkdir(parents=True)
        (agents / "com.strajabackup.sql.agent.plist").write_bytes(plistlib.dumps({
            "Label":"com.strajabackup.sql.agent", "ProgramArguments":["/Applications/Straja Backup SQL.app/Contents/MacOS/StrajaBackupSQL-Agent"],
            "RunAtLoad":True, "LimitLoadToSessionType":"Aqua", "ProcessType":"Background",
        }))
        run("codesign", "--force", "--deep", "--sign", "-", bundle, stderr=subprocess.STDOUT)
        run("codesign", "--verify", "--deep", "--strict", bundle, stderr=subprocess.STDOUT)
        artifact = OUT / (name + ".pkg")
        scripts = WORK / "scripts"
        script(scripts / "preinstall", process_guard())
        run("pkgbuild", "--root", payload, "--identifier", "com.strajabackup.sql",
            "--version", VERSION, "--scripts", scripts, "--install-location", "/", artifact)
    else:
        desktop = payload / "usr/share/applications/strajabackupsql.desktop"
        desktop.parent.mkdir(parents=True)
        desktop.write_text("[Desktop Entry]\nType=Application\nName=Straja Backup SQL\nExec=/opt/strajabackupsql/StrajaBackupSQL\nIcon=strajabackupsql\nCategories=Utility;Database;\nTerminal=false\n")
        icon = payload / "usr/share/icons/hicolor/128x128/apps/strajabackupsql.png"
        icon.parent.mkdir(parents=True)
        shutil.copy2(ROOT / "installer/assets/logo.png", icon)
        autostart = payload / "etc/xdg/autostart/strajabackupsql-agent.desktop"
        autostart.parent.mkdir(parents=True)
        autostart.write_text("[Desktop Entry]\nType=Application\nName=Straja Backup SQL Agent\nExec=/opt/strajabackupsql/StrajaBackupSQL-Agent\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n")
        control = payload / "DEBIAN"
        control.mkdir()
        (control / "control").write_text(f"Package: strajabackupsql\nVersion: {VERSION}\nArchitecture: amd64\nMaintainer: Straja Backup\nSection: utils\nPriority: optional\nDepends: libc6 (>= 2.39), libdbus-1-3, libxkbcommon0, libfontconfig1, libgtk-3-0t64, gnome-keyring, gdebi\nDescription: SQL backup manager for MariaDB, MySQL and PostgreSQL\n Bundled private clients, scheduled verified backups and remote destinations.\n")
        script(control / "preinst", process_guard())
        script(control / "prerm", process_guard())
        artifact = OUT / (name + ".deb")
        run("dpkg-deb", "--root-owner-group", "--build", payload, artifact)
    (OUT / (artifact.name + ".sha256")).write_text(f"{sha(artifact)}  {artifact.name}\n")
    (OUT / (name + "-tools.json")).write_text(json.dumps({"version":VERSION,"platform":ID,
        "files":{str(p.relative_to(app)):sha(p) for p in app.rglob("*") if p.is_file()}}, indent=2))
    if args.sources:
        sources()
    print(artifact)

if __name__ == "__main__":
    main()
