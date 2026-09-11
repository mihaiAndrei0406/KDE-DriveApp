# Initial threat model

Assets: thesis and projects, rclone and restic credentials, SSH credentials,
plaintext VFS cache, repository snapshots, filenames, connection metadata and
pending writes.

LUKS protects data at rest while the volume is locked. After login, malware with
the user's access may read decrypted files or interact with their session. Root
compromise and a fully compromised user session cannot be contained by this
controller. Session memory retention is a usability tradeoff, not a guarantee
against malware. Accidental logging, inherited environment variables, crash
dumps and unsafe IPC are additional exposure paths without deliberate spyware.

| Boundary | Risk | Initial control |
| --- | --- | --- |
| GUI to Rust | Arbitrary command or path injection | Fixed typed D-Bus methods; the only path input is a snapshot-relative value previously emitted into a current-session authorization catalog |
| Rust to tools | Environment overrides, stalled processes | Absolute allowlisted programs, cleared environment, bounded output and timeout |
| Local files | Symlink substitution, sensitive content | Fixed paths, openat2 without symlinks for content reads, metadata validation, no private-key reads; encrypted config captured only on explicit unlock |
| Rclone config | Alternate remote sections or global overrides bypass command policy | Case-sensitive remote validation; reject global.* and override.* in the controlled chain; sealed snapshot |
| Logs to GUI | Credentials or personal paths in logs | Bounded tail, only reconstructed known event summaries; raw messages discarded |
| SSH agent | Wrong key counted as ready | Compare exact public fingerprint, distinguish missing agent/key/error |
| Mount observation | Process mistaken for usable drive | Exact mountinfo source/type/path; no network health claim |
| Mount ownership | External or duplicate process killed/adopted | Foreground child handle; exact mount/remote checks; refuse external or second crypt mount |
| Cache | Dirty cache mistaken for safe state or shared by two mounts | Authenticated vfs/stats checked twice; reject error/unknown/pending; refuse another crypt mount; never delete cache |
| RC API | Broad API can read/write or execute with user rights | Unix-only socket in random 0700 directory, socket 0600, exact peer UID/PID, 256-bit ephemeral Basic auth, bcrypt hash in sealed memfd, no generic D-Bus bridge |
| Unmount race | New writes begin after last VFS check | User confirmation to close documents, two observations, regular unmount only; limitation remains non-atomic |
| Service lifecycle | Crash dumps, duplicate monitors and excessive privileges | Refuse root, per-user GUI runtime lock/socket, user service, restrictive umask, disable core dumps |
| Project registry | Tampered paths, overlap with mounted Drive, unsafe local metadata | Private atomic 0600 schema; explicit roots; same-UID, no-overlap and mount containment checks |
| Project scan | Symlink escape, unbounded tree, content exposure, GUI freeze | Descriptor traversal with O_NOFOLLOW, no content reads, default excludes, 100k/64-level bounds, one background worker |
| Backup reminder | Notification mistaken for completed protection | Monitoring alone never changes `last_verified_at`; only successful snapshot + repository check with an unchanged scan digest does |
| Backup D-Bus API | Same-UID caller supplies another path/remote/snapshot or skips policy | Strict project/snapshot IDs plus only a server-listed relative restore path cross D-Bus; Rust reloads the private registry; repository, commands and restore root are fixed |
| Repository credentials | Password leaks through argv, environment, D-Bus or persistent config | Separate pinentry secret in locked memory; one-shot peer-checked helper; empty/multiline values rejected; lock and service exit drop it |
| Restic-to-rclone chain | Helper impersonation or configuration substitution | Exact controller-restic-rclone-helper PID/executable chain; encrypted 0400 runtime config copy; fixed executable ownership/mode checks |
| Remote backup | Compromised flow deletes history or writes outside the approved target | Fixed disposable remote; append-only rclone server; no forget/prune/delete commands; exactly one job |
| False backup success | Partial snapshot or changed source marked verified | Require full snapshot ID, subsequent `restic check --with-cache`, and unchanged local metadata digest |
| Snapshot history/restore | Cross-project ID, forged history, existing local data overwritten or corrupted output accepted | Exact tag/path filtering, bounded typed output, restore only from the session's per-project authorized ID set, new 0700 destination, `restic restore --verify` and typed completion |
| Snapshot content/selection | Traversal, glob expansion, direct special-node selection, stale or oversized listing | 4 MiB/2,048-entry bounds, exact project-root stripping, normal relative components, file/dir-only catalog, escaped include pattern and catalog clear on lock; directory descendants can include archived symlinks but remain in a new target with an inspection warning |

The session bus is not an isolation boundary between programs of the same user.
Do not send secrets over it. A future unlock method must not return a secret.
An untrusted local user/process cannot be made safe merely by a GUI confirmation.
Session unlocking is implemented; its boundaries are described in `AUTHENTICATION.md`.

Existing rclone log permissions are 0640. Do not chmod it automatically. New
application files carrying sensitive information must use 0600/0700. This
iteration writes no application log file and only emits fixed operational events
to stderr. Configure bounded log retention before persistent production logging.

Mount/unmount uses serialization, ownership tracking, collision checks and RC
pending-write observations. Existing externally started mounts are not killed or
silently adopted. Unknown upload state is displayed as unavailable and blocks
unmount. The final audit plan is in `SECURITY_AUDIT_PLAN.md`.

Phase 5b now implements the fixed command/repository surface, independent project
validation, session-only credentials, bounded repository-backed history/content,
and verified complete or selective restore. Repository lock clears both in-memory
catalogs; history and selected-snapshot contents must be loaded again after unlock
or restart. Residual
work includes interrupted backup, network loss and suspend testing, repository
growth/retention design, recovery after partial transport failure, and deciding
whether any unattended confirmation model is acceptable. Remote deletion remains
disabled until a separate audit explicitly approves it.
