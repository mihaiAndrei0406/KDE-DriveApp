# Architecture and current scope

The application is designed around an existing encrypted rclone configuration;
it does not migrate or edit that setup. Local diagnostics and authenticated read-only
operations, the controlled mount lifecycle and the user-confirmed restic snapshot
lifecycle are implemented. Idle live FUSE, installed-service and disposable
backup/check/restore acceptance passed on 2026-09-10; destructive retention and
failure-mode acceptance remain pending. See `UNMOUNT_DESIGN.md`,
`AUTHENTICATION.md` and `WORK_LOG.md`.

## Components

Python/PySide6 uses QtDBus on the session bus. Rust owns observations and validation.
The bus name and interface are `ro.mihai.HetznerDrive1`, object
`/ro/mihai/HetznerDrive1`. Methods have fixed arguments and fixed output types.
No caller can supply an executable, shell snippet, remote or filesystem path.

`GetStatus` returns nine fields: application state, mount observation, SSH agent
observation, config metadata observation, rclone version, mount path, allocated
cache bytes, cache scan completeness, diagnostic codes. `CheckSshAgent` returns
a status code; `GetStorageUsage` returns availability, total, used, free and a
reason; `GetRecentLogs` returns sanitized event summaries. Explicit unlocking
enables the fixed read-only remote queries. Unknown space is never zero usage.
Additional methods: UnlockConfiguration, LockConfiguration, MountDrive,
UnmountDrive, CheckConnection, RunHealthCheck, OpenDrive, GetOperationStatus and
GetMountActivity. Backup methods report typed status, initialize/unlock/lock the
fixed repository, list a bounded project snapshot history, and start a backup or
restore using a strict 32-hex project ID plus, for restore, a 64-hex snapshot ID
previously authorized by that history. None accepts a password, command,
filesystem path, remote or RC endpoint.

Actions share a single operation guard; competing actions return `operation_busy`.
Explicit SSH and health checks refresh local observations. A failed authenticated
remote query sets `Degraded` and `remote_check_failed`, unless a local error takes
precedence. Polling does not clear that failure; a successful authenticated query
or explicit session lock does. This reports the last check, not continuous network
monitoring. Operation results, including rejected actions and local-only health
checks, are recorded with fixed codes. The GUI shows the historical last error
separately from the current state.

Use `/proc/self/mountinfo` rather than checking whether a process exists. A mount
must match both the exact mount path and `fuse.rclone` / `hetzner-crypt:` source.
Unrelated mounts are reported separately. The public key fingerprint is compared
to `ssh-add -l -E sha256`; the private key is never opened.

Ordinary status inspection is metadata only. Explicit unlocking captures the
encrypted configuration and validates rclone's redacted output. Presence alone
does not imply successful decryption or valid credentials. Cache scanning reads metadata,
not file content, does not follow symlinks and reports truncation explicitly.

## Local backup assistant

The PySide6 tray process owns the phase-5a project registry and scheduler. A
single `QThreadPool` worker scans registered roots every five minutes so large
directory walks do not block the GUI. A `QLockFile` in the per-user runtime plus
a `QLocalServer` restricted to user access permits one GUI/tray monitor; later
launches request activation of the existing window and exit. Registration is explicit; projects cannot
overlap, contain the fixed Hetzner mount, use a symlinked path or belong to a
different UID. Missing project directories remain removable and are displayed as
incomplete rather than invalidating the whole registry.

Traversal opens each absolute path component and child directory with
`O_DIRECTORY|O_NOFOLLOW`. It skips symlinks and generated-name patterns, stops at
100,000 entries or 64 levels and hashes relative names plus size, mtime and mode.
File contents are not opened. Persisted activity records contain only digests,
counters and timestamps; project names and root paths exist only in the private
registry. Both JSON files use atomic replacement, 0600 files and a 0700 app
directory, and reject unknown schema fields or unsafe existing permissions.

The first complete scan establishes unprotected local activity. A project becomes
due only after its configured interval and a ten-minute quiet period. Snooze and
six-hour notification throttling are persisted. KDE notifications are advisory;
the **Backups** tab is the durable source of state.

