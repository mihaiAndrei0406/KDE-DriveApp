use crate::{
    open_owned_file,
    secrets::{self, SecureBytes, SessionSecret, inherit_fd, secure_read},
    user_home, validate_path,
};
use configparser::ini::Ini;
use serde::Deserialize;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::AsyncWriteExt,
    net::{UnixListener, UnixStream},
    process::{Child, Command},
};

pub struct EncryptedConfig {
    file: File,
}

impl EncryptedConfig {
    pub fn capture(path: &Path) -> Result<Arc<Self>, &'static str> {
        let file = open_owned_file(path).map_err(|_| "config_unavailable")?;
        if file.metadata().map_err(|_| "config_unavailable")?.mode() & 0o777 != 0o600 {
            return Err("config_permissions");
        }
        let mut ciphertext = Vec::new();
        file.take(1024 * 1024 + 1)
            .read_to_end(&mut ciphertext)
            .map_err(|_| "config_unavailable")?;
        if ciphertext.len() > 1024 * 1024 {
            return Err("config_too_large");
        }
        let text = std::str::from_utf8(&ciphertext).map_err(|_| "config_format")?;
        let marker = text
            .lines()
            .find(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'));
        if marker != Some("RCLONE_ENCRYPT_V0:") {
            return Err("config_not_encrypted");
        }
        let fd = unsafe {
            libc::memfd_create(
                c"hetzner-config".as_ptr(),
                libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
            )
        };
        if fd < 0 {
            return Err("config_snapshot_failed");
        }
        let mut file = unsafe { File::from_raw_fd(fd) };
        file.write_all(&ciphertext)
            .map_err(|_| "config_snapshot_failed")?;
        file.seek(SeekFrom::Start(0))
            .map_err(|_| "config_snapshot_failed")?;
        if unsafe {
            libc::fcntl(
                fd,
                libc::F_ADD_SEALS,
                libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL,
            )
        } < 0
        {
            return Err("config_snapshot_failed");
        }
        Ok(Arc::new(Self { file }))
    }

    pub(crate) fn runtime_copy(&self) -> Result<RuntimeConfig, &'static str> {
        let directory =
            secrets::trusted_runtime()?.join(format!("hetzner-config-{}", secrets::random_name()?));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .map_err(|_| "config_snapshot_failed")?;
        let path = directory.join("rclone.conf");
        let result = (|| {
            let mut source = self
                .file
                .try_clone()
                .map_err(|_| "config_snapshot_failed")?;
            source
                .seek(SeekFrom::Start(0))
                .map_err(|_| "config_snapshot_failed")?;
            let mut ciphertext = Vec::new();
            source
                .take(1024 * 1024 + 1)
                .read_to_end(&mut ciphertext)
                .map_err(|_| "config_snapshot_failed")?;
            if ciphertext.len() > 1024 * 1024 {
                return Err("config_snapshot_failed");
            }
            let mut target = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
                .map_err(|_| "config_snapshot_failed")?;
            target
                .write_all(&ciphertext)
                .map_err(|_| "config_snapshot_failed")?;
            target.sync_all().map_err(|_| "config_snapshot_failed")?;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o400))
                .map_err(|_| "config_snapshot_failed")?;
            Ok(())
        })();
        match result {
            Ok(()) => Ok(RuntimeConfig { directory, path }),
            Err(error) => {
                let _ = fs::remove_file(&path);
                let _ = fs::remove_dir(&directory);
                Err(error)
            }
        }
    }
}

pub(crate) struct RuntimeConfig {
    directory: PathBuf,
    path: PathBuf,
}

impl RuntimeConfig {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for RuntimeConfig {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_dir(&self.directory);
    }
}

pub(crate) struct PasswordChannel {
    directory: PathBuf,
    listener: UnixListener,
}

