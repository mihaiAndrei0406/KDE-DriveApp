import os
import unittest
from unittest import mock

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from PySide6.QtCore import QObject, Signal, Qt
from PySide6.QtGui import QColor, QPalette
from PySide6.QtWidgets import QApplication, QMessageBox
from hetzner_drive.app import DARK_THEME, LIGHT_THEME, DriveWindow, STATE_LABELS, theme_for_palette
from hetzner_drive.client import decode_snapshot_entries, decode_snapshot_history, decode_status
from hetzner_drive.projects import ProjectStatus


class FakeClient(QObject):
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
    snapshot_entries_received = Signal(str, str, bool, bool, object, str)

    def __init__(self):
        super().__init__()
        self.history_requests = []
        self.entry_requests = []
        self.restore_requests = []
        self.selective_restore_requests = []

    def refresh(self):
        pass

    def check_ssh(self):
        pass

    def get_logs(self):
        pass

    def unlock(self):
        pass

    def lock(self):
        pass

    def mount(self):
        pass

    def unmount(self):
        pass

    def check_connection(self):
        pass

    def health_check(self):
        pass

    def open_drive(self):
        pass

    def get_storage(self):
        pass

    def initialize_backup_repository(self):
        pass

    def unlock_backup_repository(self):
        pass

    def lock_backup_repository(self):
        pass

    def start_project_backup(self, _project_id):
        pass

    def get_project_snapshots(self, project_id):
        self.history_requests.append(project_id)

    def get_snapshot_entries(self, project_id, snapshot_id):
        self.entry_requests.append((project_id, snapshot_id))

    def start_backup_restore(self, project_id, snapshot_id):
        self.restore_requests.append((project_id, snapshot_id))

    def start_selective_restore(self, project_id, snapshot_id, selected_path):
        self.selective_restore_requests.append((project_id, snapshot_id, selected_path))


class EmptyProjectMonitor:
    def projects(self):
        return []

    def refresh(self):
        return []

    def notification_candidates(self, _statuses):
        return []


class GuiTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    def setUp(self):
        self.window = DriveWindow(FakeClient(), EmptyProjectMonitor())
        self.window.timer.stop()
        self.window.show()
        self.app.processEvents()

    def tearDown(self):
        self.window.backup_timer.stop()
        if self.window.tray:
            self.window.tray.hide()
        self.window.deleteLater()
        self.app.processEvents()

    def status(self, state="Locked"):
        return decode_status([state, "unmounted", "loaded", "present_not_unlocked",
            "rclone v1.75.1", "/home/example/HetznerDrive", 1024, True, "demo_data"])

    def test_main_states_and_unavailable_storage(self):
        for state, label in STATE_LABELS.items():
            self.window.apply_status(self.status(state))
            self.assertEqual(self.window.state_label.text(), label)
            self.assertTrue(self.window.state_label.property("stateTone"))
            self.assertIn("Indisponibil", self.window.values["storage"].text())

    def test_midnight_and_lightning_palettes_follow_kde(self):
        palette = QPalette()
        palette.setColor(QPalette.ColorRole.Window, QColor("#100B18"))
        self.assertEqual(theme_for_palette(palette), DARK_THEME)
        palette.setColor(QPalette.ColorRole.Window, QColor("#F8F7FC"))
        self.assertEqual(theme_for_palette(palette), LIGHT_THEME)

    def test_english_interface_translates_static_and_runtime_status(self):
        window = DriveWindow(FakeClient(), EmptyProjectMonitor(), language="en")
        window.timer.stop()
        window.backup_timer.stop()
        window.show()
        self.app.processEvents()
        try:
            self.assertEqual(window.refresh_button.text(), "Refresh")
            self.assertEqual(window.tabs.tabText(window.backup_tab_index), "Backups")
            self.assertEqual(window.restore_selection_button.text(), "Restore selection")
            window.apply_status(self.status("Ready"))
            self.assertEqual(window.state_label.text(), "Ready")
            self.assertIn("authentication not verified", window.values["storage"].text())
            status = ProjectStatus(
                id="a" * 32,
                name="Thesis",
                path="/home/example/Projects/Thesis",
                state="due",
                file_count=3,
                total_bytes=1024,
                dirty_since=1,
                last_change=1,
                last_verified_at=0,
                snoozed_until=0,
                reason="ok",
                due=True,
            )
            window.apply_project_statuses([status])
            self.assertEqual(window.project_tree.topLevelItem(0).text(1), "Backup recommended")
            self.assertIn("monitored projects", window.backup_summary.text())
            window.apply_storage(True, 1000, 250, 750, "ok")
            self.assertIn("Free", window.values["storage"].text())
            window.apply_mount_activity(True, 2, 1, 0, False, "ok")
            self.assertIn("Queued: 2", window.values["transfers"].text())
            window.show_error(
                "Serviciul nu raspunde. Verifica daca este pornit in sesiunea curenta."
            )
            self.assertEqual(window.state_label.text(), "Service unavailable")
            self.assertIn("not responding", window.error_label.text())
        finally:
            if window.tray:
                window.tray.hide()
            window.deleteLater()
            self.app.processEvents()

    def test_backup_dashboard_shows_due_projects_without_enabling_upload(self):
        status = ProjectStatus(
            id="a" * 32,
            name="Licenta",
            path="/home/example/Projects/Thesis",
            state="due",
            file_count=34,
            total_bytes=128 * 1024 * 1024,
            dirty_since=1,
            last_change=1,
            last_verified_at=0,
            snoozed_until=0,
            reason="ok",
            due=True,
        )
        self.window.apply_project_statuses([status])
        self.assertEqual(self.window.project_tree.topLevelItemCount(), 1)
        self.assertIn("Backup recomandat", self.window.project_tree.topLevelItem(0).text(1))
        self.assertIn("(1)", self.window.tabs.tabText(self.window.backup_tab_index))
        self.window.project_tree.setCurrentItem(self.window.project_tree.topLevelItem(0))
        self.app.processEvents()
        self.assertTrue(self.window.snooze_project_button.isEnabled())
        self.assertTrue(self.window.remove_project_button.isEnabled())
        self.assertFalse(self.window.backup_now_button.isEnabled())

    def test_restic_status_enables_only_audited_backup_actions(self):
        status = ProjectStatus(
            id="a" * 32, name="Licenta", path="/tmp/Licenta", state="due",
            file_count=3, total_bytes=1024, dirty_since=1, last_change=1,
            last_verified_at=0, snoozed_until=0, reason="ok", due=True,
        )
        self.window.apply_status(self.status("Ready") | {"config": "unlocked"})
        self.window.apply_project_statuses([status])
        self.window.project_tree.setCurrentItem(self.window.project_tree.topLevelItem(0))
        self.window.apply_backup_status(
            True, True, True, False, "ready", "", 0, 0, 0, 0, "", "", ""
        )
        self.app.processEvents()
        self.assertTrue(self.window.backup_now_button.isEnabled())
        self.assertFalse(self.window.restore_backup_button.isEnabled())
        self.assertFalse(self.window.initialize_backup_button.isEnabled())
        self.assertTrue(self.window.lock_backup_button.isEnabled())
        self.window.apply_backup_status(
            True, True, True, False, "backup_complete", "a" * 32,
            1024, 1024, 3, 3, "b" * 64, "", "",
        )
        self.assertFalse(self.window.restore_backup_button.isEnabled())
        self.window.apply_snapshot_history(
            "a" * 32,
            True,
            False,
            [{
                "id": "b" * 64,
                "created_at": "2026-09-10T12:00:00Z",
                "files": 3,
                "bytes": 1024,
            }],
            "ok",
        )
        self.assertTrue(self.window.restore_backup_button.isEnabled())
        self.assertEqual(self.window.client.entry_requests, [("a" * 32, "b" * 64)])
        self.window.restore_selected_backup()
        self.assertEqual(self.window.client.restore_requests, [("a" * 32, "b" * 64)])
        self.window.apply_snapshot_entries(
            "a" * 32,
            "b" * 64,
            True,
            False,
            [
                {"path": "src", "kind": "dir", "size": 0},
                {"path": "src/main.rs", "kind": "file", "size": 42},
            ],
            "ok",
        )
        directory = self.window.snapshot_contents_tree.topLevelItem(0)
        self.assertEqual(directory.data(0, Qt.ItemDataRole.UserRole), "src")
        self.assertEqual(directory.childCount(), 1)
        self.window.snapshot_contents_tree.setCurrentItem(directory.child(0))
        self.app.processEvents()
        self.assertTrue(self.window.restore_selection_button.isEnabled())
        with mock.patch.object(
            QMessageBox,
            "warning",
            return_value=QMessageBox.StandardButton.Yes,
        ):
            self.window.restore_selected_entry()
        self.assertEqual(
            self.window.client.selective_restore_requests,
            [("a" * 32, "b" * 64, "src/main.rs")],
        )

    def test_snapshot_history_schema_requires_aligned_safe_entries(self):
        decoded = decode_snapshot_history([
            True,
            False,
            ["a" * 64],
            ["2026-09-10T12:00:00Z"],
            [3],
            [1024],
            "ok",
        ])
        self.assertEqual(decoded[2][0]["bytes"], 1024)
        with self.assertRaises(ValueError):
            decode_snapshot_history([
                True, False, ["../unsafe"], ["2026-09-10T12:00:00Z"], [3], [1024], "ok"
            ])
        with self.assertRaises(ValueError):
            decode_snapshot_history([
                True, False, ["a" * 64], [], [3], [1024], "ok"
            ])

    def test_snapshot_entry_schema_rejects_arbitrary_or_unaligned_paths(self):
        decoded = decode_snapshot_entries([
            True,
            False,
            ["src", "src/main.rs"],
            ["dir", "file"],
            [0, 42],
            "ok",
        ])
        self.assertEqual(decoded[2][1]["path"], "src/main.rs")
        for path in ("../escape", "/absolute", "src//file", "line\nbreak"):
            with self.assertRaises(ValueError):
                decode_snapshot_entries([True, False, [path], ["file"], [1], "ok"])
        with self.assertRaises(ValueError):
            decode_snapshot_entries([True, False, ["safe"], [], [1], "ok"])

    def test_disconnect_invalidates_stale_status(self):
        self.window.apply_status(self.status("Mounted"))
        self.window.show_error("Service unavailable")
        self.assertEqual(self.window.state_label.text(), "Serviciu indisponibil")
        self.assertTrue(all(w.text() == "Necunoscut" for w in self.window.values.values()))

    def test_log_refresh_preserves_demo_indicator(self):
        self.window.apply_status(self.status())
        self.window.set_busy(True)
        self.window.set_busy(False)
        self.window.apply_logs(["INFO Upload completed"])
        self.assertEqual(self.window.mode_label.text(), "Date simulate")
        self.assertEqual(self.window.statusBar().currentMessage(), "Evenimente actualizate")

    def test_busy_controls_and_incomplete_cache(self):
        self.window.set_busy(True)
        self.assertFalse(self.window.refresh_button.isEnabled())
        self.assertFalse(self.window.ssh_button.isEnabled())
        self.window.set_busy(False)
        self.assertTrue(self.window.refresh_button.isEnabled())
        status = self.status()
        status["cache_complete"] = False
        self.window.apply_status(status)
        self.assertIn("incompleta", self.window.values["cache"].text())

    def test_status_schema_rejects_invalid_types(self):
        with self.assertRaises(ValueError):
            decode_status(["Mounted"])
        values = list(self.status().values())
        values[6] = "a lot"
        with self.assertRaises(ValueError):
            decode_status(values)

    def test_unlock_controls_and_large_storage_values(self):
        self.window.apply_status(self.status())
        self.assertTrue(self.window.operation_buttons["unlock"].isEnabled())
        self.assertFalse(self.window.operation_buttons["connection"].isEnabled())
        status = self.status("Ready")
        status["config"] = "unlocked"
        self.window.apply_status(status)
        self.assertFalse(self.window.operation_buttons["unlock"].isEnabled())
        self.assertTrue(self.window.operation_buttons["connection"].isEnabled())
        self.assertTrue(self.window.operation_buttons["mount"].isEnabled())
        self.assertFalse(self.window.operation_buttons["unmount"].isEnabled())
        self.window.apply_storage(True, 1_000_000_000_000, 25_000_000_000, 975_000_000_000, "ok")
        self.assertIn("GiB", self.window.values["storage"].text())
        self.assertNotIn("Indisponibil", self.window.values["storage"].text())
        self.assertEqual(self.window.storage_progress.value(), 25)

    def test_small_window_layout(self):
        self.window.resize(540, 480)
        status = self.status("Error")
        status["config"] = "unsafe_permissions"
        self.window.apply_status(status)
        self.app.processEvents()
        for widget in self.window.values.values():
            self.assertGreater(widget.width(), 0)
            self.assertGreaterEqual(widget.height(), widget.heightForWidth(widget.width()))

    def test_connection_failure_and_operation_history_are_visible(self):
        status = self.status("Degraded")
        status["config"] = "unlocked"
        status["diagnostic"] = "demo_data,remote_check_failed"
        self.window.apply_status(status)
        self.assertEqual(self.window.state_label.text(), "Verificarea conexiunii a esuat")
        self.assertEqual(self.window.mode_label.text(), "Date simulate")
        self.assertIn("verifica din nou conexiunea", self.window.values["storage"].text())
        self.window.apply_operation_details(True, "health_check", "rclone_timeout", False)
        self.assertEqual(self.window.values["last_operation"].text(), "Diagnostic (in curs)")
        self.assertIn("nu a raspuns", self.window.values["last_error"].text())
        self.assertFalse(self.window.operation_buttons["health"].isEnabled())
        self.window.apply_operation_details(False, "health_check", "rclone_timeout", False)
        self.assertTrue(self.window.operation_buttons["health"].isEnabled())
        mounted = self.status("Mounted")
        mounted["config"] = "unlocked"
        mounted["mount"] = "mounted"
        self.window.apply_status(mounted)
        self.window.apply_operation_details(False, "mount_drive", "", True)
        self.assertTrue(self.window.operation_buttons["unmount"].isEnabled())
        self.window.apply_operation_details(False, "mount_drive", "", False)
        self.assertFalse(self.window.operation_buttons["unmount"].isEnabled())
        self.window.apply_mount_activity(True, 2, 1, 0, False, "ok")
        self.assertIn("In asteptare: 2", self.window.values["transfers"].text())
        self.window.apply_mount_activity(True, 0, 0, 1, False, "ok")
        self.assertIn("Atentie", self.window.values["transfers"].text())
        self.window.apply_mount_activity(False, 0, 0, 0, False, "mount_not_owned")
        self.assertIn("pornit extern", self.window.values["transfers"].text())
        self.window.show_error("Service unavailable")
        self.assertEqual(self.window.operation_details, "")
        self.assertEqual(self.window.values["last_error"].text(), "Necunoscut")


if __name__ == "__main__":
    unittest.main()
