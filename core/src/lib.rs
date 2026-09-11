use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    ffi::CString,
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    os::{
        fd::FromRawFd,
        unix::fs::{MetadataExt, PermissionsExt},
    },
    path::{Component, Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::{io::AsyncReadExt, process::Command};

pub mod auth;
pub mod backup;
pub mod controller;
pub mod mount;
pub mod secrets;

pub const BUS: &str = "ro.mihai.HetznerDrive1";
pub const OBJECT: &str = "/ro/mihai/HetznerDrive1";
pub const REMOTE: &str = "hetzner-crypt:";
const OUTPUT_LIMIT: usize = 65536;

#[derive(Clone)]
pub struct Paths {
    pub mount: PathBuf,
    pub config: PathBuf,
    pub key_file: PathBuf,
    pub sftp_host: String,
    pub sftp_user: String,
    pub public_key: PathBuf,
    pub cache: PathBuf,
    pub log: PathBuf,
}

impl Default for Paths {
    fn default() -> Self {
        let home = user_home();
        let key_file = std::env::var_os("HETZNER_DRIVE_SSH_KEY")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".ssh/hetzner_storagebox"));
        let mut public_key_name = key_file.as_os_str().to_os_string();
        public_key_name.push(".pub");
        Self {
            mount: home.join("HetznerDrive"),
            config: home.join(".config/rclone/rclone.conf"),
            key_file,
            sftp_host: std::env::var("HETZNER_DRIVE_SFTP_HOST").unwrap_or_default(),
            sftp_user: std::env::var("HETZNER_DRIVE_SFTP_USER").unwrap_or_default(),
            public_key: PathBuf::from(public_key_name),
            cache: home.join(".cache/rclone/hetzner-vfs"),
            log: home.join(".local/state/rclone/hetzner-mount.log"),
        }
    }
}

pub fn user_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| validate_path(path))
        .unwrap_or_else(|| "/nonexistent".into())
}

pub fn validate_path(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|c| matches!(c, Component::RootDir | Component::Normal(_)))
        && !path.as_os_str().as_encoded_bytes().contains(&0)
}

pub fn validate_remote(remote: &str) -> bool {
    remote == REMOTE
}

pub(crate) fn no_symlink_components(path: &Path) -> bool {
    if !validate_path(path) {
        return false;
    }
    let mut current = PathBuf::new();
    for part in path.components() {
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(m) if !m.file_type().is_symlink() => (),
            _ => return false,
        }
    }
    true
}

pub fn validate_private_directory(path: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    metadata.is_dir()
        && metadata.uid() == unsafe { libc::geteuid() }
        && metadata.permissions().mode() & 0o077 == 0
        && no_symlink_components(path)
}

pub fn remote_mount_targets(contents: &str) -> Result<Vec<PathBuf>, &'static str> {
    let mut targets = Vec::new();
    for line in contents.lines() {
        let (before, after) = line.split_once(" - ").ok_or("mount_state_unknown")?;
        let fields: Vec<_> = before.split_whitespace().collect();
        let backend: Vec<_> = after.split_whitespace().collect();
        if fields.len() < 6 || backend.len() < 3 {
            return Err("mount_state_unknown");
        }
        if backend[0] == "fuse.rclone" && backend[1] == REMOTE {
            targets.push(PathBuf::from(
                unescape_mount(fields[4]).ok_or("mount_state_unknown")?,
            ));
        }
    }
    Ok(targets)
}

