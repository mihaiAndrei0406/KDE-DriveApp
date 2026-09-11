# Development work log

This file records material implementation decisions, validation, and remaining
limits. It contains no passwords, decrypted configuration, private-key content,
or personal project data. Command output is summarized rather than copied when it
could reveal operational metadata.

## 2026-09-09 — Initial offline implementation

- Inspected the requirements and existing rclone workflow without reading the
  private key, decrypting configuration, or contacting Hetzner.
- Created an isolated Python environment and lockfile, Rust workspace/lockfile,
  typed D-Bus service, native Qt GUI/tray, local diagnostics, tests, and packaging
  templates.
- Implemented mountinfo/config/public-key/cache/log observations with bounded and
  sanitized output. GUI layout was verified offscreen at normal and minimum sizes.
- Added duplicate D-Bus-name rejection and GUI recovery after service restart.
- At this stage KDE tray, real authentication, FUSE, systemd, and remote access
  were intentionally unverified.

## 2026-09-09 — Authentication and controller

- Implemented pinentry-based `rclone.conf` unlock. Secrets use bounded Assuan
  parsing, `mlock`, `MADV_DONTDUMP`, zeroization, and no Debug representation.
- Captured the encrypted config in a sealed memfd. rclone obtains the password
  through a one-shot 0600 Unix socket in a 0700 runtime directory; broker/helper
  verify UID, PID, and process ancestry. Passwords never enter GUI/D-Bus/argv.
- Added fixed authenticated operations for redacted config validation, connection,
  and storage usage. The validator requires the exact crypt→SFTP chain, host-key
  pinning, agent use, and rejects global/override options, external SSH, proxies,
  and insecure settings.
- Added controller operation serialization, stable sanitized event codes, remote
  failure persistence/recovery, local health, Dolphin opening, and GUI display of
  last operation plus historical last error.
- Fixed a GUI race by retaining one follow-up status refresh while a prior request
  is in flight. Packaging preview now renders complete managed files.
- Offline Rust/Python/D-Bus, format, Clippy, debug/release, and systemd-template
  checks passed. Sandbox-denied Unix socket tests passed in the approved context.

## 2026-09-09 — Private RC and controlled mount lifecycle

- The owner explicitly approved rclone RC only over a private authenticated Unix
  socket. No TCP port, web interface, generic RC method, or external-mount adoption
  was enabled.
- Implemented an owned foreground rclone mount with strict remote/path/cache
  checks, VFS full cache, bounded settings, and collision rejection.
- Each mount receives a random 256-bit RC password in protected memory; only a
  bcrypt hash reaches rclone through a sealed memfd. Rust verifies exact peer UID/
  PID before sending authentication and calls only `vfs/stats`.
- Safe unmount requires two clean VFS observations 750 ms apart, refuses unknown,
  queued, active, errored, out-of-space, busy, force, and lazy states, confirms
  mount disappearance, reaps the child, and preserves cache.
- Tests with real local `rclone rcd`, synthetic lifecycle, D-Bus demo, and static
  command review passed. Live FUSE and final audit were still pending.

## 2026-09-10 — Live KDE/FUSE acceptance

- After the pre-existing external mount was removed, verified the empty 0700 mount
  point, dedicated 0700 cache, and absence of another rclone process.
- Loaded the dedicated SSH key through KDE askpass. Cancellation and a wrong
  credential type failed closed; prompt wording was clarified to distinguish the
  `rclone.conf` password from crypt password/salt.
- Real read-only connection/storage checks succeeded. An application-owned FUSE
  mount used fixed password-free arguments, a private RC socket, no TCP listener,
  and reported idle VFS 0/0/0. Duplicate mount was refused.
- The live GUI drove read-only checks and Dolphin opening. Safe unmount removed the
  mount/process/runtime while preserving cache. The temporary backend stopped cleanly.

## 2026-09-10 — Installed user integration and UI theme

- With explicit approval, installed only the four managed user-level files:
  systemd unit, D-Bus activation, launcher, and disabled autostart entry.
- D-Bus activation inherited the keychain socket and the installed service passed
  unlock, live mount, authenticated VFS observation, safe unmount, and clean stop.
- Incremental hardening showed several systemd restrictions break Debian's setuid
  FUSE helper or desktop propagation. The final compatible unit retains
  `KeyringMode=private`, `UMask=0077`, `LimitCORE=0`, and five-minute stop timeout.
- Reworked the Qt Widgets UI into the Purple Midnight / Lightning Purple dashboard
  following KDE palette changes. Offscreen normal/minimum layouts passed.
- Isolated D-Bus smoke configuration now cannot accidentally activate the installed
  release service. Old GUI/backend processes from visual testing were closed.

## 2026-09-10 — Phase 5a project monitor

- Added the Backups page, strict private project registry, five-minute/manual
  metadata scans, quiet period, due/changed/verified/incomplete states, snooze, and
  bounded KDE notifications.
