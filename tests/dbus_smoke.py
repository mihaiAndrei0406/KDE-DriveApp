"""Run on tests/dbus-session.conf; only the synthetic core may be started here."""
import os
from pathlib import Path
import subprocess
import tempfile
import time
import xml.etree.ElementTree as ET

os.environ["QT_QPA_PLATFORM"] = "offscreen"
test_xdg = tempfile.TemporaryDirectory(prefix="hetzner-drive-smoke-")
os.environ["XDG_CONFIG_HOME"] = str(Path(test_xdg.name) / "config")
os.environ["XDG_STATE_HOME"] = str(Path(test_xdg.name) / "state")

from PySide6.QtDBus import QDBusConnection, QDBusMessage
from PySide6.QtWidgets import QApplication
from hetzner_drive.app import DriveWindow, SingleInstance
from hetzner_drive.client import (
    BUS,
    OBJECT,
    DriveClient,
    decode_snapshot_entries,
    decode_snapshot_history,
    decode_status,
)

ROOT = Path(__file__).resolve().parents[1]
app = QApplication([])
bus = QDBusConnection.sessionBus()

# Fail closed before starting or contacting the application service. This guard
# catches invocations on a normal user bus, or with dbus-run-session's default
# service directories, where the installed release backend could be activated.
activatable_reply = bus.call(
    QDBusMessage.createMethodCall(
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "ListActivatableNames",
    ),
    timeout=3000,
)
if activatable_reply.type() != QDBusMessage.MessageType.ReplyMessage:
    raise RuntimeError(f"Could not inspect D-Bus activation: {activatable_reply.errorMessage()}")
if BUS in activatable_reply.arguments()[0]:
    raise RuntimeError(
        "Installed Hetzner Drive activation is visible. Run with "
        "dbus-run-session --config-file=tests/dbus-session.conf -- ..."
    )


def call(method, interface=BUS, arguments=None):
    message = QDBusMessage.createMethodCall(BUS, OBJECT, interface, method)
    if arguments:
        message.setArguments(arguments)
    return bus.call(message, timeout=3000)


def wait_for(predicate, message):
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        app.processEvents()
        if predicate():
            return
        time.sleep(0.01)
    raise AssertionError(message)


single_name = f"hetzner-drive-gui-smoke-{os.getpid()}"
single_lock = Path(test_xdg.name) / "gui.lock"
primary_gui = SingleInstance(single_name, single_lock)
assert primary_gui.acquire()
activation_requests = []
primary_gui.activate_requested.connect(lambda: activation_requests.append(True))
duplicate_gui = SingleInstance(single_name, single_lock)
assert not duplicate_gui.acquire()
wait_for(lambda: bool(activation_requests), "Existing GUI activation was not requested")
primary_gui.server.close()
primary_gui.lock.unlock()


