//! Owned rclone mount lifecycle and its authenticated, Unix-only RC channel.
use crate::{
    Paths, REMOTE,
    auth::{self, EncryptedConfig, PasswordChannel},
    mount_observation,
    secrets::{self, SecureBytes, SessionSecret},
};
use bcrypt::hash_with_salt;
use serde::Deserialize;
use std::{
    fs::{self, File},
    io::{Seek, SeekFrom, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
    },
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
    process::{Child, Command},
};
use zeroize::Zeroizing;

const RC_USER: &str = "hetzner-drive";
const RC_BCRYPT_COST: u32 = 10;
const RC_OUTPUT_LIMIT: usize = 64 * 1024;

pub struct OwnedMount {
    child: Child,
    rc: RcRuntime,
}

impl OwnedMount {
    pub fn process_id(&self) -> Option<u32> {
        self.child.id()
    }

    pub fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>, &'static str> {
        self.child
            .try_wait()
            .map_err(|_| "mount_process_wait_failed")
    }

    pub async fn stats(&self) -> Result<VfsStats, &'static str> {
        self.rc.stats().await
    }

    pub async fn wait(&mut self, duration: Duration) -> Result<(), &'static str> {
        match tokio::time::timeout(duration, self.child.wait()).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(_)) => Err("mount_process_wait_failed"),
            Err(_) => Err("mount_process_still_running"),
        }
    }

    pub async fn terminate(&mut self) {
        if let Some(pid) = self.child.id() {
            unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            }
            if self.wait(Duration::from_secs(5)).await.is_ok() {
                return;
            }
        }
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }
}

struct RcRuntime {
    directory: PathBuf,
    socket: PathBuf,
    htpasswd: File,
    password: Arc<SessionSecret>,
    expected_pid: AtomicU32,
}

