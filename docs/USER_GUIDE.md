# User guide — Hetzner Drive Manager

This guide describes only behavior available in the current application. Planned
features are listed separately and must not be treated as active.

## Language

The GUI uses Romanian for desktop locales beginning with `ro` and English for all
other locales. Override automatic selection with `hetzner-drive --language ro`,
`hetzner-drive --language en`, or `HETZNER_DRIVE_LANGUAGE=ro|en`. Secure pinentry
dialogs are bilingual because D-Bus can activate the backend independently.

Language selection covers GUI labels, local errors, notifications, and displayed
states. Stable D-Bus codes and sanitized technical events are not translated.

## Daily startup

1. Open **Hetzner Drive** from KDE. D-Bus starts the user service on demand; the
   drive is never mounted automatically.
2. Check SSH status. Load the dedicated key through the existing keychain workflow
   if necessary, then choose **Check SSH**.
3. Choose **Unlock** and enter the password protecting encrypted `rclone.conf` in
   pinentry. Do not enter the crypt password or salt.
4. For filesystem access choose **Mount**, then **Open**. Project backup does not
   require the drive to be mounted.
5. For backup open **Backups**, choose **Unlock restic**, and enter the separate
   repository password.
6. Before unmounting, close documents and require the Transfers card to show a
   safe state. **Lock restic** and **Lock** remove session credentials as described
   below.

## Credentials and confirmations

| Dialog or action | Required value |
| --- | --- |
| Unlock rclone configuration | Password protecting encrypted `rclone.conf` |
| Initialize restic | A new, separate restic password entered identically twice |
| Unlock restic | The restic password retained in the user's password manager |
| Backup/restore confirmation | Explicit pinentry confirmation; not a password |
| SSH key | Its passphrase is handled by the keychain/agent, not this application |

Never paste passwords into terminal commands, chat, screenshots, or logs. The
crypt password and salt remain inside encrypted rclone configuration.

The default private key path is `$HOME/.ssh/hetzner_storagebox`. If `rclone.conf`
uses another filename, start the backend with `HETZNER_DRIVE_SSH_KEY` pointing to
that absolute path, or pass the same path to the installer with `--ssh-key`. It
must be directly below `$HOME/.ssh` and contain only ASCII letters, digits, and
`/._-`; the application opens only the matching `.pub` file for agent fingerprint
comparison.

Direct backend runs also require `HETZNER_DRIVE_SFTP_HOST` and
`HETZNER_DRIVE_SFTP_USER`. Desktop installation requires the matching
`--sftp-host` and `--sftp-user`; the installer validates their Storage Box account
relationship and renders them only into the private user-service file.
The `uninstall` action removes only managed integration files and does not require
Storage Box identity arguments; without `--apply` it remains a preview.

## Status and drive lifecycle

The Status page shows overall state, mount state at `$HOME/HetznerDrive`, SSH and
rclone configuration state, rclone version, remote capacity, local cache use,
observable VFS activity, last operation, and historical last error.

Quick actions refresh observations, check SSH, unlock/lock the configuration,
mount/unmount, open the fixed directory in Dolphin, check authenticated
connectivity, read storage capacity, and run diagnostics. A visible mount is not
evidence of a completed backup.

Mount refuses an occupied or unsafe directory, an unsafe cache, a second mount of
the remote, and an externally owned mount. Before safe unmount, close applications
using the drive. The backend samples VFS state twice and refuses unmount when
uploads are active/queued, errors exist, cache is out of space, or state is
unknown. It never uses force/lazy unmount and never deletes the persistent cache.
Locking credentials does not unmount an existing drive or revoke plaintext already
exposed through FUSE.

## Monitored projects

The Backups page can register up to 32 non-overlapping, user-owned local project
directories outside the Hetzner mount. Roots and parents must not be symbolic
links. Registration chooses a name and a 1–168 hour reminder interval; the default
is two hours with a ten-minute quiet period.

The GUI scans metadata every five minutes and on **Scan now**. It hashes relative
names, sizes, modes, and modification times without opening file contents or
following symlinks. Scans stop at 100,000 entries or 64 levels. Common generated
directories and editor artifacts are excluded by default.

