# User-service packaging

Templates are not installed or enabled automatically. `packaging/install.py plan`
prints the four destination paths and their complete rendered contents, replacing
`@PROJECT_ROOT@` with this checkout's absolute path. `install --apply` installs
user-level systemd, D-Bus activation, launcher and disabled autostart files;
`--autostart` explicitly enables tray startup. The installer reloads user systemd
configuration but does not start or stop the service or any mount. Build the
release binary first; the checkout and venv must remain at the installed location.
The service receives the expected Storage Box host/user and SSH private-key path
through `HETZNER_DRIVE_SFTP_HOST`, `HETZNER_DRIVE_SFTP_USER`, and
`HETZNER_DRIVE_SSH_KEY`. Installer `--sftp-host`/`--sftp-user` are required and
must describe the same account; `--ssh-key` renders a constrained absolute path
directly below `$HOME/.ssh`, defaulting to `$HOME/.ssh/hetzner_storagebox`.

| Option | Purpose and compatibility |
| --- | --- |
| UMask=0077 | Restrict new files/directories; never chmod existing setup |
| LimitCORE=0 | Prevent regular core files; secret retention also uses mlock, MADV_DONTDUMP and disabled process dumps |
| TimeoutStopSec=5min | Give the controller time to wait for queued VFS uploads and perform verified unmount |
| KeyringMode=private | Give the service a private kernel session keyring; SSH still uses the explicit agent socket |
| NoNewPrivileges (not enabled) | The distribution fusermount3 helper may require its existing privilege transition for unprivileged FUSE; enabling this before a live compatibility test can break mount/unmount |
| LockPersonality (not enabled) | Prevented the existing setuid FUSE helper from completing a synthetic mount |
| RestrictRealtime (not enabled) | Also prevented the setuid FUSE helper from completing a synthetic mount in this user manager |
| PrivateTmp (not enabled) | Could hide an SSH agent socket under /tmp |
| ProtectSystem=strict (not enabled) | Requires tested writable paths and user namespace support; mount namespaces can also hide a future FUSE mount from Dolphin |
| RestrictSUIDSGID (not enabled) | Broke the descriptor-pinned public-key fingerprint check in the real user manager |

The template requires validation under the user's actual systemd manager. Offline
tests exercise an isolated session bus, not KDE startup, FUSE or systemd sandboxing.
No linger, root service, sudo command or system-wide installation is provided.
The existing keychain --systemd flow supplies SSH_AUTH_SOCK; the controller does
not source shell files, run keychain or alter the manager's environment.

The core still refuses root and executes only the fixed system fusermount3 path.
It does not install or modify a setuid binary. Removing `NoNewPrivileges=yes`
from the template is a compatibility choice, not permission for arbitrary child
processes; the final audit must test whether a tighter working unit is possible.

The non-namespacing restrictions above were selected so an application-owned
FUSE mount remains visible to the KDE session. Options such as `ProtectSystem`,
`PrivateMounts`, `PrivateTmp` and `ProtectHome` remain disabled until an explicit
test proves they preserve both mount propagation and the keychain socket.

`RestrictSUIDSGID=yes` was tested in the real user manager on 2026-09-10. It
caused the descriptor-pinned public-key fingerprint check to fail, while an
otherwise identical transient unit without it succeeded. The option remains
disabled. `MemoryDenyWriteExecute`, `RestrictAddressFamilies` and
`SystemCallArchitectures` each independently caused a synthetic FUSE mount to
fail because the existing setuid `fusermount3` helper could not complete its
privilege transition. These options remain disabled based on direct tests.
`LockPersonality` and `RestrictRealtime` failed the same FUSE test independently
and were removed. `KeyringMode=private` passed the synthetic FUSE test and
remains enabled.

Inspect the preview before applying installation. The application launches the
core through session D-Bus when required. Closing the GUI does not lock or stop
the core; use the explicit configuration lock action to release the session secret.
`packaging/install.py uninstall --apply` removes only managed integration files.
Integration was installed in the live user profile on 2026-09-10 with autostart
disabled; D-Bus activation is available without enabling the unit at boot or
mounting automatically.