The Rust phase-5b subsystem owns the only restic execution path. It accepts a
project ID, reloads the same strict registry, and revalidates ownership and every
root component before opening file content through restic. The repository is
fixed at `rclone:hetzner-crypt:HetznerDrive-Backup-Disposable/restic-v1`.
Restic invokes a fixed append-only `rclone serve restic --stdio` transport using
an encrypted 0400 runtime copy of `rclone.conf`; only the already-encrypted bytes
reach disk. The rclone config password and the separate restic repository password
are delivered over distinct one-shot Unix sockets after UID/PID/executable-chain
verification. Neither secret appears in argv, environment values or D-Bus.

Initialization asks for the restic password twice. Unlock validates it with a
bounded metadata listing; lock drops the session secret. Backup uses one filesystem,
fixed tags and the registry exclusions, emits only bounded numeric progress, and
is followed by `restic check --with-cache`. Only then, and only if the GUI scan
digest is unchanged, is local activity marked verified. Restore addresses the
exact full snapshot ID plus original project subpath, uses `--verify`, and writes
only to a newly created 0700 directory below `$HOME/HetznerDrive-Restores`.
History calls query at most 257 recent candidates and expose at most 256 entries
after an exact path plus application/project-tag filter. Only the full IDs in the
current per-project session catalog can be restored; repository lock clears that
catalog, so history is reloaded after restart or unlock.
Only one restic job can run. The command surface contains no forget, prune, delete
or user-selected remote; retention remains a future phase.

## Owned mount and RC

The Rust service starts rclone in foreground mode and retains its child handle.
It refuses an occupied target, a second `hetzner-crypt:` FUSE mount, unsafe or
non-empty mount directories, unsafe cache paths and mounts not created by this
process. A disappeared child is reported as `mount_process_exited`.

The owned process receives the sealed encrypted config and session password using
the existing one-shot helper. Its RC server binds only to a random Unix socket
under a 0700 XDG runtime directory. The socket is changed to 0600. Every mount has
a random 256-bit RC password held in locked memory; rclone reads only its bcrypt
hash (cost 10) from a sealed inherited memfd. No RC password appears in argv,
environment variables or persistent files. Rust implements the bounded HTTP client
directly, verifies the Unix peer UID and exact owned rclone PID, and calls only the
internal `vfs/stats` route. RC has no endpoint-level
capability model, so filesystem permissions and authentication remain defense in
depth rather than isolation from a fully compromised process running as the user.

Unmount requires two successful `vfs/stats` observations separated by 750 ms,
with zero queued/in-progress uploads, zero errored files and no out-of-space flag.
It then invokes fixed `/usr/bin/fusermount3 -u`, confirms mountinfo and reaps the
owned process. Unknown, busy and error states remain mounted. The check cannot be
atomic with arbitrary desktop applications; the GUI first asks the user to close
documents. On service shutdown, pending uploads may be given up to 270 seconds to
finish within the five-minute systemd stop timeout. The persistent VFS cache is
never deleted and rclone can retry retained dirty entries on the next identical
mount.

## Session lifecycle

Start after graphical login, when the session bus and desktop exist, not at the LUKS
unlock prompt. Prepare optional desktop autostart without enabling it now.
Do not enable lingering or automatic mounting. Closing the window leaves the
tray available where supported. Exiting the GUI does not stop an existing mount.

Implemented: Rust-owned session secret, explicit lock action, no persistent store,
pinentry, fixed password-command helper with one-shot peer-checked delivery,
zeroize, memory locking and disabled service/helper dumps. Details and limitations
are in `AUTHENTICATION.md`. Do not promise erasure of rclone's memory.
Locking the controller does not revoke an already mounted decrypted filesystem.
Screen lock should not automatically interrupt outstanding uploads.

The restic password is a second independent session secret. Locking the repository
drops only that secret; locking the rclone configuration or exiting the service
drops both. Repository metadata and encrypted backup data persist remotely, but
the application never persists either plaintext password.

Sources consulted 2026-09-09:
- https://rclone.org/docs/#configuration-encryption
- https://rclone.org/commands/rclone_mount/#vfs-file-caching
- https://dbus2.github.io/zbus/server.html
- https://doc.qt.io/qtforpython-6/PySide6/QtDBus/QDBusConnection.html
- https://specifications.freedesktop.org/autostart/latest/
