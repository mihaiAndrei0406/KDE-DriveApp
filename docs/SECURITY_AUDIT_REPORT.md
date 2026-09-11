# Security audit report — checkpoint 2026-09-11

## Scope and method

This is an evidence checkpoint, not a security certification. It covers static
command/IPC/secret review, automated synthetic integration tests, the controlled
live idle mount and disposable snapshot acceptance performed on 2026-09-10, and
the public-source preparation performed on 2026-09-11.

No private key, decrypted configuration, password, or personal project content is
included in the repository. Public-release work did not contact Hetzner or create,
delete, overwrite, forget, or prune remote data.

## Confirmed controls

| Area | Evidence | Result |
| --- | --- | --- |
| Privileges | Service rejects UID 0; FUSE uses the distribution helper | OK |
| D-Bus | Typed fixed methods; no arbitrary command/path/remote execution | OK |
| Processes | Absolute executables, cleared environment, bounded output, timeout/reap | OK |
| Config | Mode/owner/no-symlink checks and one sealed encrypted snapshot | OK |
| Secrets | pinentry, one-shot peer-checked sockets, mlock/zeroize, no D-Bus/argv secret | OK within documented limits |
| SSH/SFTP | Agent key, expected key path, port 23, strict Storage Box host/user relationship, host pin | OK static/live |
| Mount | Exact source/type/path, owned foreground child, collision rejection | OK live idle |
| RC | Unix-only 0700/0600 runtime, bcrypt through memfd, exact peer PID | OK live idle |
| Unmount | Two VFS observations, no force/lazy, mountinfo confirmation | OK live idle |
| Backup | Fixed append-only repository/transport, one job, explicit confirmation | OK disposable fixture |
| Restore | Server-validated snapshot catalog, `--verify`, new 0700 target | OK disposable fixture |
| Public tree | Personal input documents ignored; account ID and absolute user paths removed | OK static scan |

## Findings

### AUD-01 — systemd hardening compatible with FUSE (MEDIUM, closed with limit)

`NoNewPrivileges`, `PrivateTmp`, `ProtectSystem`, `RestrictSUIDSGID`,
`MemoryDenyWriteExecute`, `RestrictAddressFamilies`,
`SystemCallArchitectures`, `LockPersonality`, and `RestrictRealtime` separately
blocked Debian's setuid FUSE helper, SSH agent access, or desktop mount propagation
during controlled tests. The final unit retains `KeyringMode=private`, `UMask=0077`,
`LimitCORE=0`, and a five-minute stop timeout. This is a deliberately accepted,
documented surface rather than a claim of maximal systemd isolation.

### AUD-02 — supply-chain audit incomplete (MEDIUM, open)

Lockfiles exist and the dependency tree was inspected, but a current Cargo/Python
advisory and license database has not been run, and `/usr/local/bin/rclone`
provider provenance has not been revalidated. Routine autostart remains unapproved
until this is completed or explicitly accepted.

### AUD-03 — Git metadata absent (MEDIUM, in remediation)

The supplied `.git` directory was empty, so previous changes had no reproducible
commit history or diff. Public-tree review and test evidence are being established
before an initial Git commit. This finding closes only after the reviewed commit is
pushed and its GitHub revision can be verified.

### AUD-04 — same-UID D-Bus callers (LOW/MEDIUM, accepted model limit)

Any process under the desktop UID can call D-Bus methods. The GUI is not a security
boundary and passwords do not cross D-Bus. A fully compromised same-UID process can
already access user files and mounts. GUI confirmation is therefore UX, not an
authorization boundary.

### AUD-05 — unmount observation is not atomic (MEDIUM, accepted MVP limit)

Another application can begin writing between VFS observations and
`fusermount3 -u`. Unknown/busy/error states fail closed and force/lazy modes are
forbidden, but the race cannot be eliminated without filesystem/desktop
cooperation. Concurrent-write and dirty-cache testing remains required.

### AUD-06 — live failure injection incomplete (MEDIUM, open)

Pending/failed uploads, cache exhaustion, network loss, suspend, SIGKILL, restart
with dirty cache, interrupted restic, and partial remote writes have not been
induced. These tests can affect data and require a controlled disposable setup.

### AUD-07 — deployment migration for public configuration (LOW, open)

Source now derives paths from `$HOME`, expects a generic SSH key name, and uses
`$HOME/HetznerDrive-Restores`. Existing installed service files still point to the
last verified binary until the owner explicitly rebuilds, previews, and reinstalls
integration with the correct `--ssh-key`. Old restore directories remain intact.

## Residual operational risk

- rclone/restic may retain their own transient secret copies.
- Configuration lock does not unmount FUSE or revoke already exposed plaintext.
- SSH key expiry follows the external keychain policy; the app does not load or
  unload keys.
- The append-only client intentionally cannot remove remote snapshots and has no
  retention policy.
- A mounted drive, notification, or local scan is not proof of a verified backup.

## Verdict

No CRITICAL/HIGH finding was observed. The explicit disposable MVP is functional,
and its secret/IPC/mount/backup controls have direct evidence. Routine unattended
use and autostart remain unapproved until supply-chain review and controlled
failure testing. The public release may be published as an auditable development
checkpoint once the reviewed MIT-licensed commit and remote revision are verified.