impl RcRuntime {
    async fn new() -> Result<Self, &'static str> {
        let base = secrets::trusted_runtime()?;
        let directory = base.join(format!("hetzner-drive-{}", secrets::random_name()?));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .map_err(|_| "rc_runtime_failed")?;
        let result = Self::create_in(directory.clone()).await;
        if result.is_err() {
            let _ = fs::remove_dir(&directory);
        }
        result
    }

    async fn create_in(directory: PathBuf) -> Result<Self, &'static str> {
        let metadata = fs::symlink_metadata(&directory).map_err(|_| "rc_runtime_failed")?;
        if !metadata.is_dir()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err("rc_runtime_failed");
        }
        let mut random = Zeroizing::new([0u8; 48]);
        if unsafe { libc::getrandom(random.as_mut_ptr().cast(), random.len(), 0) }
            != random.len() as isize
        {
            return Err("random_unavailable");
        }
        let password_text = Zeroizing::new(hex(&random[..32]));
        let password = SessionSecret::new(password_text.as_bytes())?;
        let salt: [u8; 16] = random[32..]
            .try_into()
            .map_err(|_| "rc_authentication_failed")?;
        let password_for_hash = password.clone();
        let hash = tokio::task::spawn_blocking(move || {
            hash_with_salt(password_for_hash.expose(), RC_BCRYPT_COST, salt)
                .map(|parts| parts.to_string())
        })
        .await
        .map_err(|_| "rc_authentication_failed")?
        .map_err(|_| "rc_authentication_failed")?;

        let fd = unsafe {
            libc::memfd_create(
                c"hetzner-rc-auth".as_ptr(),
                libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
            )
        };
        if fd < 0 {
            return Err("rc_authentication_failed");
        }
        let mut htpasswd = unsafe { File::from_raw_fd(fd) };
        writeln!(htpasswd, "{RC_USER}:{hash}").map_err(|_| "rc_authentication_failed")?;
        htpasswd
            .seek(SeekFrom::Start(0))
            .map_err(|_| "rc_authentication_failed")?;
        if unsafe {
            libc::fcntl(
                fd,
                libc::F_ADD_SEALS,
                libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL,
            )
        } < 0
        {
            return Err("rc_authentication_failed");
        }
        Ok(Self {
            socket: directory.join("rc.sock"),
            directory,
            htpasswd,
            password,
            expected_pid: AtomicU32::new(0),
        })
    }

    fn set_process(&self, pid: u32) {
        self.expected_pid.store(pid, Ordering::Relaxed);
    }

    fn configure(&self, command: &mut Command) {
        command.arg("--rc");
        self.configure_options(command);
    }

    fn configure_options(&self, command: &mut Command) {
        secrets::inherit_fd(command, self.htpasswd.as_raw_fd());
        command.args([
            "--rc-addr",
            &format!("unix://{}", self.socket.display()),
            "--rc-htpasswd",
            &format!("/proc/self/fd/{}", self.htpasswd.as_raw_fd()),
            "--rc-max-header-bytes",
            "4096",
            "--rc-server-read-timeout",
            "5s",
            "--rc-server-write-timeout",
            "5s",
        ]);
    }

    async fn stats(&self) -> Result<VfsStats, &'static str> {
        let body = self.request("vfs/stats").await?;
        serde_json::from_slice(&body).map_err(|_| "rc_response_invalid")
    }

    async fn request(&self, endpoint: &str) -> Result<SecureBytes, &'static str> {
        tokio::time::timeout(Duration::from_secs(7), self.request_inner(endpoint))
            .await
            .map_err(|_| "rc_timeout")?
    }

    async fn request_inner(&self, endpoint: &str) -> Result<SecureBytes, &'static str> {
        if endpoint.is_empty()
            || !endpoint
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'/')
        {
            return Err("rc_endpoint_invalid");
        }
        let mut stream =
            tokio::time::timeout(Duration::from_secs(3), UnixStream::connect(&self.socket))
                .await
                .map_err(|_| "rc_timeout")?
                .map_err(|_| "rc_unavailable")?;
        let peer = stream.peer_cred().map_err(|_| "rc_peer_failed")?;
        let expected_pid = self.expected_pid.load(Ordering::Relaxed);
        if expected_pid == 0
            || peer.uid() != unsafe { libc::geteuid() }
            || peer.pid() != Some(expected_pid as i32)
        {
            return Err("rc_peer_failed");
        }
        let credentials = Zeroizing::new(format!(
            "{RC_USER}:{}",
            std::str::from_utf8(self.password.expose()).map_err(|_| "rc_authentication_failed")?
        ));
        let encoded = Zeroizing::new(base64(credentials.as_bytes()));
        let request = Zeroizing::new(format!(
            "POST /{endpoint} HTTP/1.1\r\nHost: localhost\r\nAuthorization: Basic {}\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}",
            encoded.as_str()
        ));
        stream
            .write_all(request.as_bytes())
            .await
            .map_err(|_| "rc_io_failed")?;
        stream.shutdown().await.map_err(|_| "rc_io_failed")?;
        let bytes = read_bounded(&mut stream).await?;
        Ok(Zeroizing::new(parse_response_body(&bytes)?.to_vec()))
    }
}

impl Drop for RcRuntime {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.socket);
        let _ = fs::remove_dir(&self.directory);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct VfsStats {
    #[serde(rename = "diskCache")]
    pub disk_cache: Option<DiskCacheStats>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DiskCacheStats {
    #[serde(rename = "uploadsQueued")]
    pub uploads_queued: u64,
    #[serde(rename = "uploadsInProgress")]
    pub uploads_in_progress: u64,
    #[serde(rename = "erroredFiles")]
    pub errored_files: u64,
    #[serde(rename = "outOfSpace")]
    pub out_of_space: bool,
}

pub fn safe_to_unmount(stats: &VfsStats) -> Result<(), &'static str> {
    let cache = stats.disk_cache.as_ref().ok_or("upload_state_unknown")?;
    if cache.out_of_space {
        return Err("cache_out_of_space");
    }
    if cache.errored_files != 0 {
        return Err("upload_errors");
    }
    if cache.uploads_queued != 0 || cache.uploads_in_progress != 0 {
        return Err("uploads_pending");
    }
    Ok(())
}