impl PasswordChannel {
    pub(crate) fn new() -> Result<Self, &'static str> {
        Self::in_runtime(&secrets::trusted_runtime()?)
    }

    fn in_runtime(runtime: &Path) -> Result<Self, &'static str> {
        let directory = runtime.join(format!("hetzner-pass-{}", secrets::random_name()?));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .map_err(|_| "password_channel_failed")?;
        let socket = directory.join("password.sock");
        let listener = match UnixListener::bind(&socket) {
            Ok(listener) => listener,
            Err(_) => {
                let _ = fs::remove_dir(&directory);
                return Err("password_channel_failed");
            }
        };
        let channel = Self {
            directory,
            listener,
        };
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))
            .map_err(|_| "password_channel_failed")?;
        Ok(channel)
    }

    pub(crate) fn configure(&self, command: &mut Command) -> Result<(), &'static str> {
        command
            .arg("--password-command")
            .arg(Self::helper_command("--password-helper")?);
        self.configure_environment(command, "HETZNER_PASSWORD_SOCKET");
        Ok(())
    }

    pub(crate) fn helper_command(argument: &str) -> Result<String, &'static str> {
        let executable = std::env::current_exe().map_err(|_| "helper_unavailable")?;
        let executable = executable.to_str().ok_or("helper_path")?;
        let quoted = executable.replace('\\', "\\\\").replace('"', "\\\"");
        Ok(format!("\"{quoted}\" {argument}"))
    }

    pub(crate) fn configure_environment(&self, command: &mut Command, variable: &str) {
        command
            .env(variable, self.directory.join("password.sock"))
            .env("HETZNER_CONTROLLER_PID", std::process::id().to_string());
    }

    pub(crate) async fn deliver(
        &self,
        child_pid: u32,
        secret: &SessionSecret,
    ) -> Result<(), &'static str> {
        loop {
            let (mut stream, _) = self
                .listener
                .accept()
                .await
                .map_err(|_| "password_channel_failed")?;
            let peer = stream.peer_cred().map_err(|_| "password_peer_failed")?;
            let Some(pid) = peer.pid() else {
                continue;
            };
            if peer.uid() != unsafe { libc::geteuid() } || parent_pid(pid as u32) != Some(child_pid)
            {
                continue;
            }
            stream
                .write_all(secret.expose())
                .await
                .map_err(|_| "password_channel_failed")?;
            stream
                .shutdown()
                .await
                .map_err(|_| "password_channel_failed")?;
            return Ok(());
        }
    }

    pub(crate) async fn deliver_to_restic_child(
        &self,
        restic_pid: u32,
        restic_executable: &Path,
        secret: &SessionSecret,
    ) -> Result<(), &'static str> {
        self.deliver_matching(secret, |helper_pid| {
            parent_pid(helper_pid) == Some(restic_pid)
                && process_executable(restic_pid).as_deref() == Some(restic_executable)
        })
        .await
    }

    pub(crate) async fn deliver_to_rclone_grandchild(
        &self,
        restic_pid: u32,
        restic_executable: &Path,
        rclone_executable: &Path,
        secret: &SessionSecret,
    ) -> Result<(), &'static str> {
        self.deliver_matching(secret, |helper_pid| {
            let Some(rclone_pid) = parent_pid(helper_pid) else {
                return false;
            };
            parent_pid(rclone_pid) == Some(restic_pid)
                && process_executable(restic_pid).as_deref() == Some(restic_executable)
                && process_executable(rclone_pid).as_deref() == Some(rclone_executable)
        })
        .await
    }

    async fn deliver_matching(
        &self,
        secret: &SessionSecret,
        permitted: impl Fn(u32) -> bool,
    ) -> Result<(), &'static str> {
        loop {
            let (mut stream, _) = self
                .listener
                .accept()
                .await
                .map_err(|_| "password_channel_failed")?;
            let peer = stream.peer_cred().map_err(|_| "password_peer_failed")?;
            let Some(pid) = peer.pid() else {
                continue;
            };
            if peer.uid() != unsafe { libc::geteuid() } || !permitted(pid as u32) {
                continue;
            }
            stream
                .write_all(secret.expose())
                .await
                .map_err(|_| "password_channel_failed")?;
            stream
                .shutdown()
                .await
                .map_err(|_| "password_channel_failed")?;
            return Ok(());
        }
    }
}