pub(crate) fn open_owned_file(path: &Path) -> std::io::Result<File> {
    if !validate_path(path) {
        return Err(std::io::ErrorKind::PermissionDenied.into());
    }
    let path = CString::new(path.as_os_str().as_encoded_bytes())?;
    let mut how: libc::open_how = unsafe { std::mem::zeroed() };
    how.flags = (libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NONBLOCK) as u64;
    how.resolve = libc::RESOLVE_NO_SYMLINKS;
    // Resolve every component atomically without symlinks; fail closed on older kernels.
    let fd = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            libc::AT_FDCWD,
            path.as_ptr(),
            &how,
            std::mem::size_of::<libc::open_how>(),
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let file = unsafe { File::from_raw_fd(fd as i32) };
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o022 != 0
    {
        return Err(std::io::ErrorKind::PermissionDenied.into());
    }
    Ok(file)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum State {
    Locked,
    Ready,
    Mounting,
    Mounted,
    Unmounting,
    Degraded,
    Error,
}

impl State {
    pub fn transition(self, next: Self) -> Result<Self, &'static str> {
        use State::*;
        let allowed = matches!(
            (self, next),
            (Locked, Ready | Mounted | Degraded | Error)
                | (Ready, Locked | Mounting | Mounted | Degraded | Error)
                | (Mounting, Mounted | Locked | Degraded | Error)
                | (Mounted, Unmounting | Degraded | Error)
                | (Unmounting, Ready | Locked | Degraded | Error)
                | (Degraded, Locked | Ready | Mounted | Error)
                | (Error, Locked | Ready | Mounted | Degraded)
        );
        if allowed {
            Ok(next)
        } else {
            Err("invalid_transition")
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Version,
    AgentKeys,
    PublicFingerprint,
}

impl Operation {
    pub fn command(self, paths: &Paths) -> (&'static str, Vec<String>) {
        let (program, args): (_, Vec<&str>) = match self {
            Self::Version => (
                "/usr/local/bin/rclone",
                vec!["version", "--config", "/dev/null"],
            ),
            Self::AgentKeys => ("/usr/bin/ssh-add", vec!["-l", "-E", "sha256"]),
            Self::PublicFingerprint => ("/usr/bin/ssh-keygen", vec!["-l", "-E", "sha256", "-f"]),
        };
        let mut args: Vec<String> = args.into_iter().map(str::to_owned).collect();
        if self == Self::PublicFingerprint {
            args.push(paths.public_key.to_string_lossy().into());
        }
        (program, args)
    }
}

#[derive(Debug)]
pub struct Output {
    pub code: i32,
    pub stdout: String,
}

#[async_trait]
pub trait Runner: Send + Sync {
    async fn run(&self, operation: Operation, paths: &Paths) -> Result<Output, &'static str>;
}

pub struct ProcessRunner;

async fn bounded_read<R: tokio::io::AsyncRead + Unpin>(reader: R) -> Result<Vec<u8>, &'static str> {
    let mut bytes = Vec::new();
    reader
        .take((OUTPUT_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| "process_io")?;
    if bytes.len() > OUTPUT_LIMIT {
        Err("output_limit")
    } else {
        Ok(bytes)
    }
}

async fn execute(mut command: Command, deadline: Duration) -> Result<Output, &'static str> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|_| "process_start_failed")?;
    let stdout = child.stdout.take().ok_or("process_io")?;
    let stderr = child.stderr.take().ok_or("process_io")?;
    let work = async {
        let (status, out, _) = tokio::try_join!(
            async { child.wait().await.map_err(|_| "process_wait_failed") },
            bounded_read(stdout),
            bounded_read(stderr)
        )?;
        Ok(Output {
            code: status.code().unwrap_or(-1),
            stdout: String::from_utf8(out).map_err(|_| "invalid_output")?,
        })
    };
    let result = tokio::time::timeout(deadline, work)
        .await
        .unwrap_or(Err("process_timeout"));
    if result.is_err() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    result
}

#[async_trait]
impl Runner for ProcessRunner {
    async fn run(&self, operation: Operation, paths: &Paths) -> Result<Output, &'static str> {
        if operation == Operation::PublicFingerprint {
            // Pin the public file by descriptor; ssh-keygen never opens a substituted private path.
            let file = open_owned_file(&paths.public_key).map_err(|_| "public_key_unavailable")?;
            use std::os::fd::AsRawFd;
            let mut command = Command::new("/usr/bin/ssh-keygen");
            command
                .env_clear()
                .env("LC_ALL", "C")
                .args(["-l", "-E", "sha256", "-f"])
                .arg(format!("/proc/self/fd/{}", file.as_raw_fd()));
            secrets::inherit_fd(&mut command, file.as_raw_fd());
            let result = execute(command, Duration::from_secs(3)).await;
            drop(file);
            return result;
        }
        let (program, args) = operation.command(paths);
        let mut command = Command::new(program);
        command.args(args).env_clear().env("LC_ALL", "C");
        if operation == Operation::AgentKeys {
            let socket = std::env::var_os("SSH_AUTH_SOCK").ok_or("agent_unavailable")?;
            command.env("SSH_AUTH_SOCK", socket);
        }
        execute(command, Duration::from_secs(3)).await
    }
}

