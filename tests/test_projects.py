import json
import os
from pathlib import Path
import stat
import tempfile
import unittest

from hetzner_drive.projects import (
    BackupProject,
    DEFAULT_EXCLUDES,
    ProjectError,
    ProjectMonitor,
    ProjectRegistry,
    scan_project,
)


class ProjectMonitorTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.project = self.root / "project"
        self.project.mkdir()
        self.mount = self.root / "mount"
        self.mount.mkdir()
        self.registry_path = self.root / "config" / "projects.json"
        self.state_path = self.root / "state" / "activity.json"
        self.registry = ProjectRegistry(self.registry_path, self.mount)
        self.monitor = ProjectMonitor(self.registry, self.state_path)

    def tearDown(self):
        self.temporary.cleanup()

    def test_registry_is_private_atomic_and_rejects_overlaps(self):
        project = self.registry.add("Teza", str(self.project), interval_minutes=120)
        self.assertEqual(self.registry.load(), [project])
        self.assertEqual(stat.S_IMODE(self.registry_path.stat().st_mode), 0o600)
        self.assertEqual(stat.S_IMODE(self.registry_path.parent.stat().st_mode), 0o700)

        nested = self.project / "nested"
        nested.mkdir()
        with self.assertRaises(ProjectError):
            self.registry.add("Suprapus", str(nested), interval_minutes=120)
        with self.assertRaises(ProjectError):
            self.registry.add("Drive", str(self.mount), interval_minutes=120)

        link = self.root / "project-link"
        link.symlink_to(self.project, target_is_directory=True)
        with self.assertRaises(ProjectError):
            self.registry.add("Symlink", str(link), interval_minutes=120)

        os.chmod(self.registry_path, 0o644)
        with self.assertRaises(ProjectError):
            self.registry.load()

    def test_missing_project_remains_removable(self):
        project = self.registry.add("Temporar", str(self.project), interval_minutes=120)
        self.project.rmdir()
        statuses = self.monitor.refresh(now=1_000)
        self.assertEqual(statuses[0].state, "attention")
        self.assertEqual(statuses[0].reason, "scan_unavailable")
        self.assertTrue(self.monitor.remove_project(project.id))
        self.assertEqual(self.registry.load(), [])

    def test_project_replaced_by_symlink_is_blocked_but_removable(self):
        project = self.registry.add("Schimbat", str(self.project), interval_minutes=120)
        replacement = self.root / "replacement"
        replacement.mkdir()
        self.project.rmdir()
        self.project.symlink_to(replacement, target_is_directory=True)
        statuses = self.monitor.refresh(now=1_000)
        self.assertEqual(statuses[0].state, "attention")
        self.assertEqual(statuses[0].reason, "scan_unavailable")
        self.assertTrue(self.monitor.remove_project(project.id))

    def test_scanner_ignores_generated_directories_and_symlinks(self):
        source = self.project / "main.rs"
        source.write_text("fn main() {}\n", encoding="utf-8")
        generated = self.project / "target"
        generated.mkdir()
        artifact = generated / "binary"
        artifact.write_bytes(b"generated")
        outside = self.root / "outside.txt"
        outside.write_text("private", encoding="utf-8")
        (self.project / "outside-link").symlink_to(outside)
        project = BackupProject("a" * 32, "Rust", str(self.project), excludes=DEFAULT_EXCLUDES)

        first = scan_project(project)
        self.assertTrue(first.complete)
        self.assertEqual(first.file_count, 1)
        self.assertEqual(first.total_bytes, source.stat().st_size)

        artifact.write_bytes(b"different generated content")
        self.assertEqual(scan_project(project).digest, first.digest)
        source.write_text("fn main() { println!(\"changed\"); }\n", encoding="utf-8")
        self.assertNotEqual(scan_project(project).digest, first.digest)

    def test_due_snooze_notification_and_verified_lifecycle(self):
        source = self.project / "chapter.odt"
        source.write_bytes(b"first")
        project = self.registry.add("Licenta", str(self.project), interval_minutes=15, quiet_minutes=10)

        initial = self.monitor.refresh(now=1_000)[0]
        self.assertEqual(initial.state, "unprotected")
        self.assertFalse(initial.due)
        due = self.monitor.refresh(now=1_900)[0]
        self.assertEqual(due.state, "due")
        self.assertTrue(due.due)
        self.assertEqual([item.id for item in self.monitor.notification_candidates([due], now=1_900)], [project.id])
        self.monitor.mark_notified([project.id], now=1_900)
        self.assertEqual(self.monitor.notification_candidates([due], now=1_901), [])

        self.monitor.snooze(project.id, 60, now=1_900)
        snoozed = self.monitor.refresh(now=2_000)[0]
        self.assertEqual(snoozed.state, "snoozed")
        self.assertFalse(snoozed.due)

        self.monitor.mark_verified(project.id, now=2_100)
        verified = self.monitor.refresh(now=2_101)[0]
        self.assertEqual(verified.state, "verified")
        self.assertFalse(verified.due)
        self.assertEqual(verified.last_verified_at, 2_100)

        source.write_bytes(b"second version")
        changed = self.monitor.refresh(now=2_200)[0]
        self.assertEqual(changed.state, "changed")
        self.assertFalse(changed.due)
        due_again = self.monitor.refresh(now=3_100)[0]
        self.assertEqual(due_again.state, "due")
        self.assertTrue(due_again.due)

        state_text = self.state_path.read_text(encoding="utf-8")
        self.assertNotIn("chapter.odt", state_text)
        self.assertEqual(stat.S_IMODE(self.state_path.stat().st_mode), 0o600)

    def test_schema_rejects_unknown_fields_and_modes(self):
        self.registry_path.parent.mkdir(mode=0o700)
        payload = {
            "version": 1,
            "projects": [
                {
                    "id": "b" * 32,
                    "name": "Fixture",
                    "path": str(self.project),
                    "interval_minutes": 120,
                    "quiet_minutes": 10,
                    "mode": "automatic",
                    "excludes": [],
                    "unexpected": True,
                }
            ],
        }
        self.registry_path.write_text(json.dumps(payload), encoding="utf-8")
        os.chmod(self.registry_path, 0o600)
        with self.assertRaises(ProjectError):
            self.registry.load()

        self.registry_path.unlink()
        self.state_path.parent.mkdir(mode=0o700)
        self.state_path.write_text(
            json.dumps({"version": 1, "projects": {"c" * 32: {"last_verified_at": True}}}),
            encoding="utf-8",
        )
        os.chmod(self.state_path, 0o600)
        with self.assertRaises(ProjectError):
            self.monitor.refresh(now=1_000)


if __name__ == "__main__":
    unittest.main()
