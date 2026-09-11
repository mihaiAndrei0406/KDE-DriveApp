# Security audit gate

The application is not considered production-ready until this audit is completed,
findings are recorded and all critical/high findings are fixed or explicitly
accepted by the owner with a documented rationale. The audit happens after live
KDE/FUSE acceptance, before enabling routine autostart or relying on the controller
for important data.

## 1. Freeze and inventory

- Freeze the exact source, Cargo.lock, Python requirements lock, rclone binary
  version/hash, packaging output and test environment. Restore functional Git
  metadata or create an owner-approved repository so every audited change has a
  reviewable diff; the current workspace exposes an empty `.git` directory.
- Inventory every executable, file path, descriptor, environment variable, D-Bus
  method, socket, secret and network destination. Compare this inventory with the
  original DOCX and the current threat model.
- Define trust assumptions explicitly: normal user, hostile D-Bus caller under the
  same UID, compromised GUI, malicious configuration content, local unprivileged
  account, lost network, hostile remote responses and root/session compromise.

## 2. Static source review

- Review every Rust `unsafe` block, descriptor inheritance, mmap/mlock/zeroize
  lifetime, child process cleanup, PID/PPid checks and PID-reuse windows.
- Trace both the rclone config password and ephemeral RC password from creation to
  destruction. Search argv, environment, D-Bus, logs, errors, crash artifacts,
  temporary files and inherited descriptors for accidental copies.
- Audit all command construction and prove paths, remotes, flags and RC endpoints
  are fixed. Reject shell execution, config/global overrides, raw remote writes,
  destructive rclone verbs and unbounded subprocess input/output.
- Review filesystem operations for symlink/hardlink substitution, TOCTOU, owner
  and permission checks, mount namespace ambiguity, cache sharing and unsafe
  cleanup. Review all D-Bus schemas and GUI state transitions for confused-deputy
  behavior or stale status that could authorize a dangerous action.
- Review RC as a high-risk boundary. Confirm Unix-only binding, mode 0700/0600,
  global Basic authentication, bcrypt compatibility, sealed htpasswd memfd,
  endpoint allowlist, response limits/timeouts and cleanup after every exit path.
- Review packaging and systemd restrictions in the real user manager. Determine
  empirically whether NoNewPrivileges or another tighter restriction can coexist
  with Debian FUSE; do not weaken or enable options based only on assumptions.

## 3. Dependency and supply-chain review

- Run current Rust advisory, license and duplicate-dependency checks against the
  locked graph. Review build scripts and the small direct dependency set manually.
- Audit pinned Python wheels and hashes, editable installation behavior and QtDBus
  usage. Run a Python advisory scanner against the lock and record its database
  date and limitations.
- Verify rclone provenance, signature/hash and relevant security advisories for the
  exact installed version. Re-run RC regression tests if rclone changes.
- Record tool versions and complete commands so the audit can be reproduced.

## 4. Negative and dynamic testing

- Fuzz or property-test mountinfo, INI/redacted config, JSON/HTTP RC responses,
  D-Bus replies, Assuan pinentry data and sanitized log input with strict bounds.
- Exercise malicious config sections/casing/overrides, symlink swaps, wrong owners
  and modes, oversized files/responses, stalled children, early exits, forged
  password-helper peers, duplicate services and unexpected process death.
- Inspect `/proc/<pid>/cmdline`, environ, open descriptors, runtime paths, Unix/TCP
  listeners and journal output while locked, unlocked, mounted and shutting down.
  Use only synthetic secrets for this inspection.
- In a disposable live test path, exercise pending uploads, failed uploads, cache
  full, busy file handles, network loss/recovery, suspend/resume, logout/shutdown,
  SIGTERM/SIGKILL and restart with dirty cache. Verify hashes after recovery and
  prove the application never uses `hetzner-raw:` for user writes.
- Attempt direct calls from an untrusted same-UID D-Bus client and direct RC socket
  access without credentials. Confirm arbitrary commands, paths, endpoints and
  forced/lazy unmount cannot be reached.

## 5. Findings and release decision

- Record each finding with severity, evidence, affected asset, realistic attack
  path, fix, regression test and residual risk. Keep failed experiments as well as
  successful checks; remove all synthetic secrets and private filenames from the
  report.
- Re-run the full automated and manual suites after fixes. Review diffs created by
  audit fixes and repeat dependency scans. Critical/high findings block release.
- Produce a concise final report that states what was tested, what was not, which
  assumptions remain and whether autostart/installation can be enabled. A clean
  automated suite alone is not a security certification.

Live destructive or remote-writing audit cases require a separate, explicit test
scope and verified recoverable data. The normal encrypted directory is never used
as a disposable target.
