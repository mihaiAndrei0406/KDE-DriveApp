"""Explicit live 5b acceptance. Writes only to the fixed disposable repository."""
import os
from pathlib import Path
import subprocess
import tempfile
import time

from PySide6.QtDBus import QDBusConnection, QDBusMessage

from hetzner_drive.client import BUS, OBJECT
from hetzner_drive.projects import ProjectRegistry


ROOT = Path(__file__).resolve().parents[1]
fixture = tempfile.TemporaryDirectory(prefix="hetzner-drive-live-backup-")
fixture_root = Path(fixture.name)
config_home = fixture_root / "config"
state_home = fixture_root / "state"
source = fixture_root / "source-project"
source.mkdir(mode=0o700)
expected = b"Hetzner Drive phase 5b disposable restore fixture\n"
(source / "document.txt").write_bytes(expected)
os.chmod(source / "document.txt", 0o600)
os.environ["XDG_CONFIG_HOME"] = str(config_home)
os.environ["XDG_STATE_HOME"] = str(state_home)
project = ProjectRegistry().add("Test disposable 5b", str(source), 120)

bus = QDBusConnection.sessionBus()


def call(method, arguments=None, timeout=220_000):
    message = QDBusMessage.createMethodCall(BUS, OBJECT, BUS, method)
    if arguments:
        message.setArguments(arguments)
    reply = bus.call(message, timeout=timeout)
    if reply.type() != QDBusMessage.MessageType.ReplyMessage:
        raise RuntimeError(f"{method}: {reply.errorName()}: {reply.errorMessage()}")
    return reply.arguments()


def require_success(method, arguments=None, timeout=220_000):
    result = call(method, arguments, timeout)
    if result != [True, "ok"] and result != [True, "accepted"]:
        raise RuntimeError(f"{method} refuzat: {result}")


def wait_job(expected_phase):
    deadline = time.monotonic() + 180
    while time.monotonic() < deadline:
        status = call("GetBackupStatus", timeout=10_000)
        if not status[3]:
            if status[4] != expected_phase:
                raise RuntimeError(f"Job restic esuat: phase={status[4]}, error={status[12]}")
            return status
        time.sleep(0.25)
    raise RuntimeError("Jobul restic nu s-a terminat in 180 de secunde")


core = subprocess.Popen(
    [str(ROOT / "target/debug/hetzner-drive-core")],
    stdout=subprocess.DEVNULL,
    stderr=subprocess.PIPE,
    env=os.environ.copy(),
)
restore_path = None
try:
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if core.poll() is not None:
            raise RuntimeError("Controllerul live s-a oprit inainte de test")
        if bus.interface().isServiceRegistered(BUS).value():
            break
        time.sleep(0.05)
    else:
        raise RuntimeError("Controllerul live nu a inregistrat serviciul D-Bus")

    require_success("UnlockConfiguration")
    require_success("InitializeBackupRepository", timeout=400_000)
    require_success("StartProjectBackup", [project.id], timeout=220_000)
    backup = wait_job("backup_complete")
    if len(backup[10]) != 64:
        raise RuntimeError("Snapshotul restic nu are un identificator complet")
    history = call("GetProjectSnapshots", [project.id])
    if not history[0] or backup[10] not in history[2]:
        raise RuntimeError(f"Snapshotul creat nu apare in istoricul proiectului: {history}")

    require_success("StartBackupRestore", [project.id, backup[10]], timeout=220_000)
    restored = wait_job("restore_complete")
    restore_path = Path(restored[11])
    restored_file = restore_path / "document.txt"
    if restored_file.read_bytes() != expected:
        raise RuntimeError("Continutul restaurat nu corespunde fixture-ului")

    require_success("LockBackupRepository")
    require_success("LockConfiguration")
    print(
        "PASS: disposable repository init, peer-checked session secrets, snapshot, "
        f"repository check, filtered history and selected verified restore at {restore_path}"
    )
finally:
    if core.poll() is None:
        core.terminate()
        try:
            core.wait(timeout=10)
        except subprocess.TimeoutExpired:
            core.kill()
            core.wait(timeout=5)
    if core.returncode not in (0, -15):
        details = core.stderr.read().decode("utf-8", "replace")[-4000:]
        print(details)
