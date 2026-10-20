"""Give Intel test executables the same ad-hoc identity as the packaged Mac app."""
import json
import os
import subprocess

arguments = ["cargo", "test", "--locked", "--lib", "--bin", "StrajaBackupSQL",
             "--test", "core", "--test", "mail", "--test", "backup_pipeline"]
result = subprocess.run(arguments + ["--no-run", "--message-format=json"], stdout=subprocess.PIPE, text=True, check=True)
executables = set()
for line in result.stdout.splitlines():
    item = json.loads(line)
    if item.get("executable"):
        executables.add(item["executable"])
for path in sorted(executables):
    subprocess.run(["codesign", "--force", "--sign", "-", path], check=True)
# A cold compile may outlast the keychain's idle timer; unlock the fixture again.
keychain = os.path.join(os.environ["RUNNER_TEMP"], "straja-test.keychain-db")
subprocess.run(["security", "unlock-keychain", "-p", "fixture", keychain], check=True)
subprocess.run(["security", "default-keychain", "-d", "user"], check=True)
# Keychain access must work without a GUI prompt in this disposable test keychain.
subprocess.run(arguments, check=True, timeout=120)