Displayed states are initial backup required, changes detected, backup
recommended, snoozed, verified, or incomplete scan. Removing monitoring changes
only the private local registry. It never deletes local files, restores, or remote
snapshots, but the removed project ID's history is no longer accessible in the
application.

## Restic backup and snapshot history

The repository is fixed at:

`hetzner-crypt:HetznerDrive-Backup-Disposable/restic-v1`

Initialize it once with **Initialize restic**, entering a new separate password
twice in pinentry and retaining it externally. On later sessions use **Unlock
restic**. The key exists only in protected process memory for the current session
and is discarded by restic lock, configuration lock, or service exit.

For a manual backup, unlock both configurations, select a safely scanned project,
and choose **Back up now**. Confirm in pinentry. One restic job may run at a time.
Progress exposes only numeric counters. Success requires a complete snapshot ID
and `restic check --with-cache`; the project is marked verified only when its
local digest did not change during the job.

The history view loads at most 256 recent snapshots filtered simultaneously by
the application tag, exact project ID, and registered path. Refresh is read-only.
Restic lock clears both the table and the session authorization catalog.

To restore the complete project version, select a listed snapshot, choose
**Restore all**, and confirm in pinentry. The backend accepts only the full ID
authorized for that project in the current session and runs verified restore into
a new 0700 directory below `$HOME/HetznerDrive-Restores`.

Selecting a snapshot also loads its **Snapshot contents** tree. Choose one regular
file or directory, select **Restore selection**, review the GUI warning, and
confirm the separate selective-restore pinentry prompt. The service accepts the
relative path only if the exact project/snapshot/path tuple came from its current
validated listing. It escapes restic pattern characters and restores into another
new private directory; the original project is never overwritten.

The listing captures at most 4 MiB and displays at most 2,048 restorable entries.
If more valid entries fit in the response the table is visibly marked truncated;
an oversized response fails closed. Symlinks and special nodes are not directly
selectable. Restoring a directory still restores all of its archived descendants,
including any symlinks below it, so inspect the new target before copying anything
into the original project.
Filenames are exposed on the same-user session bus and in this explicit GUI tree,
but are not added to progress, diagnostics, or sanitized event logs. Locking
restic/configuration or restarting the service clears both history and path
authorization, so reload the snapshot and its contents.

The rclone transport is append-only. The application exposes no arbitrary remote,
command, delete, forget, prune, retention, or overwrite operation.

## Tray, diagnostics, and common failures

Closing the window hides it in the tray when available. Tray Exit closes the GUI
but does not promise to stop the backend or unmount. D-Bus can start the backend
on demand; tray autostart and automount remain disabled by default.

Events contain only allowlisted operations and sanitized codes, never credentials
or raw filenames. The explicitly requested snapshot-content tree is the one place
where stored relative filenames are displayed. Diagnostics expose typed technical
values. When reporting a problem, copy the error code—not passwords, filenames,
or decrypted configuration.

Disabled actions usually mean that the selected project, SSH agent, rclone
configuration, restic repository, or job state is not ready. An incomplete scan
means the root is missing/unsafe or a bound was reached. A snapshot whose check
failed must not be treated as verified. If restore rejects a snapshot after lock
or refresh, reload history and select it again.

A missing/invalid Storage Box policy requires reinstalling integration with the
correct `--sftp-host`, `--sftp-user`, and optional `--ssh-key`; these are public
identity/path values, never password arguments.

## Safety limits

- Keep rclone and restic passwords separate in a password manager.
- A mount, notification, or metadata scan is not proof of backup.
- Inspect restored data before copying it back; do not treat a restore directory
  as the original working project.
- Never delete VFS cache or repository data as an ad-hoc repair.
- Keep an independent copy of critical documents. The disposable repository has
  no retention policy and does not replace a 3-2-1 strategy.

## Planned but inactive

- opt-in automation with power/network conditions;
- controlled cancellation and interrupted-job recovery;
- periodic sampled data checks;
- separately audited retention.
