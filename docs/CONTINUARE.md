# Resume checkpoint

Last updated: 2026-09-11.

## Current implementation state

- Phases 1–5b.1 are implemented: secure rclone unlock, typed D-Bus status/actions,
  controlled FUSE mount/unmount, project metadata monitoring, user-confirmed
  append-only restic snapshots, bounded project history, and whole-snapshot restore.
- The GUI supports Romanian and English. Desktop locale selects the language;
  `--language ro|en` and `HETZNER_DRIVE_LANGUAGE` provide explicit overrides.
  Pinentry prompts are bilingual.
- User paths derive from `$HOME`; restores now use
  `$HOME/HetznerDrive-Restores`. No Storage Box account ID is embedded in source.
  Private service policy supplies host/user values and anchors fixed overrides for
  the encrypted rclone snapshot; strict Hetzner hostname/user, port, SSH-agent,
  key-path, and host-pin checks remain mandatory.
- The SSH key defaults to `$HOME/.ssh/hetzner_storagebox`. A different key filename
  requires `HETZNER_DRIVE_SSH_KEY` or installer `--ssh-key`.
- Snapshot history returns at most 256 newest entries. Restore accepts only a full
  ID authorized for the selected project in the current unlocked session and
  always writes into a new private directory.
- Locking restic, locking rclone configuration, or restarting the service clears
  the in-memory snapshot authorization catalog.

## Latest verification

- 39 Rust unit tests plus the real local synthetic `auth_flow` pass after the
  public-release policy update.
- 25 Python tests pass, including explicit English-interface and installer-policy
  coverage.
- Formatting, Clippy, debug/release builds, the isolated D-Bus smoke test, and the
  generic installer preview pass. The 49-file staged tree and ignore rules passed
  the public secret/identity scan; the owner selected the MIT license before the
  initial public commit.
- Previous live acceptance created and restored one synthetic disposable fixture.
  This public-release work does not authorize or perform another remote write.
- Public repository `mihaiAndrei0406/KDE-DriveApp` now exists on `main`. GitHub
  confirmed initial revision `ebe1d5cc0175db132791c9461eefc0b3b7b7936e` and the
  MIT license file.

## Local runtime state at the start of this change

- Desktop integration was installed with autostart disabled.
- The user service was inactive/dead and disabled.
- The drive was unmounted; no rclone/restic process was active.
- The local project registry contained zero projects.

## Resume order

1. Keep the installed service on its last validated configuration until the owner
   explicitly previews and applies the documented public-policy migration.
2. In a later session, visually validate live history with a disposable project,
   then design phase 5b.2 selective file/subdirectory restore.

## Planned features, still inactive

1. Selective restore from a bounded `restic ls --json` tree.
2. Actionable due-backup notification while retaining explicit confirmation.
3. Controlled cancellation, child reap, and recovery validation.
4. Power/network-aware scheduling that fails closed on unknown state.
5. Rotating sampled data checks.
6. Separately secured and audited retention; no forget/prune/delete exists in the
   append-only client.
