"""Launch only the staged Ubuntu app with an empty disposable profile under Xvfb."""
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time

root = Path(__file__).resolve().parent.parent
app = root / "target/native/payload/opt/strajabackupsql/StrajaBackupSQL"
output = root / "dist/native"
assert app.is_file() and os.environ.get("DISPLAY"), "Run the staged app under Xvfb"
with tempfile.TemporaryDirectory(prefix="straja-gui-") as profile:
    environment = {**os.environ, "HOME": profile, "XDG_DATA_HOME": profile + "/data"}
    with (output / "ubuntu-gui-smoke.log").open("w") as log:
        child = subprocess.Popen([app], env=environment, stdout=log, stderr=log, start_new_session=True)
        try:
            time.sleep(8)
            if child.poll() is not None:
                raise RuntimeError(f"GUI exited early: {child.returncode}; see ubuntu-gui-smoke.log")
            tree = subprocess.check_output(["xwininfo", "-root", "-tree"], text=True)
            if "Straja" not in tree:
                raise RuntimeError("The Straja window was not mapped")
            subprocess.run(["import", "-window", "root", str(output / "ubuntu-gui-smoke.png")], check=True)
        finally:
            # Only this test's isolated group (empty profile, no connections/plans).
            try:
                os.killpg(child.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            child.wait(timeout=10)