fn fingerprints(text: &str) -> HashSet<&str> {
    text.lines()
        .filter_map(|line| {
            let fingerprint = line.split_whitespace().nth(1)?;
            let value = fingerprint.strip_prefix("SHA256:")?;
            (value.len() == 43
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/'))
            .then_some(fingerprint)
        })
        .collect()
}

pub async fn check_agent(runner: &impl Runner, paths: &Paths) -> String {
    let expected = match runner.run(Operation::PublicFingerprint, paths).await {
        Ok(out) if out.code == 0 => out.stdout,
        _ => return "public_key_unavailable".into(),
    };
    let expected = fingerprints(&expected);
    if expected.len() != 1 {
        return "public_key_invalid".into();
    }
    match runner.run(Operation::AgentKeys, paths).await {
        Ok(out) if out.code == 0 => {
            if fingerprints(&out.stdout)
                .intersection(&expected)
                .next()
                .is_some()
            {
                "loaded"
            } else {
                "key_missing"
            }
        }
        Ok(out) if out.code == 1 => "key_missing",
        Ok(out) if out.code == 2 => "agent_unavailable",
        Err("agent_unavailable") => "agent_unavailable",
        Err("process_timeout") => "agent_timeout",
        _ => "agent_error",
    }
    .into()
}

fn unescape_mount(text: &str) -> Option<String> {
    let mut out = Vec::new();
    let mut index = 0;
    let bytes = text.as_bytes();
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            let code = bytes.get(index + 1..index + 4)?;
            out.push(match code {
                b"040" => b' ',
                b"011" => b'\t',
                b"012" => b'\n',
                b"134" => b'\\',
                _ => return None,
            });
            index += 4;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

pub fn mount_observation(contents: &str, mount: &Path) -> &'static str {
    let mut found = None;
    for line in contents.lines() {
        let Some((before, after)) = line.split_once(" - ") else {
            return "unknown";
        };
        let fields: Vec<_> = before.split_whitespace().collect();
        let backend: Vec<_> = after.split_whitespace().collect();
        if fields.len() < 6 || backend.len() < 3 {
            return "unknown";
        }
        let Some(path) = unescape_mount(fields[4]) else {
            return "unknown";
        };
        if Path::new(&path) == mount {
            if found.is_some() {
                return "conflict";
            }
            found = Some(if backend[0] == "fuse.rclone" && backend[1] == REMOTE {
                "mounted"
            } else {
                "conflict"
            });
        }
    }
    found.unwrap_or("unmounted")
}

fn config_observation(path: &Path) -> &'static str {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "missing",
        Err(_) => "unavailable",
        Ok(metadata) => {
            if !metadata.is_file() || !no_symlink_components(path) {
                "unsafe_path"
            } else if metadata.uid() != unsafe { libc::geteuid() }
                || metadata.permissions().mode() & 0o777 != 0o600
            {
                "unsafe_permissions"
            } else {
                "present_not_unlocked"
            }
        }
    }
}

pub fn cache_allocation(path: &Path) -> (u64, bool) {
    if !no_symlink_components(path) {
        return (0, false);
    }
    let mut stack = vec![path.to_path_buf()];
    let start = Instant::now();
    let mut count = 0;
    let mut bytes: u64 = 0;
    let mut complete = true;
    while let Some(path) = stack.pop() {
        count += 1;
        if count > 10000 || start.elapsed() > Duration::from_millis(150) {
            return (bytes, false);
        }
        let Ok(meta) = fs::symlink_metadata(&path) else {
            complete = false;
            continue;
        };
        if meta.file_type().is_symlink() {
            complete = false;
            continue;
        }
        if meta.is_file() {
            bytes = bytes.saturating_add(meta.blocks().saturating_mul(512));
        } else if meta.is_dir() {
            let Ok(entries) = fs::read_dir(path) else {
                complete = false;
                continue;
            };
            for entry in entries {
                if stack.len() + count >= 10000 {
                    return (bytes, false);
                }
                match entry {
                    Ok(e) => stack.push(e.path()),
                    Err(_) => complete = false,
                }
            }
        } else {
            complete = false;
        }
    }
    (bytes, complete)
}

