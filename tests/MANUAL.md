# Manual validation in KDE

Automated tests cover the demo service and synthetic credentials. They do not
validate the real KDE session, account, network or FUSE. Run these checks as the
normal desktop user. Keep passwords exclusively in pinentry; never copy them
into a terminal command, screenshot, issue or log.

## Local dashboard

1. If the encrypted config does not use the default
   `$HOME/.ssh/hetzner_storagebox` key, set `HETZNER_DRIVE_SSH_KEY` to its absolute
   private-key path in the backend environment. Start
   `./target/debug/hetzner-drive-core`, then `.venv/bin/hetzner-drive`.
   For a visual-only check use `--demo` on the core and confirm "Simulated data".
   On any desktop locale, launch once with `--language en` and once with
   `--language ro`; verify static controls, runtime states, errors, notifications,
   and backup/history text. Pinentry prompts intentionally show both languages.
2. Compare the displayed mount with `findmnt "$HOME/HetznerDrive"`.
   A mount observation alone does not verify remote connectivity.
3. Compare SSH state with `ssh-add -l -E sha256` and the dedicated public key's
   fingerprint. Never open or print the private key. "Check SSH" must refresh
   the observation; the app does not load keys into the agent itself.
4. Confirm config existence/permissions are reported and the session starts
   locked. No password dialog should open until "Unlock" is selected.
5. "Diagnostic" while locked checks local health and explicitly says the config
   remains locked. Space must be unavailable, never an invented value or zero.
6. Check "Last operation" and the historical "Last error".
   Successful actions do not erase error history. Events contain only fixed
   summaries/codes, never raw filenames or credentials.

## Explicit authentication and read-only remote checks

1. Select "Unlock". Pinentry asks for the password protecting the encrypted
   `rclone.conf`, not the crypt remote password or its salt. Cancel pinentry and
   confirm the app stays locked and the cancellation is shown. Retry with an
   incorrect password; it must fail without displaying or logging the password.
2. Unlock with the actual config password. Only pinentry handles the input.
   "Connection" and "Cloud storage" become available; space refreshes automatically
   after a successful unlock. Compare results with the existing read-only workflow.
3. Run "Diagnostic" while unlocked. The last operation must remain "Diagnostic",
   even though it includes an authenticated connection check.
4. When connectivity naturally fails, a failed remote check must leave the state
   requiring attention across refreshes. Stale space is replaced by unavailable.
   A successful remote recheck clears the failure. Do not deliberately interrupt
   networking while other programs have outstanding cloud writes.
5. Select "Lock"; remote queries must be disabled and space unavailable.
   If a drive was mounted externally, it remains mounted and accessible.
6. Restart the core; the session must begin locked without reusing the password.
   When the agent key expires, "Check SSH" must report the missing key;
   recover with the existing keychain workflow, then check again.

## Desktop and service lifecycle

1. Switch KDE light/dark themes; check native controls, text and contrast. Resize
   to 540x480; overview rows, including the operation history, must scroll cleanly.
2. Close to tray, reopen, then Exit. Existing mounts must remain unchanged.
3. Stop the core. GUI must replace old values with unknown and disable actions.
   Restart and refresh; it must recover without restarting the GUI.
4. For an externally mounted drive, "Open" must open the fixed directory in
   Dolphin, "Transfers" must say that activity is unavailable for an
   external mount, and "Unmount" must remain disabled. An unmounted drive must
   refuse the open action.
5. Preview `.venv/bin/python packaging/install.py plan`. Validate live activation,
   tray autostart and user-systemd restrictions only after explicit installation.

## Local project monitoring

1. Open `Backupuri` and confirm that an empty setup says no project is configured.
   Opening, scanning or refreshing the page alone must not prompt for a password
   or issue a remote request. `Back up now` stays disabled until both configuration
   and repository are unlocked and a safe project is selected.
2. Create a disposable local directory outside `$HOME/HetznerDrive`, add it
   with a harmless name and inspect the project registry below
   `$XDG_CONFIG_HOME/hetzner-drive/`. The directory must be 0700 and the JSON file
   0600. Do not edit permissions merely to make an unsafe file pass.
3. Add one regular fixture file, an excluded `target` or `.venv` directory and a
   symlink to a file outside the fixture. Select `Scan now`: only the regular
   file must contribute to the displayed count/size. The activity state file must
   be 0600 and must not contain the fixture filename.
4. Rename or remove the disposable root. The project must show an incomplete scan
   and remain removable; it must not cause another registered project to vanish.