core = subprocess.Popen([str(ROOT / "target/debug/hetzner-drive-core"), "--demo"],
                        stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
try:
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        if core.poll() is not None:
            raise RuntimeError("Demo core failed to start")
        if bus.interface().isServiceRegistered(BUS).value():
            break
        time.sleep(0.03)
    else:
        raise RuntimeError("Service registration timed out")

    status = call("GetStatus")
    assert status.type() == QDBusMessage.MessageType.ReplyMessage, status.errorMessage()
    snapshot = decode_status(status.arguments())
    assert snapshot["state"] == "Locked"
    assert snapshot["diagnostic"] == "demo_data"
    original_owner = bus.interface().serviceOwner(BUS).value()
    duplicate = subprocess.run([str(ROOT / "target/debug/hetzner-drive-core"), "--demo"],
                               stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, timeout=3)
    assert duplicate.returncode != 0
    assert bus.interface().serviceOwner(BUS).value() == original_owner
    assert call("CheckSshAgent").arguments() == ["loaded"]
    assert call("GetStorageUsage").arguments() == [False, 0, 0, 0, "configuration_locked"]
    assert isinstance(call("GetRecentLogs").arguments()[0], list)
    assert call("ExecuteCommand").type() == QDBusMessage.MessageType.ErrorMessage
    assert call("MountDrive").arguments() == [False, "configuration_locked"]
    xml = call("Introspect", "org.freedesktop.DBus.Introspectable").arguments()[0]
    interface = ET.fromstring(xml).find(f"interface[@name='{BUS}']")
    assert interface is not None
    assert {m.attrib["name"] for m in interface.findall("method")} == {
        "GetStatus", "CheckSshAgent", "GetStorageUsage", "GetRecentLogs", "UnlockConfiguration",
        "LockConfiguration", "CheckConnection", "RunHealthCheck", "OpenDrive", "GetOperationStatus",
        "MountDrive", "UnmountDrive", "GetMountActivity"
        , "GetBackupStatus", "InitializeBackupRepository", "UnlockBackupRepository",
        "LockBackupRepository", "StartProjectBackup", "GetProjectSnapshots", "StartBackupRestore",
        "GetSnapshotEntries", "StartSelectiveRestore",
    }

    client = DriveClient()
    window = DriveWindow(client)
    received, failures = [], []
    client.status_received.connect(received.append)
    client.failed.connect(failures.append)
    window.show()
    client.refresh()
    wait_for(lambda: received and not client.pending, "Initial GUI status timed out")
    assert received and not failures, failures
    assert window.values["ssh"].text() == "Cheia Hetzner este incarcata"
    logs = []
    client.logs_received.connect(logs.append)
    client.get_logs()
    wait_for(lambda: logs and not client.pending, "Log response timed out")
    assert logs and not failures, failures

    assert call("UnlockConfiguration").arguments() == [True, "ok"]
    assert decode_status(call("GetStatus").arguments())["state"] == "Ready"
    backup_status = call("GetBackupStatus").arguments()
    assert backup_status[:4] == [True, True, True, False]
    project_id = "a" * 32
    assert call("StartProjectBackup", arguments=[project_id]).arguments() == [True, "accepted"]
    wait_for(lambda: call("GetBackupStatus").arguments()[4] == "backup_complete",
             "Demo backup did not complete")
    backup_status = call("GetBackupStatus").arguments()
    assert backup_status[5] == project_id and len(backup_status[10]) == 64
    history = call("GetProjectSnapshots", arguments=[project_id]).arguments()
    history_success, history_truncated, history_entries, history_reason = decode_snapshot_history(history)
    assert history_success and not history_truncated and history_reason == "ok"
    assert len(history_entries) == 1 and len(history_entries[0]["id"]) == 64
    async_history = []
    client.snapshot_history_received.connect(lambda *values: async_history.append(values))
    client.get_project_snapshots(project_id)
    wait_for(lambda: async_history and not client.pending, "Async history response timed out")
    assert async_history[0][0] == project_id and async_history[0][1:3] == (True, False)
    assert async_history[0][3][0]["id"] == history_entries[0]["id"]
    contents = call(
        "GetSnapshotEntries", arguments=[project_id, history_entries[0]["id"]]
    ).arguments()
    contents_success, contents_truncated, content_entries, contents_reason = (
        decode_snapshot_entries(contents)
    )
    assert contents_success and not contents_truncated and contents_reason == "ok"
    assert [entry["path"] for entry in content_entries] == ["README.md", "src", "src/main.rs"]
    async_contents = []
    client.snapshot_entries_received.connect(lambda *values: async_contents.append(values))
    client.get_snapshot_entries(project_id, history_entries[0]["id"])
    wait_for(lambda: async_contents and not client.pending, "Async snapshot contents timed out")
    assert async_contents[0][:4] == (project_id, history_entries[0]["id"], True, False)
    assert call(
        "StartSelectiveRestore",
        arguments=[project_id, history_entries[0]["id"], "src/main.rs"],
    ).arguments() == [True, "accepted"]
    wait_for(lambda: call("GetBackupStatus").arguments()[4] == "restore_complete",
             "Demo selective restore did not complete")
    assert call(
        "StartBackupRestore", arguments=[project_id, history_entries[0]["id"]]
    ).arguments() == [True, "accepted"]
    wait_for(lambda: call("GetBackupStatus").arguments()[4] == "restore_complete",
             "Demo restore did not complete")
    assert call("GetBackupStatus").arguments()[11] == "/tmp/hetzner-drive-demo-restore"
    assert call("CheckConnection").arguments() == [True, "ok"]
    assert call("RunHealthCheck").arguments() == [True, "ok"]
    assert call("GetOperationStatus").arguments()[:2] == [False, "health_check"]
    assert call("OpenDrive").arguments() == [False, "drive_not_mounted"]
    assert call("GetOperationStatus").arguments() == [False, "open_drive", "drive_not_mounted", False]
    assert call("MountDrive").arguments() == [True, "ok"]
    assert decode_status(call("GetStatus").arguments())["state"] == "Mounted"
    assert call("GetOperationStatus").arguments() == [False, "mount_drive", "drive_not_mounted", True]
    assert call("GetMountActivity").arguments() == [True, 0, 0, 0, False, "ok"]
    assert call("OpenDrive").arguments() == [True, "demo_open_drive"]
    assert call("UnmountDrive").arguments() == [True, "ok"]
    assert call("GetMountActivity").arguments() == [False, 0, 0, 0, False, "drive_not_mounted"]
    assert decode_status(call("GetStatus").arguments())["state"] == "Ready"
    storage = []
    client.storage_received.connect(lambda *values: storage.append(values))
    client.get_storage()
    wait_for(lambda: bool(storage), "Storage response timed out")
    assert storage and storage[0][1] == 1_000_000_000_000
    assert call("LockConfiguration").arguments() == [True, "ok"]
    assert decode_status(call("GetStatus").arguments())["state"] == "Locked"
    assert call("RunHealthCheck").arguments() == [True, "local_ok_configuration_locked"]
    client.refresh()

    # The storage handler already started a refresh before lock/health completed.
    # A repeated refresh must fetch the new operation, in either reply order.
    wait_for(lambda: not client.pending, "Overlapping GUI refreshes timed out")
    assert window.values["last_operation"].text() == "Diagnostic"
    assert window.values["last_error"].text() == "Drive-ul nu este montat."

    screenshot_dir = os.environ.get("HETZNER_TEST_SCREENSHOTS")
    if screenshot_dir:
        destination = Path(screenshot_dir)
        destination.mkdir(parents=True, exist_ok=True)
        for width, height in [(800, 610), (540, 480)]:
            window.resize(width, height)
            app.processEvents()
            assert window.grab().save(str(destination / f"drive-{width}.png"))

    core.terminate()
    core.wait(timeout=5)
    client.refresh()
    wait_for(lambda: failures and not client.pending, "Service loss was not detected")
    assert failures
    assert window.state_label.text() == "Serviciu indisponibil"
    core.stderr.close()
    core = subprocess.Popen([str(ROOT / "target/debug/hetzner-drive-core"), "--demo"],
                            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        if bus.interface().isServiceRegistered(BUS).value():
            break
        time.sleep(0.03)
    else:
        raise RuntimeError("Restart registration timed out")
    received.clear()
    client.refresh()
    wait_for(lambda: received and not client.pending, "GUI recovery timed out")
    assert received
    assert window.state_label.text() == "Deblocare necesara"
    assert call("UnlockConfiguration").arguments() == [True, "ok"]
    assert call("MountDrive").arguments() == [True, "ok"]
    core.terminate()
    core.wait(timeout=5)
    assert core.returncode == 0
    window.timer.stop()
    window.backup_timer.stop()
    if window.tray:
        window.tray.hide()
    window.close()
    print(
        "PASS: single GUI, typed D-Bus API, duplicate rejection, allowlist, unlock/lock, "
        "snapshot contents/selective restore, 64-bit storage, async GUI, service loss/recovery"
    )
finally:
    if core.poll() is None:
        core.terminate()
        try:
            core.wait(timeout=5)
        except subprocess.TimeoutExpired:
            core.kill()
            core.wait(timeout=5)
    core.stderr.close()
    test_xdg.cleanup()
