"""Collect successful native CI artifacts and prepare a verified, unpublished draft."""
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time
import zipfile

REPO = "weblikero/straja-backup-sql"
PLATFORMS = ("linux-x64", "macos-arm64", "macos-x64")
ROOT = Path(__file__).resolve().parent.parent
WORK = ROOT / "target/native-release"
ASSETS = WORK / "assets"


def gh(*args):
    return subprocess.check_output(["gh", *args], text=True).strip()


def api(path):
    return json.loads(gh("api", f"repos/{REPO}/{path}"))


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def release_for(tag):
    releases = api("releases?per_page=100")
    matching = [release for release in releases if release["tag_name"] == tag]
    require(len(matching) <= 1, "Ambiguous release tag")
    return matching[0] if matching else None


def source_archive(directory, destination):
    expected = {}
    for line in (directory / "SHA256SUMS.txt").read_text().splitlines():
        checksum, name = line.split("  ", 1)
        require(re.fullmatch(r"[a-f0-9]{64}", checksum), "Invalid source checksum")
        require(Path(name).name == name and "/" not in name and "\\" not in name,
                "Source name escapes its artifact")
        require(name not in expected, "Duplicate source name")
        expected[name] = checksum
    actual = {p.name for p in directory.iterdir() if p.name != "SHA256SUMS.txt"}
    require(actual == set(expected) and len(actual) >= 2, "Incomplete source artifact")
    for name, checksum in expected.items():
        path = directory / name
        require(path.is_file() and not path.is_symlink() and digest(path) == checksum,
                f"Source integrity mismatch: {name}")
    # Upstream archives are already compressed; avoid wasting CPU recompressing them.
    with zipfile.ZipFile(destination, "w", zipfile.ZIP_STORED, allowZip64=True) as archive:
        for path in sorted(directory.iterdir()):
            archive.write(path, path.name)


