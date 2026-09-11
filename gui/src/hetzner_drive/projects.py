"""Private project registry and metadata-only backup activity monitor."""

from __future__ import annotations

from dataclasses import asdict, dataclass
import fnmatch
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import tempfile
import threading
import time
import uuid


SCHEMA_VERSION = 1
MAX_CONFIG_BYTES = 1024 * 1024
MAX_PROJECTS = 32
MAX_SCAN_ENTRIES = 100_000
MAX_SCAN_DEPTH = 64
MIN_INTERVAL_MINUTES = 15
MAX_INTERVAL_MINUTES = 7 * 24 * 60
MIN_QUIET_MINUTES = 1
MAX_QUIET_MINUTES = 120
PROJECT_ID = re.compile(r"^[a-f0-9]{32}$")
SAFE_PATTERN = re.compile(r"^[A-Za-z0-9._*?~#-]{1,64}$")
DIGEST = re.compile(r"^[a-f0-9]{64}$")
STATE_DIGEST_FIELDS = {"digest", "last_notified_digest", "last_verified_digest"}
STATE_INTEGER_FIELDS = {
    "file_count",
    "total_bytes",
    "dirty_since",
    "last_change",
    "last_scan",
    "snoozed_until",
    "last_notified",
    "last_verified_at",
}
DEFAULT_EXCLUDES = (
    "target",
    "node_modules",
    ".venv",
    "venv",
    "build",
    "dist",
    "__pycache__",
    ".cache",
    "*.pyc",
    "*.pyo",
    "*.swp",
    "*.tmp",
    "*~",
    ".DS_Store",
    ".directory",
    ".~lock.*#",
)


class ProjectError(ValueError):
    """A safe, user-facing project configuration error."""


@dataclass(frozen=True)
class BackupProject:
    id: str
    name: str
    path: str
    interval_minutes: int = 120
    quiet_minutes: int = 10
    mode: str = "ask"
    excludes: tuple[str, ...] = DEFAULT_EXCLUDES


@dataclass(frozen=True)
class ScanSnapshot:
    digest: str
    file_count: int
    total_bytes: int
    complete: bool
    reason: str = "ok"


@dataclass(frozen=True)
class ProjectStatus:
    id: str
    name: str
    path: str
    state: str
    file_count: int
    total_bytes: int
    dirty_since: int
    last_change: int
    last_verified_at: int
    snoozed_until: int
    reason: str
    due: bool


def default_config_path() -> Path:
    base = Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config"))
    return base / "hetzner-drive" / "projects.json"


def default_state_path() -> Path:
    base = Path(os.environ.get("XDG_STATE_HOME", Path.home() / ".local/state"))
    return base / "hetzner-drive" / "project-activity.json"


def _ensure_private_parent(path: Path) -> None:
    parent = path.parent
    if parent.exists():
        metadata = parent.lstat()
        if (
            not stat.S_ISDIR(metadata.st_mode)
            or parent.is_symlink()
            or metadata.st_uid != os.getuid()
            or metadata.st_mode & 0o077
        ):
            raise ProjectError("Directorul privat al aplicatiei are permisiuni nesigure.")
        return
    try:
        parent.mkdir(mode=0o700, parents=False)
    except FileNotFoundError:
        parent.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        if parent.parent.stat().st_uid != os.getuid():
            raise ProjectError("Directorul de configurare nu apartine utilizatorului curent.")
        parent.mkdir(mode=0o700, exist_ok=True)
    os.chmod(parent, 0o700)


def _read_private_json(path: Path, missing):
    try:
        descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    except FileNotFoundError:
        return missing
    except OSError as error:
        raise ProjectError(f"Fisier local nesigur: {path.name}") from error
    try:
        metadata = os.fstat(descriptor)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_uid != os.getuid()
            or metadata.st_mode & 0o077
            or metadata.st_size > MAX_CONFIG_BYTES
        ):
            raise ProjectError(f"Fisier local nesigur: {path.name}")
        with os.fdopen(descriptor, "r", encoding="utf-8") as handle:
            descriptor = None
            return json.load(handle)
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise ProjectError(f"Fisier local invalid: {path.name}") from error
    finally:
        if descriptor is not None:
            os.close(descriptor)


