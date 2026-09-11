# Current verification

Validated initially on 2026-09-09 with Rust/Cargo 1.96.0, Python 3.13.5 and
PySide6 6.11.2; the phase-5b suite and live acceptance were completed on
2026-09-10 with restic 0.18.0. Public-release and bilingual-interface checks were
completed on 2026-09-11.
This replaces the iteration-1-only report; the earlier results remain recorded
in `WORK_LOG.md`.

## Automated checks

- `cargo test --locked --offline --all-targets`: 41 Rust unit tests passed. Coverage includes
  command restrictions, mount observations, operation serialization, persistent
  remote failures/recovery, local-only health results, operation history, private
  directory validation, remote mount collision detection, owned demo lifecycle,
  shutdown cleanup, fixed foreground mount arguments, strict backup registry
  parsing, exact project/tag/path snapshot-history filtering, restore authorization
  from per-project session catalogs, the fixed disposable append-only repository
  policy, bounded typed snapshot-content parsing, traversal/stale-catalog
  rejection, and exact escaped selective-restore arguments.
- The same command runs the standalone `auth_flow` test: real rclone decrypts a
  temporary encrypted fixture through the one-shot password helper. Correct and
  incorrect synthetic passwords, unchanged configuration and socket cleanup pass.
  It never connects to Hetzner or reads the user's key/configuration.
- Authentication regressions cover case-sensitive remote validation, literal
  `#`/`;` in serialized values, rejection of `global.*`/`override.*`, unauthorized
  password peers, immediate process exit before requesting a password and the
  validated external SFTP destination policy with mandatory pre-existing host-key
  pinning. The fixture uses only the generic `u12345` example account.
- RC tests validate Basic authentication encoding, bounded and typed `vfs/stats`
  parsing, every unsafe unmount condition, absence of the clear RC credential in
  argv/htpasswd and the exact exclusion of daemon/raw/destructive mount arguments.
  A real `rclone rcd` process accepted the bcrypt htpasswd through an inherited
  sealed memfd on a temporary Unix socket twice; an intentionally wrong peer PID
  was refused before credentials were sent. No TCP listener or remote exists.
- A real local restic/rclone integration test uses both peer-checked password
  helpers, an encrypted rclone config, append-only transport, snapshot creation,
  repository checking, JSON history/content filtering, complete restore, exact
  `--verify` restore of a file whose name contains restic pattern characters,
  sibling exclusion, and verified restoration of a complete selected directory.
  Its
  repository and credentials are synthetic and temporary; it never contacts
  Hetzner.
- `cargo clippy --locked --all-targets -- -D warnings`: passed.
- `cargo fmt --all -- --check`: passed.
- Debug and release builds using `--locked --offline`: passed.
- The English-catalog AST audit checked 205 static and mapped UI strings: no
  translation entry was missing.
- `.venv/bin/python -m unittest discover -s tests -p 'test_*.py' -v`: 26 passed
  (14 GUI/schema tests, 6 project-monitor tests and 6 installer tests using
  temporary paths). Coverage includes explicit English static/runtime UI and the
  public installer policy, identity-free uninstall preview, defensive snapshot
  entry decoding, hierarchical display, and selective request routing.
- `dbus-run-session --config-file=tests/dbus-session.conf -- .venv/bin/python
  tests/dbus_smoke.py`: passed. The private bus has no activation
  directories and the test refuses to run when the installed service is visible.
  It checks the per-user GUI single-instance activation, fixed typed API,
  rejection of unsupported methods, duplicate service ownership,
  unlock/lock, simulated backup, typed snapshot history/content, complete and
  selective restore, owned mount/unmount, activity status,
  safe shutdown, 64-bit space values, operation/error history, overlapping
  refreshes and GUI recovery after service loss. The core always uses `--demo`.
- Purple Midnight and Lightning Purple offscreen views were inspected at 900x760
  and 540x480. The new Backupuri page was also inspected with synthetic verified,
  changed and due projects; actions reflow below 700 px and the page scrolls at
  minimum size without clipping controls. Captures are temporary artifacts.
  The new selective tree is covered by offscreen hierarchy/action tests but has
  not yet received a live KDE visual inspection with a real snapshot.
- The release `--demo --inspect` command returns a synthetic locked snapshot.
- Installer `plan` shows four destination paths plus complete rendered contents.
  The 2026-09-11 plan used only generic example policy values and changed no live
  files. `systemd-analyze --user verify` previously passed for the
  rendered user unit stored in a temporary directory. Static verification does
  not prove graphical activation or runtime sandbox compatibility.
- The staged public tree contains 49 source/license/documentation/test files. Ignore rules
  exclude the virtual environments, build output, agent metadata, bytecode, and
  personal source documents. Pattern scans found no real account ID, personal home
  path, actual SSH-key filename, private-key block, GitHub token, or credential;
  two clearly named synthetic password fixtures remain solely for authentication
  and log-redaction tests.
- Initial commit `ebe1d5cc0175db132791c9461eefc0b3b7b7936e` was pushed to the
  public `main` branch at `mihaiAndrei0406/KDE-DriveApp`. The GitHub API returned
  the same revision and confirmed the 1,069-byte MIT `LICENSE` file. No ignored
  local artifact was included.

