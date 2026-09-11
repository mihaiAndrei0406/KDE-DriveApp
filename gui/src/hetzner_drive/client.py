from PySide6.QtCore import QObject, Signal, Slot
from PySide6.QtDBus import QDBusArgument, QDBusConnection, QDBusMessage, QDBusPendingCallWatcher

BUS = "ro.mihai.HetznerDrive1"
OBJECT = "/ro/mihai/HetznerDrive1"
FIELDS = (
    "state", "mount", "ssh", "config", "version", "mount_path",
    "cache_bytes", "cache_complete", "diagnostic",
)
OPERATIONS = frozenset({
    "UnlockConfiguration", "LockConfiguration", "MountDrive", "UnmountDrive",
    "CheckConnection", "RunHealthCheck", "OpenDrive",
})
BACKUP_OPERATIONS = frozenset({
    "InitializeBackupRepository", "UnlockBackupRepository", "LockBackupRepository",
    "StartProjectBackup", "StartBackupRestore",
})
METHODS = OPERATIONS | BACKUP_OPERATIONS | {
    "GetStatus", "GetOperationStatus", "GetMountActivity", "GetStorageUsage",
    "CheckSshAgent", "GetRecentLogs", "GetBackupStatus", "GetProjectSnapshots",
}


def decode_status(arguments):
    if len(arguments) != len(FIELDS):
        raise ValueError("Invalid status field count")
    for index in (0, 1, 2, 3, 4, 5, 8):
        if not isinstance(arguments[index], str):
            raise ValueError("Invalid status text")
    if type(arguments[6]) is not int or arguments[6] < 0:
        raise ValueError("Invalid cache allocation")
    if type(arguments[7]) is not bool:
        raise ValueError("Invalid cache completeness")
    return dict(zip(FIELDS, arguments, strict=True))


def decode_snapshot_history(arguments):
    if (
        len(arguments) != 7
        or type(arguments[0]) is not bool
        or type(arguments[1]) is not bool
        or not isinstance(arguments[6], str)
    ):
        raise ValueError("Invalid snapshot history response")
    success, truncated, *encoded_arrays, reason = arguments
    arrays = []
    for encoded in encoded_arrays:
        if isinstance(encoded, list):
            arrays.append(encoded)
            continue
        if not isinstance(encoded, QDBusArgument):
            raise ValueError("Invalid snapshot history array")
        values = []
        encoded.beginArray()
        while not encoded.atEnd():
            values.append(encoded.asVariant())
        # PySide6 6.11 selects the writable endArray overload for a read-only
        # reply and emits a warning. The one-shot reply argument is discarded.
        arrays.append(values)
    snapshot_ids, created_at, files, sizes = arrays
    count = len(snapshot_ids)
    if count > 256 or any(len(values) != count for values in (created_at, files, sizes)):
        raise ValueError("Invalid snapshot history shape")
    entries = []
    for snapshot_id, timestamp, file_count, size in zip(
        snapshot_ids, created_at, files, sizes, strict=True
    ):
        if (
            not isinstance(snapshot_id, str)
            or len(snapshot_id) != 64
            or any(character not in "0123456789abcdef" for character in snapshot_id)
            or not isinstance(timestamp, str)
            or not 20 <= len(timestamp) <= 64
            or "T" not in timestamp
            or any(ord(character) < 32 or ord(character) == 127 for character in timestamp)
            or type(file_count) is not int
            or file_count < 0
            or type(size) is not int
            or size < 0
        ):
            raise ValueError("Invalid snapshot history entry")
        entries.append({
            "id": snapshot_id,
            "created_at": timestamp,
            "files": file_count,
            "bytes": size,
        })
    if not success and (truncated or entries):
        raise ValueError("Invalid failed snapshot history response")
    return success, truncated, entries, reason