def _write_private_json(path: Path, value) -> None:
    _ensure_private_parent(path)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w", encoding="utf-8", prefix=f".{path.name}-", dir=path.parent, delete=False
        ) as handle:
            temporary = Path(handle.name)
            os.fchmod(handle.fileno(), 0o600)
            json.dump(value, handle, ensure_ascii=False, indent=2, sort_keys=True)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
        temporary = None
        directory_fd = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)
    except OSError as error:
        raise ProjectError("Configuratia proiectelor nu a putut fi salvata.") from error
    finally:
        if temporary is not None:
            try:
                temporary.unlink()
            except FileNotFoundError:
                pass


def _canonical_project_path(value: str, mount_path: Path, require_available: bool = False) -> Path:
    candidate = Path(value)
    if not candidate.is_absolute() or "\0" in value:
        raise ProjectError("Directorul proiectului trebuie sa fie o cale absoluta.")
    normalized = Path(os.path.normpath(candidate))
    if normalized != candidate:
        raise ProjectError("Directorul proiectului nu poate contine componente relative.")
    try:
        resolved = candidate.resolve(strict=False)
    except OSError as error:
        raise ProjectError("Directorul proiectului nu este disponibil.") from error
    if require_available and candidate != resolved:
        raise ProjectError("Directorul proiectului sau un parinte este un symlink.")
    if require_available:
        try:
            metadata = candidate.lstat()
        except FileNotFoundError:
            raise ProjectError("Directorul proiectului nu este disponibil.") from None
        except OSError as error:
            raise ProjectError("Directorul proiectului nu este disponibil.") from error
        if candidate.is_symlink() or not stat.S_ISDIR(metadata.st_mode):
            raise ProjectError("Calea proiectului nu este un director obisnuit.")
        if metadata.st_uid != os.getuid():
            raise ProjectError("Directorul proiectului nu apartine utilizatorului curent.")
    try:
        mount = mount_path.resolve(strict=False)
    except OSError:
        mount = mount_path
    safe_root = resolved if require_available else candidate
    if safe_root == mount or safe_root in mount.parents or mount in safe_root.parents:
        raise ProjectError("Proiectul nu poate include directorul Hetzner Drive.")
    return safe_root


def _validate_project(raw, mount_path: Path, require_available: bool = False) -> BackupProject:
    if not isinstance(raw, dict) or set(raw) != {
        "id", "name", "path", "interval_minutes", "quiet_minutes", "mode", "excludes"
    }:
        raise ProjectError("Schema unui proiect este invalida.")
    project_id = raw["id"]
    name = raw["name"]
    if not isinstance(project_id, str) or not PROJECT_ID.fullmatch(project_id):
        raise ProjectError("Identificatorul proiectului este invalid.")
    if not isinstance(name, str) or not 1 <= len(name.strip()) <= 80 or any(ord(c) < 32 for c in name):
        raise ProjectError("Numele proiectului este invalid.")
    interval = raw["interval_minutes"]
    quiet = raw["quiet_minutes"]
    if type(interval) is not int or not MIN_INTERVAL_MINUTES <= interval <= MAX_INTERVAL_MINUTES:
        raise ProjectError("Intervalul proiectului este invalid.")
    if type(quiet) is not int or not MIN_QUIET_MINUTES <= quiet <= MAX_QUIET_MINUTES:
        raise ProjectError("Perioada de repaus este invalida.")
    if raw["mode"] != "ask":
        raise ProjectError("Modul automat nu este activat inca.")
    excludes = raw["excludes"]
    if (
        not isinstance(excludes, list)
        or len(excludes) > 64
        or not all(isinstance(item, str) and SAFE_PATTERN.fullmatch(item) for item in excludes)
    ):
        raise ProjectError("Lista de excluderi este invalida.")
    root = _canonical_project_path(raw["path"], mount_path, require_available)
    return BackupProject(
        id=project_id,
        name=name.strip(),
        path=str(root),
        interval_minutes=interval,
        quiet_minutes=quiet,
        mode="ask",
        excludes=tuple(excludes),
    )


