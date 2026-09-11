# Safe unmount and rclone observability

Status: the user explicitly approved the private RC exception on 2026-09-09. It
is implemented only for mounts owned by the application and has passed a live
idle mount/unmount acceptance.

## Verified problem

rclone 1.75.1 keeps an upload queue inside the mount process. Process exit does
not prove that the queue was flushed. Cache size and logs cannot atomically expose
pending uploads, while on-disk VFS metadata may lag and is not a stable control API.

The official `vfs/stats` API exposes `uploadsQueued`, `uploadsInProgress`,
`erroredFiles`, and `outOfSpace`. RC uses HTTP semantics even over a Unix socket.
Because the original requirements prohibited a local HTTP server, this mechanism
required an explicit exception.

## Approved and implemented exception

- GUI ↔ Rust communication remains fixed-method D-Bus with no generic command API.
- Only an application-owned rclone process receives RC, on a Unix socket inside a
  0700 directory below `XDG_RUNTIME_DIR`; the socket is 0600.
- There is no TCP listener, web interface, Web GUI, `serve`, or `rc-no-auth`.
- Rust calls only allowlisted internal VFS observation endpoints. D-Bus cannot
  provide arbitrary RC endpoints or arguments.
- Each mount receives a random 256-bit RC credential in locked memory. rclone gets
  only its bcrypt hash through a sealed memfd. Clear credentials never appear in
  argv, environment, or persistent files. Requests/responses are bounded and timed.
- Before sending authentication, the controller verifies that the Unix peer UID
  and exact PID match its owned rclone child.
- The application never connects to RC endpoints belonging to external mounts.

RC itself has a broad control surface; compromise of the same user's session is
outside the controller's security boundary. The private endpoint is therefore not
exported as an application API.

## Unmount protocol

1. Confirm the exact mount and that it belongs to this application.
2. Ask the user to close documents and confirm unmount in the GUI.
3. Read queue, active uploads, errors, and out-of-space state twice, 750 ms apart.
4. Revalidate state immediately before `fusermount3 -u`, with no force/lazy mode.
5. Keep the drive mounted when it is busy or any state is unknown, with a stable
   user-facing explanation.
6. Confirm mount disappearance and reap the process. Never delete VFS cache.

No pre-unmount check prevents another desktop process from starting a concurrent
write. The UX states this limitation; it does not promise an atomic transaction
between rclone and all applications.

## Residual limit

RC does not provide endpoint-level permission separation for `vfs/stats`;
authentication protects the entire API. Final controlled failure tests must still
cover pending writes, dirty cache, network loss, suspend, and process crashes.

References verified on 2026-09-09:

- <https://rclone.org/rc/#security>
- <https://rclone.org/rc/#vfs-stats>
- <https://rclone.org/commands/rclone_rc/>
- <https://rclone.org/commands/rclone_mount/#vfs-file-caching>
- <https://github.com/rclone/rclone/blob/v1.75.1/vfs/vfs.go>
- <https://github.com/rclone/rclone/blob/v1.75.1/vfs/vfscache/item.go>
