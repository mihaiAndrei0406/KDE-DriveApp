# Resume checkpoint

Last updated: 2026-09-11.

## Current implementation state

- Phases 1–5b.2 are implemented: secure rclone unlock, typed D-Bus status/actions,
  controlled FUSE mount/unmount, project metadata monitoring, user-confirmed
  append-only restic snapshots, bounded project history/content, and complete or
  selective restore.
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
- Snapshot history returns at most 256 newest entries. Snapshot content captures
  at most 4 MiB and exposes at most 2,048 regular-file/directory entries. Complete
  and selective restore accept only the current session's authorized ID/path and
  always write into a new private directory with verification.
- Locking restic, locking rclone configuration, or restarting the service clears
  both in-memory snapshot and relative-path authorization catalogs.

## Latest verification

- 41 Rust unit tests plus the real local synthetic `auth_flow` pass. The local
  restic/rclone integration now lists actual JSON content, restores one exact
  wildcard-named file without its sibling, and restores a selected directory with
  verification.
- 26 Python tests pass, including explicit English-interface, installer-policy,
  snapshot-entry schema/tree, and selective-request coverage.
- Formatting, Clippy, debug/release builds, the isolated D-Bus smoke test, and the
  generic installer preview pass. The 49-file staged tree and ignore rules passed
  the public secret/identity scan; the owner selected the MIT license before the
  initial public commit.
- Previous live acceptance created and restored one synthetic disposable fixture.
  This public-release work does not authorize or perform another remote write.
- Public repository `mihaiAndrei0406/KDE-DriveApp` now exists on `main`. GitHub
  confirmed initial revision `ebe1d5cc0175db132791c9461eefc0b3b7b7936e` and the
  MIT license file.
- Phase 5b.2 has not been deployed to the installed user service or exercised
  against the live disposable Hetzner snapshot. Those remain explicit manual
  acceptance steps, not automatic follow-up actions.

## Local runtime state at the start of this change

- Desktop integration was installed with autostart disabled.
- The user service was inactive/dead and disabled.
- The drive was unmounted; no rclone/restic process was active.
- The local project registry contained zero projects.

## Resume order

1. Keep the installed service on its last validated configuration until the owner
   explicitly previews and applies the documented public-policy migration.
2. In a later controlled session, visually validate live selective restore using
   an existing disposable snapshot before enabling any automation.

## Planned features, still inactive

1. Actionable due-backup notification while retaining explicit confirmation.
2. Controlled cancellation, child reap, and recovery validation.
3. Power/network-aware scheduling that fails closed on unknown state.
4. Rotating sampled data checks.
5. Separately secured and audited retention; no forget/prune/delete exists in the
   append-only client.