pub async fn start(
    config: &EncryptedConfig,
    password: &SessionSecret,
    paths: &Paths,
) -> Result<OwnedMount, &'static str> {
    let rc = RcRuntime::new().await?;
    let channel = PasswordChannel::new()?;
    let mut command = auth::base_command(config);
    configure_mount_arguments(&mut command, paths)?;
    rc.configure(&mut command);
    channel.configure(&mut command)?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|_| "mount_start_failed")?;
    let pid = child.id().ok_or("mount_start_failed")?;
    rc.set_process(pid);
    let ready = async {
        tokio::try_join!(
            channel.deliver(pid, password),
            wait_ready(&mut child, &rc, paths),
        )?;
        Ok(())
    };
    let result = tokio::time::timeout(Duration::from_secs(30), ready)
        .await
        .unwrap_or(Err("mount_timeout"));
    if let Err(error) = result {
        let _ = child.kill().await;
        let _ = child.wait().await;
        return Err(error);
    }
    Ok(OwnedMount { child, rc })
}

fn configure_mount_arguments(command: &mut Command, paths: &Paths) -> Result<(), &'static str> {
    command.arg("mount").arg(REMOTE).arg(&paths.mount);
    auth::transport_arguments(command, paths)?;
    command
        .args(["--vfs-cache-mode", "full", "--cache-dir"])
        .arg(&paths.cache)
        .args([
            "--vfs-cache-max-size",
            "10G",
            "--vfs-cache-max-age",
            "24h",
            "--vfs-write-back",
            "5s",
            "--timeout",
            "5m",
            "--dir-cache-time",
            "5m",
            "--poll-interval",
            "0",
            "--umask",
            "077",
            "--log-level",
            "NOTICE",
        ]);
    Ok(())
}

async fn wait_ready(child: &mut Child, rc: &RcRuntime, paths: &Paths) -> Result<(), &'static str> {
    let start = Instant::now();
    loop {
        if child
            .try_wait()
            .map_err(|_| "mount_process_wait_failed")?
            .is_some()
        {
            return Err("mount_process_exited");
        }
        if let Ok(metadata) = fs::symlink_metadata(&rc.socket) {
            if !metadata.file_type().is_socket() {
                return Err("rc_socket_invalid");
            }
            fs::set_permissions(&rc.socket, fs::Permissions::from_mode(0o600))
                .map_err(|_| "rc_socket_permissions")?;
        }
        let mountinfo =
            fs::read_to_string("/proc/self/mountinfo").map_err(|_| "mount_state_unknown")?;
        if mount_observation(&mountinfo, &paths.mount) == "mounted" {
            match rc.stats().await {
                Ok(_) => return Ok(()),
                Err("rc_unavailable" | "rc_timeout") => (),
                Err(error) => return Err(error),
            }
        }
        if start.elapsed() > Duration::from_secs(25) {
            return Err("mount_timeout");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub async fn unmount_path(paths: &Paths) -> Result<(), &'static str> {
    let mut command = Command::new("/usr/bin/fusermount3");
    command
        .env_clear()
        .arg("-u")
        .arg(&paths.mount)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let status = tokio::time::timeout(Duration::from_secs(15), command.status())
        .await
        .map_err(|_| "unmount_timeout")?
        .map_err(|_| "unmount_helper_failed")?;
    if !status.success() {
        return Err("unmount_busy_or_failed");
    }
    let start = Instant::now();
    loop {
        let mountinfo =
            fs::read_to_string("/proc/self/mountinfo").map_err(|_| "mount_state_unknown")?;
        match mount_observation(&mountinfo, &paths.mount) {
            "unmounted" => return Ok(()),
            "mounted" if start.elapsed() <= Duration::from_secs(10) => {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            "mounted" => return Err("unmount_not_confirmed"),
            _ => return Err("mount_state_unknown"),
        }
    }
}

async fn read_bounded(stream: &mut UnixStream) -> Result<SecureBytes, &'static str> {
    let mut bytes = Zeroizing::new(Vec::new());
    stream
        .take((RC_OUTPUT_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| "rc_io_failed")?;
    if bytes.len() > RC_OUTPUT_LIMIT {
        return Err("rc_output_limit");
    }
    Ok(bytes)
}

#[cfg(test)]
fn parse_stats_response(bytes: &[u8]) -> Result<VfsStats, &'static str> {
    serde_json::from_slice(parse_response_body(bytes)?).map_err(|_| "rc_response_invalid")
}

fn parse_response_body(bytes: &[u8]) -> Result<&[u8], &'static str> {
    let split = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or("rc_response_invalid")?;
    let headers = std::str::from_utf8(&bytes[..split]).map_err(|_| "rc_response_invalid")?;
    let status = headers.lines().next().ok_or("rc_response_invalid")?;
    if status != "HTTP/1.1 200 OK" && status != "HTTP/1.0 200 OK" {
        return Err(if status.contains(" 401 ") {
            "rc_authentication_failed"
        } else {
            "rc_response_failed"
        });
    }
    Ok(&bytes[split + 4..])
}