class ProjectRegistry:
    def __init__(self, path: Path | None = None, mount_path: Path | None = None):
        self.path = path or default_config_path()
        self.mount_path = mount_path or Path.home() / "HetznerDrive"

    def load(self) -> list[BackupProject]:
        raw = _read_private_json(self.path, {"version": SCHEMA_VERSION, "projects": []})
        if not isinstance(raw, dict) or set(raw) != {"version", "projects"}:
            raise ProjectError("Schema registrului de proiecte este invalida.")
        if raw["version"] != SCHEMA_VERSION or not isinstance(raw["projects"], list):
            raise ProjectError("Versiunea registrului de proiecte nu este compatibila.")
        if len(raw["projects"]) > MAX_PROJECTS:
            raise ProjectError("Registrul contine prea multe proiecte.")
        projects = [_validate_project(item, self.mount_path) for item in raw["projects"]]
        if len({project.id for project in projects}) != len(projects):
            raise ProjectError("Registrul contine identificatori duplicati.")
        roots = [Path(project.path) for project in projects]
        for index, root in enumerate(roots):
            for other in roots[index + 1:]:
                if root == other or root in other.parents or other in root.parents:
                    raise ProjectError("Directoarele proiectelor nu se pot suprapune.")
        return projects

    def save(self, projects: list[BackupProject]) -> None:
        if len(projects) > MAX_PROJECTS:
            raise ProjectError("Pot fi urmarite cel mult 32 de proiecte.")
        validated = [
            _validate_project({**asdict(project), "excludes": list(project.excludes)}, self.mount_path)
            for project in projects
        ]
        _write_private_json(
            self.path,
            {
                "version": SCHEMA_VERSION,
                "projects": [{**asdict(project), "excludes": list(project.excludes)} for project in validated],
            },
        )

    def add(self, name: str, path: str, interval_minutes: int = 120, quiet_minutes: int = 10) -> BackupProject:
        projects = self.load()
        project = _validate_project(
            {
                "id": uuid.uuid4().hex,
                "name": name,
                "path": path,
                "interval_minutes": interval_minutes,
                "quiet_minutes": quiet_minutes,
                "mode": "ask",
                "excludes": list(DEFAULT_EXCLUDES),
            },
            self.mount_path,
            require_available=True,
        )
        root = Path(project.path)
        for existing in projects:
            existing_root = Path(existing.path)
            if root == existing_root or root in existing_root.parents or existing_root in root.parents:
                raise ProjectError("Directorul este deja urmarit sau se suprapune cu alt proiect.")
        projects.append(project)
        self.save(projects)
        return project

    def remove(self, project_id: str) -> bool:
        projects = self.load()
        remaining = [project for project in projects if project.id != project_id]
        if len(remaining) == len(projects):
            return False
        self.save(remaining)
        return True


class _ScanLimit(Exception):
    pass


def _open_directory_no_symlinks(path: Path) -> int:
    if not path.is_absolute():
        raise OSError("relative project path")
    flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC
    descriptor = os.open("/", flags)
    try:
        for component in path.parts[1:]:
            next_descriptor = os.open(component, flags, dir_fd=descriptor)
            os.close(descriptor)
            descriptor = next_descriptor
        return descriptor
    except Exception:
        os.close(descriptor)
        raise


def scan_project(project: BackupProject) -> ScanSnapshot:
    """Hash names and metadata only; never open file contents or follow symlinks."""
    digest = hashlib.blake2b(digest_size=32)
    file_count = 0
    total_bytes = 0
    entries_seen = 0

    def walk(descriptor: int, relative_directory: Path, depth: int) -> None:
        nonlocal file_count, total_bytes, entries_seen
        if depth > MAX_SCAN_DEPTH:
            raise _ScanLimit
        names = []
        with os.scandir(descriptor) as iterator:
            for entry in iterator:
                if any(fnmatch.fnmatchcase(entry.name, pattern) for pattern in project.excludes):
                    continue
                names.append(entry.name)
                if len(names) + entries_seen > MAX_SCAN_ENTRIES:
                    raise _ScanLimit
        for name in sorted(names):
            entries_seen += 1
            metadata = os.stat(name, dir_fd=descriptor, follow_symlinks=False)
            if stat.S_ISLNK(metadata.st_mode):
                continue
            relative = relative_directory / name
            encoded = relative.as_posix().encode("utf-8", "surrogateescape")
            if stat.S_ISDIR(metadata.st_mode):
                digest.update(b"D\0" + encoded + b"\0")
                child = os.open(
                    name,
                    os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC,
                    dir_fd=descriptor,
                )
                try:
                    walk(child, relative, depth + 1)
                finally:
                    os.close(child)
            elif stat.S_ISREG(metadata.st_mode):
                digest.update(b"F\0" + encoded + b"\0")
                digest.update(str(metadata.st_size).encode() + b"\0")
                digest.update(str(metadata.st_mtime_ns).encode() + b"\0")
                digest.update(str(stat.S_IMODE(metadata.st_mode)).encode() + b"\0")
                file_count += 1
                total_bytes += metadata.st_size

    descriptor = None
    try:
        descriptor = _open_directory_no_symlinks(Path(project.path))
        root_metadata = os.fstat(descriptor)
        if not stat.S_ISDIR(root_metadata.st_mode) or root_metadata.st_uid != os.getuid():
            return ScanSnapshot("", 0, 0, False, "project_path_unsafe")
        walk(descriptor, Path(), 0)
        return ScanSnapshot(digest.hexdigest(), file_count, total_bytes, True)
    except _ScanLimit:
        return ScanSnapshot("", file_count, total_bytes, False, "scan_limit")
    except (OSError, UnicodeError):
        return ScanSnapshot("", file_count, total_bytes, False, "scan_unavailable")
    finally:
        if descriptor is not None:
            os.close(descriptor)