def main():
    run_id = os.environ["NATIVE_BUILD_RUN"]
    require(re.fullmatch(r"[1-9][0-9]*", run_id), "Invalid build run ID")
    build = api(f"actions/runs/{run_id}")
    require(build["conclusion"] == "success", "Every platform must pass before preparing a release")
    require(build["path"] == ".github/workflows/native-kits.yml"
            and build["head_repository"]["full_name"] == REPO
            and build["head_branch"] == "main", "Unexpected native build origin")
    commit = build["head_sha"]
    require(re.fullmatch(r"[a-f0-9]{40}", commit), "Invalid source revision")
    require(not WORK.exists(), "Use a fresh release staging directory")
    ASSETS.mkdir(parents=True)
    version = None
    for platform in PLATFORMS:
        directory = WORK / platform
        source_dir = WORK / (platform + "-sources")
        for name, target in [(f"StrajaBackupSQL-{platform}", directory),
                             (f"StrajaBackupSQL-{platform}-sources", source_dir)]:
            gh("run", "download", run_id, "--repo", REPO, "--name", name, "--dir", str(target))
        reports = list(directory.glob("*-build.json"))
        require(len(reports) == 1, f"Missing build report for {platform}")
        report = json.loads(reports[0].read_text())
        candidate = report["version"]
        require(re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", candidate),
                "Invalid package version")
        version = version or candidate
        require(candidate == version and report["platform"] == platform
                and report["commit"] == commit, "Native kits do not share one version and revision")
        prefix = f"StrajaBackupSQL-{version}-{platform}"
        name = prefix + (".deb" if platform == "linux-x64" else ".pkg")
        installer = directory / name
        require(report["installer"] == name and installer.is_file()
                and not installer.is_symlink(), "Unexpected installer")
        checksum = digest(installer)
        require(report["sha256"] == checksum, "Installer differs from its build report")
        require((directory / (name + ".sha256")).read_text().strip() == f"{checksum}  {name}",
                "Installer checksum file mismatch")
        for file in (name, name + ".sha256", prefix + "-build.json", prefix + "-tools.json"):
            shutil.copy2(directory / file, ASSETS / file)
        source_archive(source_dir, ASSETS / (prefix + "-sources.zip"))
        if platform == "linux-x64":
            require((directory / "ubuntu-gui-smoke.png").is_file(), "Missing Ubuntu GUI verification")
            shutil.copy2(directory / "ubuntu-gui-smoke.png", ASSETS / f"StrajaBackupSQL-{version}-ubuntu-preview.png")
    shutil.copy2(ROOT / "docs/NATIVE-KITS.en.txt", ASSETS)
    inventory = [{"name": p.name, "bytes": p.stat().st_size, "sha256": digest(p)}
                 for p in sorted(ASSETS.iterdir())]
    (ASSETS / "native-release-manifest.json").write_text(json.dumps({
        "product": "StrajaBackupSQL", "version": version, "source_commit": commit,
        "channel": "preview", "platforms": PLATFORMS, "build_run": build["html_url"],
        "files": inventory, "ready_for_stable": False,
    }, indent=2))
    notes = WORK / "notes.txt"
    notes.write_text(f"""Straja Backup SQL {version} — native preview

Installers for Ubuntu 24.04+ x64 and macOS 15+ Apple Silicon / Intel.
MariaDB, MySQL, PostgreSQL clients and rclone are bundled privately. Existing
SQL installations and global PATH are not modified. English plus German,
Spanish, Portuguese, French and Italian are available in the application.

Native build, backup pipeline, credential-store, update-download, retention and
UI-state tests passed. Ubuntu's graphical window was also launched under Xvfb.
Build and validation: {build["html_url"]}
Exact source revision: {commit}

Preview limits: native agents are controlled from Settings and start at graphical
login; this preview has no native tray icon. Keep the computer awake and the
credential store unlocked. Updates download in the app and finish in the OS
installer; reopen the app afterwards. Clean-machine acceptance testing is still
required. Mac packages are not Developer ID signed or notarized.

See NATIVE-KITS.en.txt for installation/removal. SHA-256 files and corresponding
bundled-tool sources accompany each platform. This draft does not change Latest
and is not advertised to installed applications on the stable update channel.
""")
    tag = "v" + version
    draft = release_for(tag)
    if draft is None:
        gh("release", "create", tag, "--repo", REPO, "--target", commit,
           "--title", f"StrajaBackupSQL {version} — Ubuntu and macOS preview",
           "--notes-file", str(notes), "--draft", "--prerelease")
        for _ in range(10):
            draft = release_for(tag)
            if draft:
                break
            time.sleep(2)
    require(draft and draft["draft"] and draft["prerelease"] and draft["target_commitish"] == commit,
            "Existing release is public or belongs to another source revision")
    assets = sorted(ASSETS.iterdir())
    desired = {p.name: (p.stat().st_size, "sha256:" + digest(p)) for p in assets}
    require(all(asset["name"] in desired for asset in draft["assets"]), "Draft contains unexpected assets")
    for path in assets:
        current = api(f"releases/{draft['id']}")
        require(current["draft"] and current["target_commitish"] == commit,
                "Release changed during upload; stopping")
        existing = [a for a in current["assets"] if a["name"] == path.name]
        if len(existing) == 1 and (existing[0]["size"], existing[0].get("digest")) == desired[path.name]:
            continue
        print(f"Uploading {path.name}", flush=True)
        gh("release", "upload", tag, str(path), "--repo", REPO, "--clobber")
    remote = {}
    for _ in range(10):
        verified = api(f"releases/{draft['id']}")
        require(verified["draft"] and verified["prerelease"] and verified["target_commitish"] == commit,
                "Release must stay a preview draft of the tested source")
        remote = {a["name"]: (a["size"], a.get("digest")) for a in verified["assets"] if a["state"] == "uploaded"}
        if remote == desired:
            break
        time.sleep(2)
    require(remote == desired, "Uploaded release assets differ from the verified kit")
    message = f"Verified native draft: {verified['html_url']}\nSource: {commit}\nAssets: {len(assets)}\n"
    print(message)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as summary:
            summary.write(message)


if __name__ == "__main__":
    main()
