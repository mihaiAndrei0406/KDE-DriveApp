# Hetzner Drive Manager

Native Qt desktop controller for the existing encrypted Hetzner drive.
The application provides local diagnostics, session unlocking via pinentry,
authenticated connection/storage checks and a native Qt GUI through a typed
session D-Bus API. Controlled mount/unmount and authenticated VFS activity
observation are implemented; the idle live KDE/FUSE lifecycle passed on
2026-09-10 without writing remote files.
A local project backup assistant detects metadata changes and recommends backups.
Its phase-5b restic engine creates user-confirmed encrypted snapshots in a fixed
append-only disposable repository and supports verified restore into a new local
directory. The GUI reads a bounded per-project snapshot history and restores the
explicitly selected version, including after a service restart and fresh unlock.
Retention, pruning and unattended upload remain disabled.
Progress is recorded in `docs/WORK_LOG.md`.

The current dashboard keeps failed authenticated checks visible until a successful
remote recheck or session lock. It also shows the last operation and historical
last error. Its Purple Midnight / Lightning Purple styling follows KDE's active
light or dark palette. Automated verification results are in `docs/VERIFICATION.md`.
The interface supports English and Romanian. Public-facing technical documentation
is maintained in English; the Romanian end-user workflow and password glossary
remain available in `docs/GHID_UTILIZARE.md`.

## Development setup

Python 3.13, Rust/Cargo, D-Bus and a KDE/Qt runtime are required. Python dependencies
are isolated in `.venv`, with exact versions and artifact hashes in
`gui/requirements.lock`; Rust dependencies are fixed by `Cargo.lock`.

```bash
python3 -m venv .venv
.venv/bin/python -m pip install uv==0.12.11
.venv/bin/uv pip install --python .venv/bin/python --require-hashes --no-deps -r gui/requirements.lock
.venv/bin/uv pip install --python .venv/bin/python --no-deps --no-build-isolation -e gui
cargo build --locked
```

Build tooling is included in the Python lock. No activation is required; use
the explicit `.venv/bin/` paths. No Python dependency is installed globally.

## Run

In two terminals from the project root, as your normal user:

```bash
./target/debug/hetzner-drive-core
```

```bash
.venv/bin/hetzner-drive
```

The GUI follows the desktop locale (`ro*` selects Romanian; every other locale
selects English). Override it with `--language ro`, `--language en`, or the
`HETZNER_DRIVE_LANGUAGE` environment variable. Secure pinentry prompts are
bilingual because the backend may be activated independently through D-Bus.

The core derives all user paths from `$HOME`. The expected private SSH key defaults
to `$HOME/.ssh/hetzner_storagebox`; set `HETZNER_DRIVE_SSH_KEY` to an absolute key
path directly below `$HOME/.ssh` when the encrypted rclone configuration uses a
different filename. The matching `.pub` file is used for agent fingerprint checks.

The core reads mountinfo, public-key metadata/content, config metadata, cache
metadata and sanitized recent logs. It invokes fixed rclone operations,
ssh-keygen on the public key, ssh-add and fusermount3. It never opens the private
key or exposes decrypted config content. Explicit unlocking captures the encrypted
config in a sealed memory file and validates rclone's redacted output. Existing
SSH agent access must be present in its environment. Passwords are entered only
in pinentry, never in the GUI, command arguments or D-Bus messages.

The restic repository uses a separate user-chosen password. Initialization asks
for it twice; later sessions ask once and validate it by listing repository
metadata. The password is kept only in locked process memory and is discarded on
repository lock, configuration lock or service exit. It is never the rclone
configuration password, crypt password or crypt salt.

Mounts started by the application run as supervised foreground children. Their
rclone RC endpoint exists only on an authenticated socket inside a private runtime
directory; no TCP port, web GUI or generic RC method is exposed. External mounts
are observed but never adopted or unmounted. Safe unmount checks VFS uploads and
errors twice, refuses force/lazy unmount and preserves the cache.

Use `./target/debug/hetzner-drive-core --demo` for a service returning synthetic
status without inspecting the setup; the GUI visibly marks these data as simulated.
Only one core may own the bus name. `--demo --inspect` prints the synthetic snapshot
without a bus. `--inspect` without `--demo` performs actual local diagnostics.

The window closes into the tray if a tray is available. Tray Exit closes the GUI
without unmounting anything. The internal RC protocol is HTTP over a private Unix
socket only; there is no TCP listener, browser UI or development web server.

## Project backup assistant