class DriveClient(QObject):
    status_received = Signal(dict)
    logs_received = Signal(list)
    ssh_received = Signal(str)
    failed = Signal(str)
    busy_changed = Signal(bool)
    storage_received = Signal(bool, object, object, object, str)
    operation_received = Signal(str, bool, str)
    operation_status_received = Signal(bool, str, str, bool)
    mount_activity_received = Signal(bool, object, object, object, bool, str)
    backup_status_received = Signal(
        bool, bool, bool, bool, str, str, object, object, object, object, str, str, str
    )
    snapshot_history_received = Signal(str, bool, bool, object, str)

    def __init__(self, parent=None):
        super().__init__(parent)
        self.bus = QDBusConnection.sessionBus()
        self.pending = {}
        self.refresh_again = set()

    def refresh(self):
        self._request("GetStatus")
        self._request("GetOperationStatus")
        self._request("GetMountActivity")
        self._request("GetBackupStatus")

    def check_ssh(self):
        self._request("CheckSshAgent")

    def get_logs(self):
        self._request("GetRecentLogs")

    def unlock(self):
        self._request("UnlockConfiguration")

    def lock(self):
        self._request("LockConfiguration")

    def check_connection(self):
        self._request("CheckConnection")

    def mount(self):
        self._request("MountDrive")

    def unmount(self):
        self._request("UnmountDrive")

    def health_check(self):
        self._request("RunHealthCheck")

    def open_drive(self):
        self._request("OpenDrive")

    def get_storage(self):
        self._request("GetStorageUsage")

    def initialize_backup_repository(self):
        self._request("InitializeBackupRepository")

    def unlock_backup_repository(self):
        self._request("UnlockBackupRepository")

    def lock_backup_repository(self):
        self._request("LockBackupRepository")

    def start_project_backup(self, project_id):
        self._request("StartProjectBackup", project_id)

    def get_project_snapshots(self, project_id):
        self._request("GetProjectSnapshots", project_id)

    def start_backup_restore(self, project_id, snapshot_id):
        self._request("StartBackupRestore", project_id, snapshot_id)

    def _request(self, method, *arguments):
        if method not in METHODS:
            raise ValueError("Unsupported method")
        if method in self.pending:
            # An action can finish while a previous status request is in flight.
            # Keep one follow-up refresh so that its result cannot be lost.
            if method in {"GetStatus", "GetOperationStatus"}:
                self.refresh_again.add(method)
            return
        message = QDBusMessage.createMethodCall(BUS, OBJECT, BUS, method)
        if arguments:
            message.setArguments(list(arguments))
        timeout = (
            220000
            if method in {"UnlockConfiguration", "InitializeBackupRepository", "UnlockBackupRepository"}
            else 200000
            if method in {"StartProjectBackup", "StartBackupRestore"}
            else 45000
            if method in OPERATIONS | BACKUP_OPERATIONS | {"GetStorageUsage"}
            else 12000
        )
        watcher = QDBusPendingCallWatcher(self.bus.asyncCall(message, timeout), self)
        self.pending[method] = watcher
        watcher.setProperty("operation", method)
        if method == "GetProjectSnapshots":
            watcher.setProperty("project_id", arguments[0])
        watcher.finished.connect(self._finished)
        self.busy_changed.emit(True)

    @Slot(QDBusPendingCallWatcher)
    def _finished(self, watcher):
        method = watcher.property("operation")
        self.pending.pop(method, None)
        reply = watcher.reply()
        watcher.deleteLater()
        if method in self.refresh_again:
            self.refresh_again.remove(method)
            self._request(method)
        self.busy_changed.emit(bool(self.pending))
        if reply.type() == QDBusMessage.MessageType.ErrorMessage:
            self.failed.emit("Serviciul nu raspunde. Verifica daca este pornit in sesiunea curenta.")
            return
        values = reply.arguments()
        try:
            if method == "GetStatus":
                self.status_received.emit(decode_status(values))
            elif method == "CheckSshAgent":
                if len(values) != 1 or not isinstance(values[0], str):
                    raise ValueError("Invalid SSH response")
                self.ssh_received.emit(values[0])
            elif method == "GetRecentLogs":
                if len(values) != 1 or not isinstance(values[0], list) or not all(isinstance(v, str) for v in values[0]):
                    raise ValueError("Invalid log response")
                self.logs_received.emit(values[0])
            elif method == "GetStorageUsage":
                if len(values) != 5 or type(values[0]) is not bool or not all(type(v) is int and v >= 0 for v in values[1:4]) or not isinstance(values[4], str):
                    raise ValueError("Invalid storage response")
                self.storage_received.emit(*values)
            elif method == "GetOperationStatus":
                if len(values) != 4 or type(values[0]) is not bool or type(values[3]) is not bool or not all(isinstance(v, str) for v in values[1:3]):
                    raise ValueError("Invalid operation status")
                self.operation_status_received.emit(*values)
            elif method == "GetMountActivity":
                if len(values) != 6 or type(values[0]) is not bool or type(values[4]) is not bool or not all(type(v) is int and v >= 0 for v in values[1:4]) or not isinstance(values[5], str):
                    raise ValueError("Invalid mount activity")
                self.mount_activity_received.emit(*values)
            elif method == "GetBackupStatus":
                if (
                    len(values) != 13
                    or not all(type(value) is bool for value in values[:4])
                    or not all(isinstance(value, str) for value in values[4:6])
                    or not all(type(value) is int and value >= 0 for value in values[6:10])
                    or not all(isinstance(value, str) for value in values[10:])
                ):
                    raise ValueError("Invalid backup status")
                self.backup_status_received.emit(*values)
            elif method == "GetProjectSnapshots":
                success, truncated, entries, reason = decode_snapshot_history(values)
                self.snapshot_history_received.emit(
                    watcher.property("project_id"), success, truncated, entries, reason
                )
            elif method in OPERATIONS | BACKUP_OPERATIONS:
                if len(values) != 2 or type(values[0]) is not bool or not isinstance(values[1], str):
                    raise ValueError("Invalid operation response")
                self.operation_received.emit(method, *values)
        except (ValueError, TypeError):
            if method == "GetProjectSnapshots":
                self.snapshot_history_received.emit(
                    watcher.property("project_id"), False, False, [], "backup_history_invalid"
                )
            self.failed.emit("Raspuns incompatibil primit de la serviciu.")
