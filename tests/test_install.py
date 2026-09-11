import importlib.util
import io
import os
from pathlib import Path
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from unittest import mock

PROJECT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("drive_installer", PROJECT / "packaging/install.py")
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


class InstallerTests(unittest.TestCase):
    POLICY = {
        "sftp_host": "u12345.your-storagebox.de",
        "sftp_user": "u12345",
    }

    def test_plan_and_install_are_confined_to_expected_files(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            files = installer.integration_files(PROJECT, root, autostart=True, **self.POLICY)
            self.assertEqual(len(files), 4)
            self.assertFalse(any(root.iterdir()))
            installer.install_files(files)
            for path, text in files.items():
                self.assertEqual(path.read_text(), text)
                self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            self.assertIn("Hidden=false", (root / ".config/autostart/ro.mihai.HetznerDrive.desktop").read_text())
            service = (root / ".config/systemd/user/hetzner-drive.service").read_text()
            self.assertIn(f'HETZNER_DRIVE_SSH_KEY={root / ".ssh/hetzner_storagebox"}', service)
            installer.remove_files(files)
            self.assertTrue(all(not path.exists() for path in files))

    def test_ssh_key_override_is_confined_to_home_ssh(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            custom = root / ".ssh/storagebox_custom"
            files = installer.integration_files(PROJECT, root, ssh_key=custom, **self.POLICY)
            service = files[root / ".config/systemd/user/hetzner-drive.service"]
            self.assertIn(f"HETZNER_DRIVE_SSH_KEY={custom}", service)
            for unsafe in (Path("relative"), root / "outside-key", root / ".ssh/space key"):
                with self.assertRaises(ValueError):
                    installer.integration_files(PROJECT, root, ssh_key=unsafe, **self.POLICY)

    def test_storagebox_identity_is_required_and_validated(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for host, user in (
                (None, None),
                ("example.invalid", "u12345"),
                ("u12345.your-storagebox.de", "u54321"),
                ("u12345.your-storagebox.de", "u12345;proxy"),
            ):
                with self.assertRaises(ValueError):
                    installer.integration_files(
                        PROJECT,
                        root,
                        sftp_host=host,
                        sftp_user=user,
                    )

    def test_uninstall_preview_does_not_require_storagebox_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with (
                mock.patch.object(installer.Path, "home", return_value=root),
                mock.patch.object(sys, "argv", ["install.py", "uninstall"]),
                redirect_stdout(io.StringIO()) as output,
            ):
                self.assertEqual(installer.main(), 0)
            self.assertIn("Preview only. No files or services changed.", output.getvalue())
            self.assertFalse(any(root.iterdir()))

    def test_refuses_unrelated_file_before_any_changes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            owned, foreign = root / "owned", root / "foreign"
            foreign.write_text("User-owned content")
            with self.assertRaises(ValueError):
                installer.install_files({owned: installer.MARKER + "new", foreign: installer.MARKER + "replacement"})
            self.assertFalse(owned.exists())
            self.assertEqual(foreign.read_text(), "User-owned content")

    def test_refuses_symlink_parent(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / "target"
            target.mkdir()
            os.symlink(target, root / "link")
            with self.assertRaises(ValueError):
                installer.install_files({root / "link/file": installer.MARKER + "new"})