5. Confirm that `Snooze for one hour` updates the state without changing local files.
   Removing monitoring must leave both the fixture and any mounted drive intact.
6. Validate a naturally due notification in KDE only after the configured time
   and quiet period. The persistent tab count remains authoritative because the
   desktop may suppress tray messages.

## Controlled restic acceptance

The fixed disposable repository was initialized and one end-to-end fixture passed
on 2026-09-10. Repeat live steps only with explicit approval; every retained test
snapshot consumes remote space and this phase intentionally provides no deletion.

1. Confirm `/usr/bin/restic` and `/usr/local/bin/rclone` pass the application's
   dependency checks. Unlock `rclone.conf` with its configuration password.
2. For a new empty repository only, select `Initialize restic`. Confirm the
   fixed path is `HetznerDrive-Backup-Disposable/restic-v1`, then enter the same
   new restic password twice in pinentry. It must be distinct from the rclone
   configuration password, crypt password and crypt salt. Store it externally;
   the application keeps it only for the current session.
3. Select `Lock restic`, verify backup is disabled, then select `Unlock restic`
   and enter the stored password. The existing repository must return to
   ready without initialization.
4. Register and select a small 0700 disposable project containing known 0600
   content. Select `Back up now` and confirm in pinentry. Require a
   complete 64-hex snapshot ID and successful post-backup repository check.
5. Restart the service, unlock both configurations again and select the project.
   `Istoric snapshoturi` must load the retained snapshot from the repository with
   aligned date/file/size fields. Locking restic must clear the table; unlock and
   selection must load it again. No history read may create a remote write.
6. Select the retained snapshot, choose `Restore all` and confirm in pinentry.
   The result must be a new 0700 directory below
   `$HOME/HetznerDrive-Restores`; the original must remain untouched. Compare
   restored size and SHA-256 with the source.
7. Reload the same snapshot's `Snapshot contents`. Confirm the tree shows only
   regular files/directories, contains no absolute path, and reports truncation if
   applicable. Select one known file, choose `Restore selection`, verify the GUI
   path, and confirm the selective pinentry prompt. Require a second new 0700
   target containing the exact verified file and no sibling file. Repeat with a
   small subdirectory and inspect every descendant, including any archived
   symlink. Lock restic and confirm the content tree and authorization are cleared;
   an old direct D-Bus tuple must be rejected until reloaded.
8. Remove the temporary project from the local registry after inspection. Local
   fixture/restore cleanup is separate. Do not attempt to remove the remote test
   snapshot through another tool while evaluating the append-only policy.

Retention, forget/prune/delete, unattended confirmation, interrupted jobs,
mid-transfer network loss and suspend recovery are not authorized by this section.

## Controlled mount acceptance

The drive was mounted by the existing external workflow during development and
the application correctly observed it without claiming ownership. On 2026-09-10
the user demounted it and the empty private mount/cache paths were verified. Before
continuing, confirm again that `findmnt` does not report the path and that no other
`rclone` process is active. The tested fallback remains authoritative until the
new lifecycle passes.

1. Unlock the application, select "Mount" and confirm the fixed path becomes
   `fuse.rclone` from `hetzner-crypt:`. Inspect the service process arguments:
   they must contain no config password or RC password, no TCP RC address and no
   `--daemon`, `--rc-pass`, `--rc-no-auth`, `--rc-serve` or web GUI option.
2. Inspect the runtime directory while mounted: it must be owned by the user with
   mode 0700; `rc.sock` must be 0600. There must be no RC TCP listener. Do not
   copy the Authorization header or inspect process memory.
3. Confirm "Transfers" reports zero queue/activity while idle. A second
   mount request must be disabled/refused. The old scripts must not be changed.
4. With all files closed and no new writes, select "Unmount", confirm the
   dialog and verify the mount disappears. The process and private runtime socket
   must disappear; the cache directory must remain.
5. Start an application-owned mount again, then stop the core while idle. It must
   perform a checked unmount and exit normally. Restarting the service must begin
   locked and must not remount automatically.
6. Validate the installed user service separately. Confirm the systemd manager
   receives the keychain SSH_AUTH_SOCK, FUSE works without NoNewPrivileges, and
   the five-minute stop timeout is honored.

Tests that create another live mount file, induce a pending upload, interrupt networking,
fill cache space, suspend the machine or crash rclone can affect real data or the
desktop session. Run them only in the final controlled audit. No mount checklist
step above authorizes another remote write, configuration change or deletion.