fn hex(bytes: &[u8]) -> String {
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(result, "{byte:02x}");
    }
    result
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let value = ((chunk[0] as u32) << 16)
            | ((chunk.get(1).copied().unwrap_or(0) as u32) << 8)
            | chunk.get(2).copied().unwrap_or(0) as u32;
        result.push(TABLE[((value >> 18) & 63) as usize] as char);
        result.push(TABLE[((value >> 12) & 63) as usize] as char);
        result.push(if chunk.len() > 1 {
            TABLE[((value >> 6) & 63) as usize] as char
        } else {
            '='
        });
        result.push(if chunk.len() > 2 {
            TABLE[(value & 63) as usize] as char
        } else {
            '='
        });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(body: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    #[test]
    fn parses_only_successful_bounded_vfs_stats() {
        let safe = r#"{"diskCache":{"uploadsQueued":0,"uploadsInProgress":0,"erroredFiles":0,"outOfSpace":false,"bytesUsed":3},"inUse":1}"#;
        assert!(safe_to_unmount(&parse_stats_response(&response(safe)).unwrap()).is_ok());
        for (field, code) in [
            ("\"uploadsQueued\":0", "uploads_pending"),
            ("\"uploadsInProgress\":0", "uploads_pending"),
            ("\"erroredFiles\":0", "upload_errors"),
            ("\"outOfSpace\":false", "cache_out_of_space"),
        ] {
            let value = if field.contains("outOfSpace") {
                field.replace("false", "true")
            } else {
                field.replace(":0", ":1")
            };
            let stats = parse_stats_response(&response(&safe.replace(field, &value))).unwrap();
            assert_eq!(safe_to_unmount(&stats), Err(code));
        }
        assert_eq!(
            safe_to_unmount(&VfsStats { disk_cache: None }),
            Err("upload_state_unknown")
        );
        assert_eq!(
            parse_stats_response(b"HTTP/1.1 401 Unauthorized\r\n\r\n{}").unwrap_err(),
            "rc_authentication_failed"
        );
        assert!(parse_stats_response(b"not http").is_err());
    }

    #[test]
    fn basic_auth_encoding_matches_standard_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(
            base64(b"hetzner-drive:fixture"),
            "aGV0em5lci1kcml2ZTpmaXh0dXJl"
        );
    }

    #[test]
    fn mount_command_is_fixed_foreground_and_uses_only_crypt_remote() {
        let mut command = Command::new("/usr/local/bin/rclone");
        let paths = Paths {
            key_file: crate::user_home().join(".ssh/fixture-storagebox"),
            sftp_host: "u12345.your-storagebox.de".into(),
            sftp_user: "u12345".into(),
            ..Paths::default()
        };
        configure_mount_arguments(&mut command, &paths).unwrap();
        let args: Vec<_> = command
            .as_std()
            .get_args()
            .map(|value| value.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[0], "mount");
        assert_eq!(args[1], REMOTE);
        assert_eq!(args[2], paths.mount.to_string_lossy());
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--vfs-cache-mode", "full"])
        );
        assert!(args.windows(2).any(|pair| pair == ["--umask", "077"]));
        assert!(!args.iter().any(|arg| arg == "--daemon"));
        assert!(!args.iter().any(|arg| arg == "hetzner-raw:"));
        for forbidden in ["sync", "delete", "purge", "move", "copy"] {
            assert!(!args.iter().any(|arg| arg == forbidden));
        }
    }

    #[tokio::test]
    async fn rc_runtime_keeps_plain_password_out_of_process_arguments() {
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let runtime = RcRuntime::create_in(directory.path().join("runtime")).await;
        assert!(
            runtime.is_err(),
            "create_in requires its private directory to exist"
        );
        let private = directory.path().join("private");
        fs::DirBuilder::new().mode(0o700).create(&private).unwrap();
        let runtime = RcRuntime::create_in(private).await.unwrap();
        let mut command = Command::new("/usr/local/bin/rclone");
        runtime.configure(&mut command);
        let arguments = format!("{:?}", command.as_std().get_args().collect::<Vec<_>>());
        let password = std::str::from_utf8(runtime.password.expose()).unwrap();
        assert!(!arguments.contains(password));
        assert!(arguments.contains("--rc-htpasswd"));
        assert!(!arguments.contains("--rc-pass"));
        assert!(!arguments.contains("--rc-no-auth"));
        let mut contents = String::new();
        use std::io::Read;
        let mut file = runtime.htpasswd.try_clone().unwrap();
        file.read_to_string(&mut contents).unwrap();
        assert!(contents.starts_with("hetzner-drive:$2"));
        assert!(!contents.contains(password));
    }

    #[tokio::test]
    async fn real_rclone_accepts_ephemeral_htpasswd_over_unix_socket() {
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let private = directory.path().join("private");
        fs::DirBuilder::new().mode(0o700).create(&private).unwrap();
        let runtime = RcRuntime::create_in(private).await.unwrap();
        let mut command = Command::new("/usr/local/bin/rclone");
        command
            .args(["rcd", "--config", "/dev/null"])
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        runtime.configure_options(&mut command);
        let mut child = command.spawn().unwrap();
        runtime.set_process(child.id().unwrap());
        let result = async {
            for _ in 0..100 {
                if runtime.socket.exists() {
                    match runtime.request("rc/noopauth").await {
                        Ok(body) => {
                            let value: serde_json::Value =
                                serde_json::from_slice(&body).map_err(|_| "rc_response_invalid")?;
                            assert!(value.is_object());
                            runtime.set_process(std::process::id());
                            assert_eq!(
                                runtime.request("rc/noopauth").await.unwrap_err(),
                                "rc_peer_failed"
                            );
                            runtime.set_process(child.id().unwrap());
                            assert!(runtime.request("rc/noopauth").await?.len() >= 2);
                            return Ok::<(), &'static str>(());
                        }
                        Err("rc_unavailable" | "rc_timeout") => (),
                        Err(error) => return Err(error),
                    }
                }
                if child.try_wait().unwrap().is_some() {
                    return Err("real_rclone_rc_exited");
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err("real_rclone_rc_timeout")
        }
        .await;
        let _ = child.kill().await;
        let _ = child.wait().await;
        result.unwrap();
    }
}