The `Backupuri` page lets the user explicitly register up to 32 non-overlapping
local project directories. Every five minutes a background worker scans at most
100,000 entries and 64 directory levels. It hashes names, sizes, modification
times and modes, but never opens file contents and never follows symlinks. Common
generated paths such as `target`, `node_modules`, `.venv`, `build`, `dist`, cache
directories and temporary editor files are excluded by default.

The default policy recommends a backup after two hours of observed changes and
ten quiet minutes; the registration dialog allows 1–168 hours. Recommendations
remain visible in the dashboard and may also appear through the KDE tray. They
can be snoozed for one hour. Monitoring runs only while the GUI/tray process is
running. A per-user runtime lock prevents duplicate GUI monitors; reopening the
launcher brings the existing window forward. Desktop autostart remains opt-in
and disabled in the installed setup.

The project registry is stored atomically as a 0600 file below
`$XDG_CONFIG_HOME/hetzner-drive/`; activity state is a separate 0600 file below
`$XDG_STATE_HOME/hetzner-drive/` and contains digests/counters, not filenames.
Removing a project only removes it from this local registry. The `Backup acum`
button is enabled only for a safe selected project after both the encrypted rclone
configuration and restic repository are unlocked. Rust independently resolves the
project ID through the private registry and revalidates its owner and symlink-free
root. D-Bus cannot supply an arbitrary path, remote or command. Storage Box host
and user values come from the explicit private service policy and must pass the
strict Hetzner hostname/user relationship. Fixed command overrides anchor the
sealed rclone snapshot to that policy alongside port, agent-key, and host-pinning
checks; no account identifier is embedded in source.

The repository is fixed at
`hetzner-crypt:HetznerDrive-Backup-Disposable/restic-v1`. The rclone transport is
fixed to append-only `serve restic`; the application exposes no forget, prune,
delete or arbitrary restic operation. Each backup and restore needs a separate
explicit pinentry confirmation. One job may run at a time. A successful snapshot is followed
by `restic check --with-cache`; the GUI marks the scanned digest verified only if
the local project did not change during the job. The history view exposes at most
256 entries matching the exact project path and application/project tags. Restore
accepts only a full ID selected from that validated list, uses restic verification
and an application-created 0700 directory below `$HOME/HetznerDrive-Restores`,
never the original project path. Phase 5c retention and optional idle automation
remain outside the enabled scope.

## Desktop integration

Build a release first (`cargo build --release --locked`). The installer requires
the public Storage Box host/user policy on every preview/install and validates
their relationship. For example:

```bash
.venv/bin/python packaging/install.py plan \
  --sftp-host u12345.your-storagebox.de --sftp-user u12345
.venv/bin/python packaging/install.py install --apply \
  --sftp-host u12345.your-storagebox.de --sftp-user u12345
```

If the SSH key has a non-default filename, include its absolute path with
`--ssh-key` in both commands. Installation is explicit; add
`--autostart` to enable tray startup after graphical login. The default leaves
autostart disabled. No existing Hetzner script/configuration is modified.

The installer adds application, session D-Bus and user-systemd activation files.
It does not start/stop a running service or mount. Removing integration uses
`.venv/bin/python packaging/install.py uninstall --apply`, and preserves all data.
This checkout and its venv must remain at the installed path. Re-run installation
after moving the checkout. The user-level integration was installed and validated
on 2026-09-10 with autostart disabled; D-Bus starts the backend on demand.

## Verification

```bash
cargo test --locked
QT_QPA_PLATFORM=offscreen .venv/bin/python -m unittest discover -s tests -v
dbus-run-session --config-file=tests/dbus-session.conf -- \
  .venv/bin/python tests/dbus_smoke.py
```

Automated tests use local fixtures and a demo service. The D-Bus smoke test uses a
private configuration without activation directories, so it cannot start an
installed release service. On 2026-09-10 an explicit live acceptance created one
151-byte snapshot in the fixed disposable Hetzner repository, completed repository
checking, restored it with verification into a new local directory and matched the
original SHA-256. No remote deletion was attempted or enabled. A manual checklist
is in `tests/MANUAL.md`. Packaging updates are not applied automatically after
source changes; rerun the explicit installer after review.

See `docs/USER_GUIDE.md`, `docs/GHID_UTILIZARE.md`, `docs/ARCHITECTURE.md`, `docs/THREAT_MODEL.md`,
`docs/CONFIGURATION.md`, `docs/IMPLEMENTATION_PLAN.md`, `docs/SYSTEMD.md`, `docs/RECOVERY.md` and
`docs/SECURITY_AUDIT_PLAN.md` for usage, scope, assumptions, rollback and the
mandatory pre-release security review.
The latest handoff state and recommended resume order are in
`docs/CONTINUARE.md`.
The project is available under the permissive [MIT License](LICENSE).