## Failures encountered and resolved

- The sandbox denies local socket operations: the password-broker test and
  isolated D-Bus startup initially failed. They passed when rerun with approved
  access outside the sandbox. Static systemd verification likewise required
  escalation because `SO_PASSCRED` was blocked.
- The expanded D-Bus test exposed a lost refresh while an earlier status request
  was still pending. The client now queues one follow-up status request. The
  integration test also waits for the requested responses rather than accepting
  any unrelated Qt signal as completion.
- Once desktop activation was installed, a default `dbus-run-session` could see
  the user's D-Bus service files and re-activate the release backend after the
  synthetic backend stopped. The smoke test now uses a service-directory-free
  bus configuration and fails closed if the installed name is activatable.
- The new D-Bus smoke assertion found that the `--demo` configuration-unlock
  branch returned before synchronizing its synthetic restic state. The demo-only
  branch now updates the backup manager before returning; the isolated smoke test
  and full regressions pass.
- The first real selective-restore test used restic's `snapshot:subpath` form for
  a file, which restic supports only for subfolders. The implementation now uses
  the documented `--include` mechanism relative to the validated project root,
  escapes Go-pattern metacharacters, and the exact-file `--verify` test passes.

## Live disposable backup acceptance

On 2026-09-10 the user initialized the fixed repository
`hetzner-crypt:HetznerDrive-Backup-Disposable/restic-v1` with a new password kept
in their password manager. The application never received it through chat,
terminal arguments or D-Bus. Backend status confirmed available, initialized,
unlocked and ready.

An explicitly registered 0700 temporary project containing one 0600, 151-byte
fixture was backed up after explicit pinentry confirmation. Restic returned the full snapshot
ID beginning `4269892a`, and the subsequent `restic check --with-cache` completed.
The exact snapshot/project subpath was then restored with `--verify` into a new
0700 directory below the then-current restore root (now
`$HOME/HetznerDrive-Restores`); the restored file remained
0600 and its SHA-256 matched the original:
`a1107f331ab860edc190417c79e855a728e6ec1d08deb24c8de549ef06625dfd`.

Repository lock changed the reported state to locked and removed the session key.
Unlock with the password retrieved from the user's manager successfully validated
the existing repository and returned to ready. No remote delete, forget, prune,
retention or overwrite was exposed or attempted. The disposable snapshot remains
remote by design because the audited transport is append-only.

## Still unverified or unimplemented

On 2026-09-10 the user demounted the pre-existing external mount; the empty,
private mount/cache paths and absence of another rclone process were confirmed.
The dedicated SSH key was loaded through KDE askpass. Pinentry successfully
unlocked the encrypted `rclone.conf`; a prior cancellation and an intentionally
wrong credential type both failed closed. The prompt was clarified to distinguish
the config password from the crypt password and salt.

The actual account passed read-only connection and storage checks. The release
backend created an application-owned `fuse.rclone` mount from `hetzner-crypt:`.
Inspection confirmed fixed password-free arguments, Unix-only RC, 0700/0600
runtime permissions and idle VFS statistics. A duplicate mount was refused. The
live GUI drove connection, health and Dolphin actions. Safe unmount succeeded;
the FUSE process/runtime disappeared and the persistent cache remained unchanged.

Installed user-systemd and D-Bus activation were validated live. The manager saw
the keychain socket, started the backend on demand and completed an idle mount,
authenticated RC check, safe unmount and clean stop. Generic systemd restrictions
that imply `no_new_privs` were shown with synthetic mounts to block Debian's
setuid FUSE helper; only compatible controls remain enabled and documented.

Still unverified: KDE light/dark theme switching, notification and tray behavior
across a full login, SSH/keychain expiry, network loss/suspend, pending or failed
live uploads, and recovery from a dirty cache. See `tests/MANUAL.md` for the
remaining controlled checklist.

The local project monitor and backup engine have been verified with temporary
fixtures; no personal working directory was backed up. Native KDE notification
delivery, five-minute runtime polling, large real project performance and
false-positive tuning remain manual checks. Restic is installed and the fixed
disposable repository is initialized, but retention remains unimplemented. The
new release method was also called while locked and failed closed with empty typed
arrays and `configuration_locked`. A live Hetzner history query awaits a registered
current project; the real local restic integration and isolated D-Bus flow passed
without creating another remote snapshot.

Phase 5b.2 selective restore passed synthetic unit, real local restic/rclone file
and directory restoration, Qt schema/tree/action, and isolated D-Bus checks. Live
listing/restoration from the existing disposable Hetzner snapshot, a response
that exceeds 4 MiB, and the 2,048-entry truncation presentation remain manual
checks. No remote read or write was performed for phase 5b.2 during this
implementation.

The RC exception was explicitly approved and the private authenticated mechanism
passed both direct and installed-service live mount checks. Pending-upload and
failure tests against FUSE remain part of controlled acceptance and the final
security audit. Interrupted backup, transport loss during a job and recovery from
a partial remote write remain unverified. One explicitly approved live Hetzner
write occurred only in the fixed disposable repository; no remote deletion or
private-key content read occurred. Autostart remains disabled.