impl Drop for PasswordChannel {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.directory.join("password.sock"));
        let _ = fs::remove_dir(&self.directory);
    }
}

fn parent_pid(pid: u32) -> Option<u32> {
    fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()?
        .lines()
        .find_map(|l| l.strip_prefix("PPid:").and_then(|s| s.trim().parse().ok()))
}

fn process_executable(pid: u32) -> Option<PathBuf> {
    fs::read_link(format!("/proc/{pid}/exe")).ok()
}

pub async fn password_helper() -> Result<(), &'static str> {
    password_helper_from("HETZNER_PASSWORD_SOCKET").await
}

pub async fn restic_password_helper() -> Result<(), &'static str> {
    password_helper_from("HETZNER_RESTIC_PASSWORD_SOCKET").await
}

async fn password_helper_from(socket_variable: &str) -> Result<(), &'static str> {
    secrets::disable_dumps()?;
    let path = std::env::var_os(socket_variable).ok_or("helper_context_missing")?;
    let controller: i32 = std::env::var("HETZNER_CONTROLLER_PID")
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or("helper_context_missing")?;
    let work = async {
        let stream = UnixStream::connect(path)
            .await
            .map_err(|_| "password_channel_failed")?;
        let peer = stream.peer_cred().map_err(|_| "password_peer_failed")?;
        if peer.uid() != unsafe { libc::geteuid() } || peer.pid() != Some(controller) {
            return Err("password_peer_failed");
        }
        let bytes = secure_read(stream, 4095).await?;
        if bytes.is_empty() {
            return Err("invalid_password");
        }
        // stdout is the private pipe created by rclone's password-command mechanism.
        let mut out = std::io::stdout().lock();
        out.write_all(&bytes)
            .map_err(|_| "password_channel_failed")?;
        out.write_all(b"\n")
            .map_err(|_| "password_channel_failed")?;
        out.flush().map_err(|_| "password_channel_failed")
    };
    tokio::time::timeout(Duration::from_secs(10), work)
        .await
        .unwrap_or(Err("password_channel_timeout"))
}

#[derive(Clone, Copy, Debug)]
pub enum AuthOperation {
    ValidateConfig,
    StorageUsage,
    CheckConnection,
}

impl AuthOperation {
    pub fn arguments(self) -> &'static [&'static str] {
        match self {
            Self::ValidateConfig => &["config", "redacted"],
            Self::StorageUsage => &["about", "hetzner-crypt:", "--json"],
            Self::CheckConnection => &["lsjson", "hetzner-crypt:", "--stat"],
        }
    }
}

pub(crate) fn base_command(config: &EncryptedConfig) -> Command {
    let mut command = Command::new("/usr/local/bin/rclone");
    command
        .env_clear()
        .env("LC_ALL", "C")
        .env("HOME", user_home())
        .env("PATH", "/usr/local/bin:/usr/bin:/bin");
    if let Some(socket) = std::env::var_os("SSH_AUTH_SOCK") {
        command.env("SSH_AUTH_SOCK", socket);
    }
    inherit_fd(&mut command, config.file.as_raw_fd());
    command
        .arg("--config")
        .arg(format!("/proc/self/fd/{}", config.file.as_raw_fd()))
        .args([
            "--ask-password=false",
            "--log-level",
            "ERROR",
            "--retries",
            "1",
            "--low-level-retries",
            "1",
            "--contimeout",
            "10s",
            "--timeout",
            "20s",
        ]);
    command
}

#[derive(Clone)]
pub(crate) struct TransportPolicy {
    host: String,
    user: String,
    key_file: PathBuf,
}