class ProjectMonitor:
    def __init__(
        self,
        registry: ProjectRegistry | None = None,
        state_path: Path | None = None,
        clock=time.time,
    ):
        self.registry = registry or ProjectRegistry()
        self.state_path = state_path or default_state_path()
        self.clock = clock
        self._lock = threading.Lock()

    def _load_state(self) -> dict:
        raw = _read_private_json(self.state_path, {"version": SCHEMA_VERSION, "projects": {}})
        if not isinstance(raw, dict) or set(raw) != {"version", "projects"}:
            raise ProjectError("Schema starii proiectelor este invalida.")
        if raw["version"] != SCHEMA_VERSION or not isinstance(raw["projects"], dict):
            raise ProjectError("Versiunea starii proiectelor nu este compatibila.")
        if len(raw["projects"]) > MAX_PROJECTS:
            raise ProjectError("Starea locala contine prea multe proiecte.")
        for project_id, record in raw["projects"].items():
            if not isinstance(project_id, str) or not PROJECT_ID.fullmatch(project_id):
                raise ProjectError("Starea locala contine un identificator invalid.")
            if not isinstance(record, dict) or not set(record) <= STATE_DIGEST_FIELDS | STATE_INTEGER_FIELDS:
                raise ProjectError("Schema starii unui proiect este invalida.")
            if any(
                not isinstance(record[field], str)
                or (record[field] and not DIGEST.fullmatch(record[field]))
                for field in set(record) & STATE_DIGEST_FIELDS
            ):
                raise ProjectError("Starea locala contine un digest invalid.")
            if any(
                type(record[field]) is not int or not 0 <= record[field] <= (1 << 63) - 1
                for field in set(record) & STATE_INTEGER_FIELDS
            ):
                raise ProjectError("Starea locala contine un contor invalid.")
        return raw

    def _save_state(self, state: dict) -> None:
        _write_private_json(self.state_path, state)

    def projects(self) -> list[BackupProject]:
        with self._lock:
            return self.registry.load()

    def add_project(self, name: str, path: str, interval_minutes: int = 120) -> BackupProject:
        with self._lock:
            return self.registry.add(name, path, interval_minutes)

    def remove_project(self, project_id: str) -> bool:
        with self._lock:
            removed = self.registry.remove(project_id)
            if removed:
                state = self._load_state()
                state["projects"].pop(project_id, None)
                self._save_state(state)
            return removed

    def snooze(self, project_id: str, minutes: int = 60, now: int | None = None) -> None:
        if not 15 <= minutes <= 24 * 60:
            raise ProjectError("Intervalul de amanare este invalid.")
        with self._lock:
            if project_id not in {project.id for project in self.registry.load()}:
                raise ProjectError("Proiectul nu mai exista.")
            state = self._load_state()
            record = state["projects"].setdefault(project_id, {})
            record["snoozed_until"] = int(now if now is not None else self.clock()) + minutes * 60
            record["last_notified"] = 0
            self._save_state(state)

    def mark_notified(self, project_ids: list[str], now: int | None = None) -> None:
        with self._lock:
            state = self._load_state()
            timestamp = int(now if now is not None else self.clock())
            for project_id in project_ids:
                record = state["projects"].get(project_id)
                if record:
                    record["last_notified"] = timestamp
                    record["last_notified_digest"] = record.get("digest", "")
            self._save_state(state)

    def current_digest(self, project_id: str) -> str:
        with self._lock:
            state = self._load_state()
            record = state["projects"].get(project_id)
            if not record or not record.get("digest"):
                raise ProjectError("Proiectul nu are o scanare completa.")
            return record["digest"]

    def mark_verified(
        self, project_id: str, now: int | None = None, expected_digest: str | None = None
    ) -> None:
        """Reserved for the audited snapshot engine; useful for state tests."""
        with self._lock:
            state = self._load_state()
            record = state["projects"].get(project_id)
            if not record or not record.get("digest"):
                raise ProjectError("Proiectul nu are o scanare completa.")
            if expected_digest is not None and record["digest"] != expected_digest:
                raise ProjectError(
                    "Proiectul s-a modificat in timpul backupului; snapshotul ramane valid, dar este necesar unul nou."
                )
            timestamp = int(now if now is not None else self.clock())
            record["last_verified_at"] = timestamp
            record["last_verified_digest"] = record["digest"]
            record["dirty_since"] = 0
            record["snoozed_until"] = 0
            record["last_notified"] = 0
            self._save_state(state)

    def refresh(self, now: int | None = None) -> list[ProjectStatus]:
        with self._lock:
            timestamp = int(now if now is not None else self.clock())
            projects = self.registry.load()
            state = self._load_state()
            records = state["projects"]
            known_ids = {project.id for project in projects}
            for stale_id in set(records) - known_ids:
                del records[stale_id]
            statuses = []
            for project in projects:
                snapshot = scan_project(project)
                record = records.setdefault(project.id, {})
                previous_digest = record.get("digest", "")
                verified_digest = record.get("last_verified_digest", "")
                dirty_since = int(record.get("dirty_since", 0))
                last_change = int(record.get("last_change", 0))
                if snapshot.complete:
                    if snapshot.digest == verified_digest and verified_digest:
                        dirty_since = 0
                    elif not dirty_since:
                        dirty_since = timestamp
                    if previous_digest and previous_digest != snapshot.digest:
                        last_change = timestamp
                    elif not last_change:
                        last_change = timestamp
                    record.update(
                        digest=snapshot.digest,
                        file_count=snapshot.file_count,
                        total_bytes=snapshot.total_bytes,
                        dirty_since=dirty_since,
                        last_change=last_change,
                        last_scan=timestamp,
                    )
                snoozed_until = int(record.get("snoozed_until", 0))
                last_verified_at = int(record.get("last_verified_at", 0))
                quiet = timestamp - last_change >= project.quiet_minutes * 60
                old_enough = dirty_since and timestamp - dirty_since >= project.interval_minutes * 60
                due = bool(snapshot.complete and dirty_since and quiet and old_enough and timestamp >= snoozed_until)
                if not snapshot.complete:
                    project_state = "attention"
                elif not dirty_since:
                    project_state = "verified"
                elif timestamp < snoozed_until:
                    project_state = "snoozed"
                elif due:
                    project_state = "due"
                elif not last_verified_at:
                    project_state = "unprotected"
                else:
                    project_state = "changed"
                statuses.append(
                    ProjectStatus(
                        id=project.id,
                        name=project.name,
                        path=project.path,
                        state=project_state,
                        file_count=snapshot.file_count,
                        total_bytes=snapshot.total_bytes,
                        dirty_since=dirty_since,
                        last_change=last_change,
                        last_verified_at=last_verified_at,
                        snoozed_until=snoozed_until,
                        reason=snapshot.reason,
                        due=due,
                    )
                )
            if projects or self.state_path.exists():
                self._save_state(state)
            return statuses

    def notification_candidates(self, statuses: list[ProjectStatus], now: int | None = None) -> list[ProjectStatus]:
        with self._lock:
            timestamp = int(now if now is not None else self.clock())
            state = self._load_state()
            result = []
            for status in statuses:
                if not status.due:
                    continue
                record = state["projects"].get(status.id, {})
                last_notified = int(record.get("last_notified", 0))
                same_digest = record.get("last_notified_digest", "") == record.get("digest", "")
                if not same_digest or timestamp - last_notified >= 6 * 60 * 60:
                    result.append(status)
            return result
