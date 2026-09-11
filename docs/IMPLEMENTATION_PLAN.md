# Implementation plan

1. Local read-only foundation: isolated Python venv and locks, Rust observations,
   typed D-Bus API, Qt dashboard/tray, process mocks and offline integration tests.
2. Authenticated read-only: pinentry and session lifetime prototype with synthetic
   credentials first, then explicit local validation; storage usage via
   `rclone about hetzner-crypt: --json`. No config rewrites or raw uploads.
3. Controlled mount lifecycle: validated state transitions, owned process,
   external mount/cache collision detection, no duplicate mounts, safe unmount
   with evidence about outstanding writes. Preserve existing script fallback.
4. Desktop integration: opt-in autostart after KDE login, notifications, agent
   expiry handling, suspend/network-loss tests, tested systemd restrictions.
5. Backup workflow, split into explicit safety gates:
   - 5a: local project registry, bounded metadata monitoring, quiet-period policy,
     dashboard reminders, snooze and tray notifications;
   - 5b: audited restic repository, session key handling, user-confirmed snapshot,
     progress, verification and restore into a new directory;
   - 5c: optional idle automation and retention only after disposable-remote,
     failure and restore acceptance. No sync or remote deletion is allowed in
     phases 5a or 5b.

## Resume checkpoint (2026-09-10)

- Phases 1 and 2: implemented and verified offline, including real rclone with
  synthetic encrypted configuration. Actual desktop credentials/network checks
  still need the manual checklist. Connection failures and operation history now
  remain visible in the dashboard.
- Phase 3: RC over a private authenticated Unix socket was explicitly approved.
  Owned foreground mount, collision checks, VFS activity display, safe unmount
  checks and shutdown handling are implemented and verified with synthetic/demo
  tests. On 2026-09-10 the idle live FUSE lifecycle, private RC channel, duplicate
  rejection and safe unmount passed without creating or modifying remote files.
- Phase 4: user-level integration installed with autostart disabled. Live D-Bus
  activation, keychain inheritance, service-owned FUSE mount/unmount and clean
  service stop passed on 2026-09-10.
- Phase 5a: implemented in the GUI process and verified with temporary local
  projects. It never opens file contents, calls rclone/restic or writes remotely.
- Phase 5b: implemented with restic 0.18.0, a separate session-only repository
  password, a fixed append-only rclone backend, explicit backup/restore
  confirmation, bounded numeric progress, post-snapshot repository checking and
  verified restore into a new directory. Increment 5b.1 adds bounded,
  repository-backed project history and restore selection that survives service
  restarts after the repository is unlocked again. Increment 5b.2 adds a bounded
  typed snapshot-content tree and verified restore of one server-authorized file
  or subdirectory into a new private destination. Local synthetic acceptance
  covers exact-file, wildcard-escaping and complete selected-directory paths. Live
  acceptance passed previously on one 151-byte disposable whole-restore fixture.
  Phase 5c and destructive retention remain disabled.

## User workflow

Primary work is a university thesis, source projects and Office documents. Prefer
local working copies, Git commits pushed to the existing private repository for
source, and independently recoverable document/project backups. A mounted cloud
folder is not proof of completed backup. Prioritize last verified backup and
restore status in every backup view; never invent these from mount status.

The current views include drive status, connection/agent health, observable VFS
activity, cache allocation, actionable errors, storage usage, project activity,
restic progress, bounded snapshot history/selection and restore destination.

Completed 5b.1: `restic snapshots --json` uses the existing fixed credential
channel, queries only the registered project tag/path and returns at most 256 typed
entries. Restore accepts only a full ID from the current project's server-validated
session catalog; repository lock clears the catalog.

Completed 5b.2: `restic ls --json --recursive` is capped at 4 MiB and 2,048
regular-file/directory entries. Paths must remain relative to the exact registered
snapshot root and are stored in a second session catalog. Selective restore escapes
restic pattern characters, requires explicit confirmation and uses the same new
0700 target plus `--verify` policy as complete restore.

## Proposed next increments

1. **5c.1 — Actionable due notification (high value, low risk).** The existing
   reminder opens the project and offers a one-click request, but the service still
   requires explicit pinentry confirmation. Add per-project time windows and cooldowns before
   considering unattended execution.
2. **5c.2 — Controlled cancellation and recovery (high value, medium risk).** Keep
   the exact restic child handle, request graceful interruption, reap it, report an
   unverified result and validate repository metadata before the next backup.
3. **5c.3 — Power/network-aware scheduling (medium value, medium risk).** Use typed
   UPower and NetworkManager D-Bus properties to avoid battery or metered links;
   unknown state must block automation rather than assume it is safe.
4. **5c.4 — Periodic data sampling (high assurance, medium bandwidth).** Schedule
   read-only `restic check --read-data-subset` slices, record only aggregate results
   and rotate through all subsets over time.
5. **Maintenance phase — Retention (useful, high risk).** Keep it outside the
   append-only backup client. Start with a read-only/dry-run proposal and require a
   separately secured maintenance context plus explicit audit before any
   forget/prune/delete capability.

Selective restore now precedes unattended automation; repository-backed history
makes complete and targeted recovery usable across service restarts after reload.

## Current limitations and assumptions

- User paths derive from `$HOME`. The Storage Box identity comes from explicit
  private service policy, is validated against the provider-specific host/user
  relationship, and anchors fixed overrides applied to the sealed configuration;
  arbitrary configuration editing remains outside scope.
- Public-key and log content are read only by the running diagnostic application.
  Development tests use generated fixtures, not personal files.
- Ordinary polling reads config metadata only. Explicit unlock uses a sealed
  encrypted snapshot and rclone's redacted output; see AUTHENTICATION.md.
- Agent presence is evaluated in the service environment. An absent socket does
  not prove there is no agent elsewhere in the desktop session.
- Local health while locked does not verify network connectivity or uploads.
- Session-secret retention is implemented and tested on synthetic config.
- Project monitoring runs every five minutes only while the GUI/tray is alive.
  Activity means metadata changes, not proof of continuous human work.
- A notification or mounted drive is not a completed backup. Only a successfully
  created snapshot followed by `restic check`, with an unchanged local scan digest,
  may update `last_verified_at`.
- Live idle KDE/FUSE and disposable backup/restore acceptance passed. Interrupted
  backup, network loss, suspend, retention design and the final security audit
  remain pending.
