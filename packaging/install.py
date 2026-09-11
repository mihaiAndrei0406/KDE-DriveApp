"""Review/install only this application's user-level desktop integration."""
import argparse
import os
from pathlib import Path
import subprocess
import tempfile

MARKER = "# Managed by Hetzner Drive Manager\n"
PROJECT = Path(__file__).resolve().parents[1]


def integration_files(project, home, autostart=False, ssh_key=None, sftp_host=None, sftp_user=None):
    project = project.resolve()
    home = home.resolve()
    ssh_key = ssh_key or home / ".ssh/hetzner_storagebox"
    if any(character in str(project) for character in '\n\r\x00"\\%$`'):
        raise ValueError("Unsupported characters in project path")
    if (
        not ssh_key.is_absolute()
        or ssh_key.parent != home / ".ssh"
        or not all(
            character.isascii() and (character.isalnum() or character in "/._-")
            for character in str(ssh_key)
        )
    ):
        raise ValueError(
            "SSH key must be a direct child of the user's .ssh directory and use only safe characters"
        )
    if not sftp_host or not sftp_user:
        raise ValueError("Storage Box host and user are required")
    host_account = sftp_host.removesuffix(".your-storagebox.de")
    main_user = sftp_user.split("-", 1)[0]
    if (
        host_account == sftp_host
        or host_account != main_user
        or not host_account.startswith("u")
        or not host_account[1:].isdigit()
        or not all(character.isascii() and (character.isalnum() or character in "-_") for character in sftp_user)
    ):
        raise ValueError("Invalid Storage Box host/user relationship")
    mappings = [
        ("systemd/hetzner-drive.service.in", home / ".config/systemd/user/hetzner-drive.service"),
        ("desktop/ro.mihai.HetznerDrive.desktop.in", home / ".local/share/applications/ro.mihai.HetznerDrive.desktop"),
        ("dbus/ro.mihai.HetznerDrive1.service.in", home / ".local/share/dbus-1/services/ro.mihai.HetznerDrive1.service"),
        ("desktop/ro.mihai.HetznerDrive-autostart.desktop.in", home / ".config/autostart/ro.mihai.HetznerDrive.desktop"),
    ]
    result = {}
    for source, destination in mappings:
        text = (project / "packaging" / source).read_text(encoding="utf-8")
        text = text.replace("@PROJECT_ROOT@", str(project))
        text = text.replace("@SSH_KEY@", str(ssh_key))
        text = text.replace("@SFTP_HOST@", sftp_host)
        text = text.replace("@SFTP_USER@", sftp_user)
        if "autostart" in source:
            text = text.replace("Hidden=true", f"Hidden={str(not autostart).lower()}")
        result[destination] = MARKER + text
    return result


def verify_destination(path):
    for part in (path, *path.parents):
        if part.is_symlink():
            raise ValueError(f"Symlink destination refused: {part}")
    if path.exists():
        if not path.is_file() or path.stat().st_uid != os.geteuid():
            raise ValueError(f"File ownership/type mismatch: {path}")
        if not path.read_text(encoding="utf-8").startswith(MARKER):
            raise ValueError(f"Existing file is not managed by this application: {path}")


def install_files(files):
    for destination in files:
        verify_destination(destination)
    for destination, content in files.items():
        destination.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        verify_destination(destination)
        temporary = None
        try:
            with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", dir=destination.parent, delete=False) as handle:
                temporary = Path(handle.name)
                os.fchmod(handle.fileno(), 0o600)
                handle.write(content)
                handle.flush()
                os.fsync(handle.fileno())
            os.replace(temporary, destination)
        finally:
            if temporary and temporary.exists():
                temporary.unlink()


def remove_files(files):
    for destination in files:
        verify_destination(destination)
    for destination in files:
        if destination.exists():
            destination.unlink()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("plan", "install", "uninstall"), nargs="?", default="plan")
    parser.add_argument("--autostart", action="store_true", help="Enable tray startup at graphical login")
    parser.add_argument(
        "--ssh-key",
        type=Path,
        help="Private SSH key path expected in rclone.conf (default: ~/.ssh/hetzner_storagebox)",
    )
    parser.add_argument(
        "--sftp-host",
        help="Storage Box host, for example u12345.your-storagebox.de",
    )
    parser.add_argument(
        "--sftp-user",
        help="Storage Box user, for example u12345 or u12345-backups",
    )
    parser.add_argument("--apply", action="store_true", help="Apply the listed user-level file changes")
    args = parser.parse_args()
    if os.geteuid() == 0:
        parser.error("Run as the normal desktop user, never root")
    if args.action != "uninstall" and (not args.sftp_host or not args.sftp_user):
        parser.error("--sftp-host and --sftp-user are required for plan and install")
    files = integration_files(
        PROJECT,
        Path.home(),
        args.autostart,
        args.ssh_key,
        args.sftp_host or "u0.your-storagebox.de",
        args.sftp_user or "u0",
    )
    action = "Remove" if args.action == "uninstall" else "Write"
    for destination, content in files.items():
        verify_destination(destination)
        print(f"{action}: {destination}")
        if args.action == "plan":
            print(content, end="\n" if content.endswith("\n") else "\n\n")
    if not args.apply or args.action == "plan":
        print("Preview only. No files or services changed.")
        return 0
    if args.action == "install":
        for binary in (PROJECT / "target/release/hetzner-drive-core", PROJECT / ".venv/bin/hetzner-drive"):
            if not binary.is_file() or not os.access(binary, os.X_OK):
                parser.error(f"Build/install the application first: {binary}")
        install_files(files)
    else:
        # Removing activation files does not stop a running service or mount.
        remove_files(files)
    result = subprocess.run(["/usr/bin/systemctl", "--user", "daemon-reload"], check=False, timeout=15)
    print("User integration updated. No running service or mount was stopped.")
    if result.returncode:
        print("The user systemd manager did not reload; retry daemon-reload in the graphical session.")
    return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())