impl TransportPolicy {
    pub(crate) fn from_paths(paths: &crate::Paths) -> Result<Self, &'static str> {
        if paths.sftp_host.is_empty() || paths.sftp_user.is_empty() {
            return Err("transport_policy_missing");
        }
        let host_account = paths
            .sftp_host
            .strip_suffix(".your-storagebox.de")
            .ok_or("transport_policy_invalid")?;
        let main_user = paths
            .sftp_user
            .split_once('-')
            .map_or(paths.sftp_user.as_str(), |(main, _)| main);
        let safe_account = host_account.starts_with('u')
            && host_account[1..]
                .chars()
                .all(|character| character.is_ascii_digit());
        let safe_user = !paths.sftp_user.is_empty()
            && paths.sftp_user.chars().all(|character| {
                character.is_ascii_alphanumeric() || character == '-' || character == '_'
            });
        let key_text = paths.key_file.to_str().ok_or("transport_policy_invalid")?;
        let safe_key = validate_path(&paths.key_file)
            && paths.key_file.parent() == Some(user_home().join(".ssh").as_path())
            && key_text.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-')
            });
        if host_account != main_user || !safe_account || !safe_user || !safe_key {
            return Err("transport_policy_invalid");
        }
        Ok(Self {
            host: paths.sftp_host.clone(),
            user: paths.sftp_user.clone(),
            key_file: paths.key_file.clone(),
        })
    }

    pub(crate) fn rclone_fragment(&self) -> String {
        format!(
            "--sftp-host {} --sftp-user {} --sftp-port 23 --sftp-key-use-agent=true \
             --sftp-key-file {} --sftp-pin-host-key=false --sftp-ask-password=false \
             --sftp-disable-hashcheck=true",
            self.host,
            self.user,
            self.key_file.display()
        )
    }
}

pub(crate) fn transport_arguments(
    command: &mut Command,
    paths: &crate::Paths,
) -> Result<(), &'static str> {
    // Public account values come from an explicit, validated local policy. Fixed
    // overrides ensure the sealed rclone snapshot cannot redirect this process.
    let policy = TransportPolicy::from_paths(paths)?;
    command.args([
        "--sftp-host",
        &policy.host,
        "--sftp-user",
        &policy.user,
        "--sftp-port",
        "23",
        "--sftp-key-use-agent=true",
        "--sftp-key-file",
        &policy.key_file.to_string_lossy(),
        "--sftp-pin-host-key=false",
        "--sftp-ask-password=false",
        "--sftp-disable-hashcheck=true",
    ]);
    Ok(())
}

pub(crate) async fn collect(
    child: &mut Child,
    limit: usize,
) -> Result<(i32, SecureBytes), &'static str> {
    let stdout = child.stdout.take().ok_or("process_io")?;
    let stderr = child.stderr.take().ok_or("process_io")?;
    let (status, stdout, _stderr) = tokio::try_join!(
        async { child.wait().await.map_err(|_| "process_wait_failed") },
        secure_read(stdout, limit),
        secure_read(stderr, 65536)
    )?;
    Ok((status.code().unwrap_or(-1), stdout))
}

async fn collect_authenticated(
    child: &mut Child,
    channel: &PasswordChannel,
    secret: &SessionSecret,
) -> Result<SecureBytes, &'static str> {
    let pid = child.id().ok_or("rclone_start_failed")?;
    let output = collect(child, 1024 * 1024);
    tokio::pin!(output);
    // A process can fail before starting its password helper. Do not wait for
    // a connection that will never arrive and disguise the failure as a timeout.
    let (code, bytes) = tokio::select! {
        biased;
        delivered = channel.deliver(pid, secret) => {
            delivered?;
            output.await?
        },
        result = &mut output => {
            let (code, _) = result?;
            return Err(if code == 0 { "password_not_requested" } else { "rclone_operation_failed" });
        }
    };
    if code != 0 {
        return Err("rclone_operation_failed");
    }
    Ok(bytes)
}