- Traversal opens directories through descriptors with `O_NOFOLLOW`, never opens
  file contents, skips symlinks/generated paths, and stops at 100,000 entries or
  64 levels. Registry/state use atomic 0600 files below private 0700 directories.
- Rejects overlapping projects, unsafe roots, other-UID roots, and any project
  containing or contained by the Hetzner mount. Persistent state contains digests
  and counters rather than filenames.
- Added per-user `QLockFile`/`QLocalServer` single-instance activation so duplicate
  GUI scans cannot run concurrently. No personal directory or remote was accessed.

## 2026-09-10 — Phase 5b verified restic snapshot/restore

- Installed and validated restic 0.18.0. Added a fixed disposable repository and
  append-only `rclone serve restic --stdio` transport; no arbitrary operation,
  delete, forget, prune, or retention surface exists.
- Added a separate restic session password using peer-checked one-shot channels.
  Initialization asks twice; later unlock validates repository metadata. Lock/
  config lock/service exit remove it from protected memory.
- Rust accepts only a strict project ID, reloads the private registry, revalidates
  owner/no-symlink root, permits one job, and requires separate pinentry confirmation
  for backup and restore. Progress exposes numeric counters, never filenames.
- Backup uses one filesystem, fixed tags/exclusions, then `restic check --with-cache`.
  Restore uses a complete snapshot ID, exact project subpath, `--verify`, and a new
  private target; originals are never overwritten.
- Explicit live acceptance initialized the disposable repository, backed up a
  synthetic 151-byte fixture, checked it, restored it, and matched its SHA-256.
  Repository lock/unlock passed. One remote snapshot remains by append-only design;
  no deletion occurred.

## 2026-09-10 — Phase 5b.1 snapshot history

- Added typed `GetProjectSnapshots`. The backend runs bounded `restic snapshots
  --json`, filters by application tag, project ID, and exact registry path, and
  returns at most 256 entries without arbitrary paths or filenames.
- Restore now accepts only a full snapshot ID from the selected project's
  server-validated session catalog. Restic/config lock and service restart clear
  that authorization.
- Added automatic/explicit history refresh, newest-first table, defensive Qt D-Bus
  decoding, and exact hidden-ID selection for restore.
- Final checkpoint: 38 Rust tests plus real synthetic `auth_flow`, 21 Python tests,
  format, Clippy, isolated D-Bus smoke, local restic/rclone integration, and release
  build passed. The current registry was empty and no new remote write occurred.

## 2026-09-11 — Public bilingual release preparation

- Added Romanian/English GUI localization. Desktop locale selects Romanian for
  `ro*` and English otherwise; CLI/environment overrides accept `ro` or `en`.
  Static controls, states, runtime errors, backup/history messages, notifications,
  dialogs, and project-monitor errors are localized. Pinentry prompts are bilingual
  because the service may be activated without a GUI language context.
- Added an English user guide while retaining the required Romanian guide. Public
  technical/maintenance documents are being normalized to English.
- Removed the Storage Box account identifier and account-specific absolute paths
  from public source. Paths derive from `$HOME`; the expected SSH key defaults to
  `$HOME/.ssh/hetzner_storagebox` and can be selected through a constrained
  `HETZNER_DRIVE_SSH_KEY`/installer `--ssh-key` value.
- Host/user now come from explicit private service policy rather than source.
  Validation enforces the Hetzner hostname/user relationship, port 23, expected
  agent key, host pinning, and prior override restrictions. Fixed command flags
  anchor the sealed config and fail closed without embedding an account identity.
- Restore root is now language-neutral: `$HOME/HetznerDrive-Restores`. Existing
  restore directories are untouched. The source installer is not applied
  automatically; deployment migration requires explicit rebuild/preview/install.
- Personal input documents and agent workspace metadata are excluded from Git.
  No credentials, live remote operation, service restart, mount, or installed-file
  modification was performed during public preparation.
- Final verification passed: 39 Rust unit tests plus real synthetic `auth_flow`,
  25 Python tests, formatting, Clippy, offline debug/release builds, isolated D-Bus
  smoke, and a generic installer preview. No remote operation was performed.
- The 49-file staged tree passed whitespace and public identity/secret review;
  ignored personal documents, environments, agent metadata, and build output are
  absent. The owner selected the MIT license.
- GitHub CLI authentication completed through the official browser device flow;
  no password, token, or 2FA value was exchanged. Repository-local Git identity
  uses the account's `users.noreply.github.com` address.
- Initial commit `ebe1d5cc0175db132791c9461eefc0b3b7b7936e` was pushed to public
  `mihaiAndrei0406/KDE-DriveApp` on `main`. GitHub returned the same revision and
  the expected MIT license file. No service, mount, credentials, or remote backup
  data was changed.