pub fn safe_log_events(text: &str) -> Vec<String> {
    // Reconstruct events from an allowlist. Never return raw messages or filenames.
    text.lines()
        .filter_map(|line| {
            let event = if line.contains("vfs cache:") && line.contains("upload succeeded") {
                "INFO Upload completed"
            } else if line.contains("ERROR :") || line.contains("ERROR:") {
                "ERROR rclone reported an error; details withheld"
            } else if line.contains("NOTICE:") || line.contains("NOTICE :") {
                "NOTICE rclone notice; details withheld"
            } else {
                return None;
            };
            let stamp = line.get(..19).filter(|s| {
                s.bytes().enumerate().all(|(i, b)| match i {
                    4 | 7 => b == b'/',
                    10 => b == b' ',
                    13 | 16 => b == b':',
                    _ => b.is_ascii_digit(),
                })
            });
            Some(match stamp {
                Some(s) => format!("{s} {event}"),
                None => event.into(),
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .take(50)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

pub fn recent_logs(path: &Path) -> Vec<String> {
    let result = (|| -> std::io::Result<Vec<String>> {
        let mut file = open_owned_file(path)?;
        let start = file.metadata()?.len().saturating_sub(OUTPUT_LIMIT as u64);
        file.seek(SeekFrom::Start(start))?;
        let mut bytes = Vec::new();
        file.take(OUTPUT_LIMIT as u64).read_to_end(&mut bytes)?;
        let text = String::from_utf8_lossy(&bytes);
        let text = if start > 0 {
            text.split_once('\n').map(|(_, tail)| tail).unwrap_or("")
        } else {
            &text
        };
        Ok(safe_log_events(text))
    })();
    match result {
        Ok(events) if !events.is_empty() => events,
        Ok(_) => vec!["No recognized events in the recent log window".into()],
        Err(_) => vec!["Log unavailable or file validation failed".into()],
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub state: State,
    pub mount: String,
    pub ssh: String,
    pub config: String,
    pub version: String,
    pub mount_path: String,
    pub cache_bytes: u64,
    pub cache_complete: bool,
    pub diagnostic: String,
}

impl Snapshot {
    pub fn demo() -> Self {
        Self {
            state: State::Locked,
            mount: "unmounted".into(),
            ssh: "loaded".into(),
            config: "present_not_unlocked".into(),
            version: "rclone v1.75.1".into(),
            mount_path: Paths::default().mount.to_string_lossy().into_owned(),
            cache_bytes: 0,
            cache_complete: true,
            diagnostic: "demo_data".into(),
        }
    }
}

pub async fn snapshot(runner: &impl Runner, paths: &Paths, mountinfo: &str) -> Snapshot {
    let mount = mount_observation(mountinfo, &paths.mount).to_owned();
    let ssh = check_agent(runner, paths).await;
    let config = config_observation(&paths.config).to_owned();
    let version = match runner.run(Operation::Version, paths).await {
        Ok(out) if out.code == 0 => out
            .stdout
            .lines()
            .next()
            .filter(|s| {
                s.starts_with("rclone v")
                    && s.len() < 80
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b" .+-".contains(&b))
            })
            .unwrap_or("unavailable")
            .to_owned(),
        _ => "unavailable".into(),
    };
    let (cache_bytes, cache_complete) = cache_allocation(&paths.cache);
    let state = if mount == "conflict"
        || mount == "unknown"
        || config != "present_not_unlocked"
        || version == "unavailable"
    {
        State::Error
    } else if mount == "mounted" {
        if ssh == "loaded" {
            State::Mounted
        } else {
            State::Degraded
        }
    } else {
        State::Locked
    };
    let mut diagnostic = Vec::new();
    if mount == "mounted" {
        diagnostic.push("mount_observed_network_unverified");
    }
    if !cache_complete {
        diagnostic.push("cache_scan_incomplete");
    }
    if ssh != "loaded" {
        diagnostic.push("ssh_requires_attention");
    }
    if config != "present_not_unlocked" {
        diagnostic.push("config_requires_attention");
    }
    if version == "unavailable" {
        diagnostic.push("rclone_unavailable");
    }
    Snapshot {
        state,
        mount,
        ssh,
        config,
        version,
        mount_path: paths.mount.to_string_lossy().into(),
        cache_bytes,
        cache_complete,
        diagnostic: diagnostic.join(","),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Mock {
        calls: Mutex<Vec<Operation>>,
        agent: &'static str,
        code: i32,
    }
    const EXPECTED: &str =
        "256 SHA256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa fixture (ED25519)";
    #[async_trait]
    impl Runner for Mock {
        async fn run(&self, op: Operation, _: &Paths) -> Result<Output, &'static str> {
            self.calls.lock().unwrap().push(op);
            Ok(Output {
                code: if op == Operation::AgentKeys {
                    self.code
                } else {
                    0
                },
                stdout: match op {
                    Operation::AgentKeys => self.agent,
                    Operation::PublicFingerprint => EXPECTED,
                    Operation::Version => "rclone v1.75.1\n- os/type: linux",
                }
                .into(),
            })
        }
    }

    #[test]
    fn rejects_invalid_transitions() {
        assert!(State::Ready.transition(State::Unmounting).is_err());
        assert!(State::Mounting.transition(State::Mounting).is_err());
        assert_eq!(
            State::Ready.transition(State::Mounting),
            Ok(State::Mounting)
        );
        assert_eq!(
            State::Mounting.transition(State::Mounted),
            Ok(State::Mounted)
        );
    }

    #[test]
    fn validates_paths_and_remote() {
        for p in ["relative", "/home/example/../secret", "/tmp/\0"] {
            assert!(!validate_path(Path::new(p)));
        }
        assert!(validate_path(Path::new("/home/example/HetznerDrive")));
        for r in [
            "hetzner-raw:",
            "hetzner-crypt:;touch /tmp/x",
            ":sftp:",
            "hetzner-crypt:folder",
        ] {
            assert!(!validate_remote(r));
        }
        assert!(validate_remote(REMOTE));
    }

    #[test]
    fn identifies_exact_mount_and_conflicts() {
        let path = Path::new("/home/example/HetznerDrive");
        let line = "12 1 0:1 / /home/example/HetznerDrive rw - fuse.rclone hetzner-crypt: rw\n";
        assert_eq!(mount_observation(line, path), "mounted");
        assert_eq!(
            mount_observation(&line.replace("hetzner-crypt:", "hetzner-raw:"), path),
            "conflict"
        );
        assert_eq!(
            mount_observation(&line.replace("HetznerDrive", "HetznerDrive2"), path),
            "unmounted"
        );
        assert_eq!(
            mount_observation(&(line.to_owned() + line), path),
            "conflict"
        );
        assert_eq!(mount_observation("malformed", path), "unknown");
        assert_eq!(
            mount_observation(
                "1 2 0:1 / /tmp/a\\040b rw - fuse.rclone hetzner-crypt: rw",
                Path::new("/tmp/a b")
            ),
            "mounted"
        );
        assert_eq!(remote_mount_targets(line).unwrap(), vec![path]);
        let elsewhere = line.replace("HetznerDrive", "Elsewhere");
        assert_eq!(
            remote_mount_targets(&elsewhere).unwrap(),
            vec![PathBuf::from("/home/example/Elsewhere")]
        );
        assert!(remote_mount_targets("malformed").is_err());
    }

    #[tokio::test]
    async fn requires_the_dedicated_agent_key() {
        let other = "256 SHA256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb fixture (ED25519)";
        let mock = Mock {
            calls: Mutex::new(vec![]),
            agent: other,
            code: 0,
        };
        assert_eq!(check_agent(&mock, &Paths::default()).await, "key_missing");
        let mock = Mock {
            calls: Mutex::new(vec![]),
            agent: EXPECTED,
            code: 0,
        };
        assert_eq!(check_agent(&mock, &Paths::default()).await, "loaded");
        let mock = Mock {
            calls: Mutex::new(vec![]),
            agent: "",
            code: 2,
        };
        assert_eq!(
            check_agent(&mock, &Paths::default()).await,
            "agent_unavailable"
        );
    }

    #[tokio::test]
    async fn kills_and_reaps_timed_out_process() {
        let mut command = Command::new("/usr/bin/sleep");
        command.arg("10");
        let start = Instant::now();
        assert_eq!(
            execute(command, Duration::from_millis(30))
                .await
                .unwrap_err(),
            "process_timeout"
        );
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn bounds_process_output() {
        let mut command = Command::new("/usr/bin/head");
        command.args(["-c", "70000", "/dev/zero"]);
        assert_eq!(
            execute(command, Duration::from_secs(1)).await.unwrap_err(),
            "output_limit"
        );
    }

    #[test]
    fn command_surface_is_read_only() {
        for op in [
            Operation::Version,
            Operation::AgentKeys,
            Operation::PublicFingerprint,
        ] {
            let (program, args) = op.command(&Paths::default());
            assert!(program.starts_with('/'));
            for forbidden in [
                "sync", "mount", "delete", "purge", "move", "sh", "bash", "-c",
            ] {
                assert!(!args.iter().any(|a| a == forbidden));
            }
        }
    }

    #[test]
    fn never_discloses_raw_log_secrets_or_paths() {
        let secret = "fake_fixture_secret";
        let private_key_marker = ["-----BEGIN OPENSSH", "PRIVATE KEY-----"].join(" ");
        let logs = format!(
            "2026/09/09 12:20:30 ERROR : password={secret}\ncontinuation {secret}\n2026/09/09 12:20:31 INFO : private-file: vfs cache: upload succeeded\n{private_key_marker}\n{secret}\n"
        );
        let events = safe_log_events(&logs).join("\n");
        assert!(!events.contains(secret));
        assert!(!events.contains("private-file"));
        assert!(!events.contains("PRIVATE KEY"));
        assert!(events.contains("2026/09/09 12:20:30 ERROR"));
    }

    #[test]
    fn rejects_symlink_log_and_wrong_config_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("fixture");
        fs::write(&target, "ERROR : fixture").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(recent_logs(&link)[0].starts_with("Log unavailable"));
        let parent_link = dir.path().join("parent-link");
        std::os::unix::fs::symlink(dir.path(), &parent_link).unwrap();
        assert!(open_owned_file(&parent_link.join("fixture")).is_err());
        assert_eq!(config_observation(&target), "present_not_unlocked");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(config_observation(&target), "unsafe_permissions");
    }

    #[test]
    fn private_directories_require_owner_mode_and_no_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        assert!(validate_private_directory(dir.path()));
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o750)).unwrap();
        assert!(!validate_private_directory(dir.path()));
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let link = dir.path().with_extension("link");
        std::os::unix::fs::symlink(dir.path(), &link).unwrap();
        assert!(!validate_private_directory(&link));
        fs::remove_file(link).unwrap();
    }

    #[tokio::test]
    async fn fingerprint_process_reads_only_the_pinned_public_fixture() {
        let dir = tempfile::tempdir().unwrap();
        let public_key = dir.path().join("fixture.pub");
        fs::write(&public_key, "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA fixture\n").unwrap();
        fs::set_permissions(&public_key, fs::Permissions::from_mode(0o600)).unwrap();
        let paths = Paths {
            public_key,
            ..Paths::default()
        };
        let output = ProcessRunner
            .run(Operation::PublicFingerprint, &paths)
            .await
            .unwrap();
        assert_eq!(output.code, 0);
        assert_eq!(fingerprints(&output.stdout).len(), 1);
    }

    #[test]
    fn cache_skips_symlinks_and_uses_allocated_size() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sparse");
        File::create(&file).unwrap().set_len(1024 * 1024).unwrap();
        let (bytes, complete) = cache_allocation(dir.path());
        assert_eq!(bytes, fs::metadata(&file).unwrap().blocks() * 512);
        assert!(complete);
        std::os::unix::fs::symlink("/", dir.path().join("outside")).unwrap();
        assert!(!cache_allocation(dir.path()).1);
    }

    #[tokio::test]
    async fn snapshot_never_claims_config_is_unlocked() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config");
        fs::write(&config, "fixture-content-not-read").unwrap();
        fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
        let paths = Paths {
            config,
            cache: dir.path().to_path_buf(),
            ..Paths::default()
        };
        let mock = Mock {
            calls: Mutex::new(vec![]),
            agent: EXPECTED,
            code: 0,
        };
        let status = snapshot(&mock, &paths, "").await;
        assert_eq!(status.state, State::Locked);
        assert_eq!(status.config, "present_not_unlocked");
        assert_eq!(mock.calls.lock().unwrap().len(), 3);
    }
}