pub async fn run(
    operation: AuthOperation,
    config: &EncryptedConfig,
    secret: &SessionSecret,
    paths: &crate::Paths,
) -> Result<SecureBytes, &'static str> {
    let channel = PasswordChannel::new()?;
    let mut command = base_command(config);
    command.args(operation.arguments());
    if !matches!(operation, AuthOperation::ValidateConfig) {
        transport_arguments(&mut command, paths)?;
    }
    channel.configure(&mut command)?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|_| "rclone_start_failed")?;
    let operation = collect_authenticated(&mut child, &channel, secret);
    let result = tokio::time::timeout(Duration::from_secs(30), operation)
        .await
        .unwrap_or(Err("rclone_timeout"));
    if result.is_err() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    result
}

pub fn validate_redacted(bytes: &[u8], paths: &crate::Paths) -> Result<(), &'static str> {
    let text = std::str::from_utf8(bytes).map_err(|_| "config_format")?;
    // Rclone remote names are case-sensitive. Folding them can validate a
    // different section from the one used by the actual network command.
    let mut ini = Ini::new_cs();
    // The input is rclone's serialized values, not user-authored INI comments.
    // Do not silently truncate a remote or option value at '#' or ';'.
    ini.set_inline_comment_symbols(Some(&[]));
    ini.read(text.to_owned()).map_err(|_| "config_format")?;
    for section in ["hetzner-crypt", "hetzner-raw"] {
        if ini.get_map_ref().get(section).is_some_and(|values| {
            values
                .keys()
                .any(|key| key.starts_with("override.") || key.starts_with("global."))
        }) {
            return Err("config_override_forbidden");
        }
    }
    let policy = TransportPolicy::from_paths(paths)?;
    let expected_key = policy.key_file.to_str().ok_or("transport_policy_invalid")?;
    let expected = [
        ("hetzner-crypt", "type", "crypt"),
        ("hetzner-crypt", "remote", "hetzner-raw:encrypted-data"),
        ("hetzner-raw", "type", "sftp"),
        ("hetzner-raw", "port", "23"),
        ("hetzner-raw", "key_use_agent", "true"),
        ("hetzner-raw", "key_file", expected_key),
        ("hetzner-raw", "shell_type", "unix"),
    ];
    for (section, key, value) in expected {
        if ini.get(section, key).as_deref() != Some(value) {
            return Err("config_policy_mismatch");
        }
    }
    // rclone deliberately masks these values in `config redacted`; require that
    // they were redacted, then anchor network commands to the external policy.
    if ini.get("hetzner-raw", "host").as_deref() != Some("XXX")
        || ini.get("hetzner-raw", "user").as_deref() != Some("XXX")
    {
        return Err("config_policy_mismatch");
    }
    for (key, default) in [
        ("filename_encryption", "standard"),
        ("directory_name_encryption", "true"),
        ("no_data_encryption", "false"),
        ("pass_bad_blocks", "false"),
        ("show_mapping", "false"),
    ] {
        if ini
            .get("hetzner-crypt", key)
            .unwrap_or_else(|| default.into())
            != default
        {
            return Err("crypt_policy_mismatch");
        }
    }
    if ini
        .get("hetzner-raw", "host_keys")
        .is_none_or(|s| s.trim().is_empty())
    {
        return Err("host_pinning_missing");
    }
    for key in [
        "known_hosts_file",
        "ssh",
        "key_pem",
        "pass",
        "key_file_pass",
        "socks_proxy",
        "http_proxy",
        "pubkey",
        "pubkey_file",
    ] {
        if ini
            .get("hetzner-raw", key)
            .is_some_and(|s| !s.trim().is_empty())
        {
            return Err("sftp_policy_mismatch");
        }
    }
    for key in ["pin_host_key", "ask_password", "use_insecure_cipher"] {
        if ini
            .get("hetzner-raw", key)
            .is_some_and(|s| s != "false" && !s.is_empty())
        {
            return Err("sftp_policy_mismatch");
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize)]
pub struct StorageUsage {
    pub total: u64,
    pub used: u64,
    pub free: u64,
}

