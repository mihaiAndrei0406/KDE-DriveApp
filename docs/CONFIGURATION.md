# Local configuration contract

This application deliberately does not edit or generate the user's rclone setup.
It validates one existing encrypted configuration and fails closed when its
provider, remote chain, paths, permissions, or security options differ.

## Required local tools and paths

- rclone executable: `/usr/local/bin/rclone`
- restic executable: `/usr/bin/restic`
- encrypted config: `$HOME/.config/rclone/rclone.conf`, regular file, owned by the
  desktop user, mode 0600, with no symlink component
- mount directory: `$HOME/HetznerDrive`, empty, owned by the user, mode 0700
- VFS cache: `$HOME/.cache/rclone/hetzner-vfs`, owned by the user, mode 0700
- backup cache: `$HOME/.cache/hetzner-drive-restic`, created privately as needed
- restore root: `$HOME/HetznerDrive-Restores`, created privately as needed

The default private-key path is `$HOME/.ssh/hetzner_storagebox`; its matching
`.pub` file must exist for fingerprint comparison with the SSH agent. A different
filename is supported only through `HETZNER_DRIVE_SSH_KEY` (direct backend runs)
or installer `--ssh-key`. Direct backend runs also require
`HETZNER_DRIVE_SFTP_HOST` and `HETZNER_DRIVE_SFTP_USER`. The key path must be an
absolute direct child of `$HOME/.ssh`, use only ASCII letters, digits, and `/._-`,
and the exact same value must appear as `key_file` in rclone config.

## Required remote structure

Remote names are case-sensitive and fixed because they are part of the D-Bus and
command policy. The following is a structural example only; never commit actual
obscured passwords, salts, account IDs, or host pins.

```ini
[hetzner-crypt]
type = crypt
remote = hetzner-raw:encrypted-data
password = <rclone-obscured-crypt-password>
password2 = <rclone-obscured-crypt-salt>
filename_encryption = standard
directory_name_encryption = true

[hetzner-raw]
type = sftp
host = u12345.your-storagebox.de
user = u12345
port = 23
key_use_agent = true
key_file = /home/example/.ssh/hetzner_storagebox
shell_type = unix
host_keys = ssh-ed25519 <pinned-server-public-key>
```

Storage Box subaccount users such as `u12345-backups` are accepted only when the
host remains `u12345.your-storagebox.de`. The host/account relationship, port 23,
agent authentication, exact key path, and non-empty host pin are validated after
unlock. External SSH commands, password/private-key material in SFTP options,
proxies, insecure ciphers, and `global.*`/`override.*` entries are rejected.

Encrypt the completed configuration using rclone's supported configuration
encryption workflow. Enter only that configuration password in the application's
rclone unlock dialog. The crypt password/salt and restic repository password are
different credential classes; see [Authentication](AUTHENTICATION.md).

## Desktop installation with a custom key

Preview and install with the same path:

```bash
.venv/bin/python packaging/install.py plan \
  --sftp-host u12345.your-storagebox.de --sftp-user u12345 \
  --ssh-key "$HOME/.ssh/my_storagebox_key"
.venv/bin/python packaging/install.py install --apply \
  --sftp-host u12345.your-storagebox.de --sftp-user u12345 \
  --ssh-key "$HOME/.ssh/my_storagebox_key"
```

The installer renders these non-secret policy values into the private
user-systemd service file. It never reads the private key. Rebuilding source does
not update installed files; preview and explicit install are always separate actions.
Uninstall does not need the Storage Box identity because it only resolves the four
fixed managed destinations:

```bash
.venv/bin/python packaging/install.py uninstall --apply
```
