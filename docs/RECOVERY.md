# Recovery and rollback

Development is contained in this project and its `.venv` and `target` folders.
No autostart, systemd enablement, rclone config edit or live cloud write is
performed by the build/test commands. The user-level integration was explicitly
installed on 2026-09-10 with autostart disabled. It remains removable through the
confined packaging installer without deleting configuration, keys, cache or data.

Close the GUI using its tray Exit action; the service and its owned mount continue.
Stopping the core requests a checked unmount and waits for queued uploads for up
to 270 seconds. If RC reports errors or an unknown state, shutdown is refused
until the service manager's five-minute hard timeout; dirty VFS cache is preserved.
Existing fallback commands remain:

```text
hetzner-mount
hetzner-unmount
```

Follow the original DOCX for unlocking SSH and rclone, checking uploads and
restoring encrypted configuration. Do not delete the VFS cache as troubleshooting:
it may contain writes not yet uploaded. Do not generate new crypt credentials
over the existing encrypted-data directory.

After an unexpected controller/rclone crash, keep the cache unchanged. Start the
application with the same remote, mount point and cache settings; rclone documents
that dirty cached files are retried on a later identical mount. Confirm the VFS
queue and errors before using or unmounting the recovered drive.

For an update, stop only this controller, retain the previous app build and
rebuild from the pinned locks. If it fails, run the previous build or use the
existing scripts. Uninstalling the development app only involves this project's
generated artifacts if integration was never installed. If it was, first use
`.venv/bin/python packaging/install.py uninstall --apply` from the installed
checkout; this removes only the four managed integration files and reloads user
systemd without stopping the service or mount. Lock the session and stop the
controller explicitly before removing its checkout. Preserve documentation and
source. Never remove the SSH
key, rclone config, mount directory or VFS cache as part of uninstalling it.