pub fn parse_storage(bytes: &[u8]) -> Result<StorageUsage, &'static str> {
    let usage: StorageUsage = serde_json::from_slice(bytes).map_err(|_| "storage_unavailable")?;
    if usage.used > usage.total || usage.free > usage.total {
        return Err("storage_invalid");
    }
    Ok(usage)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> (String, crate::Paths) {
        let key_file = user_home().join(".ssh/fixture-storagebox");
        let text = format!(
            "[hetzner-crypt]\ntype = crypt\nremote = hetzner-raw:encrypted-data\npassword = XXX\npassword2 = XXX\n[hetzner-raw]\ntype = sftp\nhost = XXX\nuser = XXX\nport = 23\nkey_use_agent = true\nkey_file = {}\nshell_type = unix\nhost_keys = ssh-ed25519 fixture-public-key\n",
            key_file.display()
        );
        let paths = crate::Paths {
            key_file,
            sftp_host: "u12345.your-storagebox.de".into(),
            sftp_user: "u12345".into(),
            ..crate::Paths::default()
        };
        (text, paths)
    }

    #[test]
    fn accepts_expected_chain_and_rejects_dangerous_overrides() {
        let (config, paths) = config();
        assert!(validate_redacted(config.as_bytes(), &paths).is_ok());
        for (before, after) in [
            ("type = crypt", "type = alias"),
            ("hetzner-raw:encrypted-data", "/tmp/plaintext"),
            ("key_use_agent = true", "key_use_agent = false"),
            ("host_keys = ssh-ed25519 fixture-public-key", "host_keys = "),
        ] {
            assert!(validate_redacted(config.replace(before, after).as_bytes(), &paths).is_err());
        }
        for option in [
            "pin_host_key = true",
            "known_hosts_file = none",
            "ssh = arbitrary",
            "http_proxy = external",
        ] {
            assert!(validate_redacted(format!("{config}{option}\n").as_bytes(), &paths).is_err());
        }
    }
    #[test]
    fn preserves_remote_case_and_refuses_global_option_overrides() {
        let (config, paths) = config();
        let wrong_case = config.replace("[hetzner-crypt]", "[HETZNER-CRYPT]");
        assert!(validate_redacted(wrong_case.as_bytes(), &paths).is_err());
        let shadow = format!("[hetzner-crypt]\ntype = local\n{wrong_case}");
        assert!(validate_redacted(shadow.as_bytes(), &paths).is_err());
        for suffix in ["#other-directory", ";other-directory"] {
            let changed = config.replace(
                "hetzner-raw:encrypted-data",
                &format!("hetzner-raw:encrypted-data{suffix}"),
            );
            assert!(validate_redacted(changed.as_bytes(), &paths).is_err());
        }
        for section in ["hetzner-crypt", "hetzner-raw"] {
            for option in [
                "global.dump = bodies",
                "override.http_proxy = localhost:9999",
            ] {
                let changed =
                    config.replace(&format!("[{section}]"), &format!("[{section}]\n{option}"));
                assert_eq!(
                    validate_redacted(changed.as_bytes(), &paths),
                    Err("config_override_forbidden")
                );
            }
        }
    }

    #[test]
    fn validates_storagebox_identity_without_embedding_an_account() {
        let (config, paths) = config();
        let subaccount_paths = crate::Paths {
            sftp_user: "u12345-backups".into(),
            ..paths.clone()
        };
        assert!(validate_redacted(config.as_bytes(), &subaccount_paths).is_ok());
        for changed in [
            crate::Paths {
                sftp_host: "example.invalid".into(),
                ..paths.clone()
            },
            crate::Paths {
                sftp_user: "u54321".into(),
                ..paths.clone()
            },
            crate::Paths {
                sftp_user: "u12345;proxy".into(),
                ..paths.clone()
            },
            crate::Paths {
                key_file: user_home().join(".ssh/space key"),
                ..paths.clone()
            },
            crate::Paths {
                key_file: user_home().join("outside-key"),
                ..paths.clone()
            },
        ] {
            assert_eq!(
                validate_redacted(config.as_bytes(), &changed),
                Err("transport_policy_invalid")
            );
        }
    }

    #[test]
    fn storage_requires_real_nonnegative_fields() {
        assert_eq!(
            parse_storage(br#"{"total":100,"used":20,"free":80}"#)
                .unwrap()
                .used,
            20
        );
        for text in [
            r#"{"used":0}"#,
            r#"{"total":10,"used":20,"free":0}"#,
            r#"{"total":10,"used":-1,"free":10}"#,
        ] {
            assert!(parse_storage(text.as_bytes()).is_err());
        }
    }
    #[test]
    fn authenticated_commands_are_fixed_and_non_destructive() {
        for operation in [
            AuthOperation::ValidateConfig,
            AuthOperation::StorageUsage,
            AuthOperation::CheckConnection,
        ] {
            assert!(
                !operation
                    .arguments()
                    .iter()
                    .any(|s| ["sync", "purge", "delete", "move", "copy"].contains(s))
            );
        }
    }

    #[test]
    fn transport_is_fixed_and_requires_the_preexisting_host_pin() {
        let (config, paths) = config();
        let mut command = Command::new("/usr/local/bin/rclone");
        transport_arguments(&mut command, &paths).unwrap();
        let arguments: Vec<_> = command
            .as_std()
            .get_args()
            .map(|value| value.to_string_lossy().into_owned())
            .collect();
        for pair in [
            ["--sftp-host", "u12345.your-storagebox.de"],
            ["--sftp-user", "u12345"],
            ["--sftp-port", "23"],
            ["--sftp-key-use-agent=true", "--sftp-key-file"],
            ["--sftp-pin-host-key=false", "--sftp-ask-password=false"],
        ] {
            assert!(arguments.windows(2).any(|window| window == pair));
        }
        assert!(!arguments.iter().any(|value| {
            value.contains("known-hosts-file")
                || value.contains("host-keys")
                || value.contains("use-insecure-cipher")
                || value.contains("proxy")
        }));
        assert!(validate_redacted(config.as_bytes(), &paths).is_ok());
        assert_eq!(
            validate_redacted(
                config
                    .replace("host_keys = ssh-ed25519 fixture-public-key", "host_keys = ")
                    .as_bytes(),
                &paths,
            ),
            Err("host_pinning_missing")
        );
    }

    #[tokio::test]
    async fn broker_refuses_a_peer_that_is_not_an_rclone_child() {
        let directory = tempfile::tempdir().unwrap();
        let channel = PasswordChannel::in_runtime(directory.path()).unwrap();
        let secret = SessionSecret::new(b"synthetic peer fixture").unwrap();
        let path = channel.directory.join("password.sock");
        let delivery = tokio::time::timeout(
            Duration::from_millis(100),
            channel.deliver(std::process::id(), &secret),
        );
        let unauthorized = async {
            let connection = UnixStream::connect(path).await.unwrap();
            let bytes = secure_read(connection, 4095).await.unwrap();
            assert!(bytes.is_empty());
        };
        let (result, ()) = tokio::join!(delivery, unauthorized);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn early_exit_without_password_helper_finishes_promptly() {
        let directory = tempfile::tempdir().unwrap();
        let channel = PasswordChannel::in_runtime(directory.path()).unwrap();
        let secret = SessionSecret::new(b"synthetic early-exit fixture").unwrap();
        for (program, expected) in [
            ("/usr/bin/false", "rclone_operation_failed"),
            ("/usr/bin/true", "password_not_requested"),
        ] {
            let mut child = Command::new(program)
                .env_clear()
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .unwrap();
            let result = tokio::time::timeout(
                Duration::from_secs(2),
                collect_authenticated(&mut child, &channel, &secret),
            )
            .await
            .expect("Exited process kept waiting for a password helper");
            assert_eq!(result.err(), Some(expected));
            assert!(child.try_wait().unwrap().is_some());
        }
    }
}
