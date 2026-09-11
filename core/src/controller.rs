use crate::{
    Paths, ProcessRunner, Snapshot, State,
    auth::{self, AuthOperation, EncryptedConfig, StorageUsage},
    backup::{BackupManager, BackupStatus, SnapshotEntry, SnapshotTreeEntry},
    mount::{self, OwnedMount},
    recent_logs,
    secrets::{self, SessionSecret},
    snapshot,
};
use std::{
    collections::VecDeque,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{process::Command, sync::Mutex};

#[derive(Clone)]
struct Credentials {
    config: Arc<EncryptedConfig>,
    password: Arc<SessionSecret>,
}

#[derive(Clone, Default)]
pub struct OperationStatus {
    pub busy: bool,
    pub last_operation: String,
    pub last_error: String,
    pub owned_mount: bool,
}

#[derive(Clone, Default)]
pub struct MountActivity {
    pub available: bool,
    pub uploads_queued: u64,
    pub uploads_in_progress: u64,
    pub errored_files: u64,
    pub out_of_space: bool,
    pub reason: String,
}

pub struct Controller {
    demo: bool,
    demo_unlocked: AtomicBool,
    demo_owned_mount: AtomicBool,
    credentials: Mutex<Option<Credentials>>,
    owned_mount: Mutex<Option<OwnedMount>>,
    state_override: Mutex<Option<State>>,
    mount_failure: AtomicBool,
    mount_attention: AtomicBool,
    operation: Mutex<()>,
    details: Mutex<OperationStatus>,
    cached: Mutex<Option<(Instant, Snapshot)>>,
    events: Mutex<VecDeque<String>>,
    remote_failure: Mutex<Option<&'static str>>,
    backup: Arc<BackupManager>,
}

fn mount_error_requires_attention(error: &str) -> bool {
    matches!(
        error,
        "upload_errors"
            | "cache_out_of_space"
            | "upload_state_unknown"
            | "rc_unavailable"
            | "rc_authentication_failed"
            | "rc_peer_failed"
            | "rc_timeout"
            | "rc_io_failed"
            | "rc_response_invalid"
            | "rc_response_failed"
            | "rc_output_limit"
    )
}

impl Controller {
    pub fn new(demo: bool) -> Self {
        Self {
            demo,
            demo_unlocked: AtomicBool::new(false),
            demo_owned_mount: AtomicBool::new(false),
            credentials: Mutex::new(None),
            owned_mount: Mutex::new(None),
            state_override: Mutex::new(None),
            mount_failure: AtomicBool::new(false),
            mount_attention: AtomicBool::new(false),
            operation: Mutex::new(()),
            details: Mutex::new(OperationStatus::default()),
            cached: Mutex::new(None),
            events: Mutex::new(VecDeque::new()),
            remote_failure: Mutex::new(None),
            backup: BackupManager::new(demo),
        }
    }

    pub async fn observe(&self) -> Snapshot {
        self.reconcile_owned_mount().await;
        let mut cached = self.cached.lock().await;
        let mut value = if let Some((at, value)) = &*cached
            && at.elapsed() < Duration::from_secs(3)
        {
            value.clone()
        } else {
            let value = if self.demo {
                Snapshot::demo()
            } else {
                let mounts = std::fs::read_to_string("/proc/self/mountinfo")
                    .unwrap_or_else(|_| "unavailable".into());
                snapshot(&ProcessRunner, &Paths::default(), &mounts).await
            };
            *cached = Some((Instant::now(), value.clone()));
            value
        };
        if self.is_unlocked().await {
            value.config = "unlocked".into();
            if value.mount == "unmounted" && value.ssh == "loaded" && value.state != State::Error {
                value.state = State::Ready;
            }
        }
        if self.demo && self.demo_owned_mount.load(Ordering::Relaxed) {
            value.mount = "mounted".into();
            value.state = State::Mounted;
        }
        if self.remote_failure.lock().await.is_some() {
            if value.state != State::Error {
                value.state = State::Degraded;
            }
            if !value.diagnostic.is_empty() {
                value.diagnostic.push(',');
            }
            value.diagnostic.push_str("remote_check_failed");
        }
        if self.mount_failure.load(Ordering::Relaxed) {
            if value.state != State::Error {
                value.state = State::Degraded;
            }
            if !value.diagnostic.is_empty() {
                value.diagnostic.push(',');
            }
            value.diagnostic.push_str("mount_process_exited");
        }
        if self.mount_attention.load(Ordering::Relaxed) {
            if value.state != State::Error {
                value.state = State::Degraded;
            }
            if !value.diagnostic.is_empty() {
                value.diagnostic.push(',');
            }
            value
                .diagnostic
                .push_str("mount_activity_requires_attention");
        }
        if let Some(state) = *self.state_override.lock().await {
            value.state = state;
        }
        value
    }

    async fn reconcile_owned_mount(&self) {
        if self.demo {
            return;
        }
        let exited = {
            let mut owned = self.owned_mount.lock().await;
            match owned.as_mut().map(OwnedMount::try_wait) {
                Some(Ok(Some(_)) | Err(_)) => {
                    owned.take();
                    true
                }
                Some(Ok(None)) | None => false,
            }
        };
        if exited {
            self.mount_failure.store(true, Ordering::Relaxed);
            let mut details = self.details.lock().await;
            details.owned_mount = false;
            details.last_error = "mount_process_exited".into();
        }
    }

    async fn observe_fresh(&self) -> Snapshot {
        self.cached.lock().await.take();
        self.observe().await
    }

    async fn record_remote_result<T>(&self, result: &Result<T, &'static str>) {
        *self.remote_failure.lock().await = result.as_ref().err().copied();
    }

    pub async fn is_unlocked(&self) -> bool {
        self.demo_unlocked.load(Ordering::Relaxed) || self.credentials.lock().await.is_some()
    }

    pub async fn operation_status(&self) -> OperationStatus {
        self.details.lock().await.clone()
    }

    pub async fn mount_activity(&self) -> MountActivity {
        if self.demo {
            self.mount_attention.store(false, Ordering::Relaxed);
            return if self.demo_owned_mount.load(Ordering::Relaxed) {
                MountActivity {
                    available: true,
                    reason: "ok".into(),
                    ..MountActivity::default()
                }
            } else {
                MountActivity {
                    reason: "drive_not_mounted".into(),
                    ..MountActivity::default()
                }
            };
        }
        self.reconcile_owned_mount().await;
        let owned = self.owned_mount.lock().await;
        let Some(process) = owned.as_ref() else {
            self.mount_attention.store(false, Ordering::Relaxed);
            let mountinfo = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
            let reason =
                if crate::mount_observation(&mountinfo, &Paths::default().mount) == "mounted" {
                    "mount_not_owned"
                } else {
                    "drive_not_mounted"
                };
            return MountActivity {
                reason: reason.into(),
                ..MountActivity::default()
            };
        };
        let activity = match process.stats().await {
            Ok(stats) => match stats.disk_cache {
                Some(cache) => MountActivity {
                    available: true,
                    uploads_queued: cache.uploads_queued,
                    uploads_in_progress: cache.uploads_in_progress,
                    errored_files: cache.errored_files,
                    out_of_space: cache.out_of_space,
                    reason: "ok".into(),
                },
                None => MountActivity {
                    reason: "upload_state_unknown".into(),
                    ..MountActivity::default()
                },
            },
            Err(reason) => MountActivity {
                reason: reason.into(),
                ..MountActivity::default()
            },
        };
        self.mount_attention.store(
            !activity.available || activity.errored_files != 0 || activity.out_of_space,
            Ordering::Relaxed,
        );
        activity
    }

    async fn started(&self, operation: &str) {
        let mut status = self.details.lock().await;
        status.busy = true;
        status.last_operation = operation.into();
    }

    async fn finished(&self, operation: &str, result: Result<(), &'static str>) -> (bool, String) {
        self.finished_with_code(operation, result.map(|()| "ok"))
            .await
    }

    async fn finished_with_code(
        &self,
        operation: &str,
        result: Result<&'static str, &'static str>,
    ) -> (bool, String) {
        let success = result.is_ok();
        let code = result.unwrap_or_else(|code| code);
        {
            let mut status = self.details.lock().await;
            status.busy = false;
            status.last_operation = operation.into();
            if !success {
                status.last_error = code.into();
            }
        }
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let level = if success { "INFO" } else { "WARNING" };
        let event = serde_json::json!({"timestamp": timestamp, "level": level, "operation": operation, "result": code}).to_string();
        let mut events = self.events.lock().await;
        if events.len() == 100 {
            events.pop_front();
        }
        events.push_back(event.clone());
        eprintln!("{event}");
        (success, code.into())
    }

    pub async fn unlock(&self) -> (bool, String) {
        let Ok(_guard) = self.operation.try_lock() else {
            return (false, "operation_busy".into());
        };
        self.started("unlock").await;
        let result = async {
            if self.is_unlocked().await {
                return Ok(());
            }
            if self.demo {
                self.demo_unlocked.store(true, Ordering::Relaxed);
                self.backup.demo_configuration_unlocked(true).await;
                return Ok(());
            }
            let paths = Paths::default();
            let config = EncryptedConfig::capture(&paths.config)?;
            secrets::disable_dumps()?;
            let password = secrets::prompt_password().await?;
            let output = auth::run(AuthOperation::ValidateConfig, &config, &password, &paths)
                .await
                .map_err(|e| {
                    if e == "rclone_operation_failed" {
                        "unlock_failed"
                    } else {
                        e
                    }
                })?;
            auth::validate_redacted(&output, &paths)?;
            *self.credentials.lock().await = Some(Credentials { config, password });
            self.backup.demo_configuration_unlocked(true).await;
            Ok(())
        }
        .await;
        self.finished("unlock", result).await
    }

    pub async fn lock(&self) -> (bool, String) {
        let Ok(_guard) = self.operation.try_lock() else {
            return (false, "operation_busy".into());
        };
        self.started("lock_configuration").await;
        self.credentials.lock().await.take();
        self.backup.lock().await;
        self.demo_unlocked.store(false, Ordering::Relaxed);
        self.remote_failure.lock().await.take();
        self.finished("lock_configuration", Ok(())).await
    }

    pub async fn mount_drive(&self) -> (bool, String) {
        let Ok(_guard) = self.operation.try_lock() else {
            return (false, "operation_busy".into());
        };
        self.started("mount_drive").await;
        *self.state_override.lock().await = Some(State::Mounting);
        let result = self.mount_inner().await;
        *self.state_override.lock().await = None;
        self.cached.lock().await.take();
        self.finished("mount_drive", result).await
    }

    async fn mount_inner(&self) -> Result<(), &'static str> {
        if !self.is_unlocked().await {
            return Err("configuration_locked");
        }
        if self.demo {
            if self.demo_owned_mount.swap(true, Ordering::Relaxed) {
                return Err("drive_already_mounted");
            }
            self.details.lock().await.owned_mount = true;
            self.mount_failure.store(false, Ordering::Relaxed);
            self.mount_attention.store(false, Ordering::Relaxed);
            return Ok(());
        }
        self.reconcile_owned_mount().await;
        if self.owned_mount.lock().await.is_some() {
            return Err("drive_already_mounted");
        }
        let paths = Paths::default();
        let mountinfo =
            std::fs::read_to_string("/proc/self/mountinfo").map_err(|_| "mount_state_unknown")?;
        match crate::mount_observation(&mountinfo, &paths.mount) {
            "mounted" => return Err("mount_not_owned"),
            "unmounted" => (),
            "conflict" => return Err("mount_path_conflict"),
            _ => return Err("mount_state_unknown"),
        }
        if !crate::remote_mount_targets(&mountinfo)?.is_empty() {
            return Err("remote_already_mounted_elsewhere");
        }
        if !crate::validate_private_directory(&paths.mount) {
            return Err("mount_directory_unsafe");
        }
        if !crate::validate_private_directory(&paths.cache) {
            return Err("cache_directory_unsafe");
        }
        if std::fs::read_dir(&paths.mount)
            .map_err(|_| "mount_directory_unavailable")?
            .next()
            .is_some()
        {
            return Err("mount_directory_not_empty");
        }
        if crate::check_agent(&ProcessRunner, &paths).await != "loaded" {
            return Err("ssh_key_unavailable");
        }
        let credentials = self
            .credentials
            .lock()
            .await
            .clone()
            .ok_or("configuration_locked")?;
        let process = mount::start(&credentials.config, &credentials.password, &paths).await?;
        if process.process_id().is_none() {
            return Err("mount_process_exited");
        }
        *self.owned_mount.lock().await = Some(process);
        self.details.lock().await.owned_mount = true;
        self.mount_failure.store(false, Ordering::Relaxed);
        self.mount_attention.store(false, Ordering::Relaxed);
        Ok(())
    }

    pub async fn unmount_drive(&self) -> (bool, String) {
        let Ok(_guard) = self.operation.try_lock() else {
            return (false, "operation_busy".into());
        };
        self.started("unmount_drive").await;
        *self.state_override.lock().await = Some(State::Unmounting);
        let result = self.unmount_inner().await;
        *self.state_override.lock().await = None;
        self.cached.lock().await.take();
        self.finished("unmount_drive", result).await
    }

    pub async fn shutdown(&self) -> (bool, String) {
        let _guard = self.operation.lock().await;
        self.started("shutdown").await;
        *self.state_override.lock().await = Some(State::Unmounting);
        let result = self.shutdown_inner().await;
        *self.state_override.lock().await = None;
        self.cached.lock().await.take();
        self.finished("shutdown", result).await
    }

    async fn shutdown_inner(&self) -> Result<(), &'static str> {
        let backup_wait_started = Instant::now();
        while self.backup.status().await.busy {
            if backup_wait_started.elapsed() >= Duration::from_secs(270) {
                return Err("backup_in_progress");
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        if self.demo {
            self.demo_owned_mount.store(false, Ordering::Relaxed);
            self.details.lock().await.owned_mount = false;
            self.mount_attention.store(false, Ordering::Relaxed);
            return Ok(());
        }
        self.reconcile_owned_mount().await;
        let Some(mut process) = self.owned_mount.lock().await.take() else {
            return Ok(());
        };
        let start = Instant::now();
        loop {
            match self.unmount_owned(&mut process).await {
                Ok(()) => {
                    self.details.lock().await.owned_mount = false;
                    self.mount_failure.store(false, Ordering::Relaxed);
                    self.mount_attention.store(false, Ordering::Relaxed);
                    return Ok(());
                }
                Err("uploads_pending") if start.elapsed() < Duration::from_secs(270) => {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                Err(error) => {
                    self.mount_attention
                        .store(mount_error_requires_attention(error), Ordering::Relaxed);
                    if process.try_wait()?.is_none() {
                        *self.owned_mount.lock().await = Some(process);
                        self.details.lock().await.owned_mount = true;
                    } else {
                        self.details.lock().await.owned_mount = false;
                        self.mount_failure.store(true, Ordering::Relaxed);
                    }
                    return Err(error);
                }
            }
        }
    }

    async fn unmount_inner(&self) -> Result<(), &'static str> {
        if self.demo {
            if !self.demo_owned_mount.swap(false, Ordering::Relaxed) {
                return Err("mount_not_owned");
            }
            self.details.lock().await.owned_mount = false;
            self.mount_attention.store(false, Ordering::Relaxed);
            return Ok(());
        }
        self.reconcile_owned_mount().await;
        let mut process = self
            .owned_mount
            .lock()
            .await
            .take()
            .ok_or("mount_not_owned")?;
        let result = self.unmount_owned(&mut process).await;
        if result.is_ok() {
            self.details.lock().await.owned_mount = false;
            self.mount_failure.store(false, Ordering::Relaxed);
            self.mount_attention.store(false, Ordering::Relaxed);
        } else {
            self.mount_attention.store(
                result
                    .as_ref()
                    .is_err_and(|error| mount_error_requires_attention(error)),
                Ordering::Relaxed,
            );
            if process.try_wait()?.is_none() {
                *self.owned_mount.lock().await = Some(process);
                self.details.lock().await.owned_mount = true;
            } else {
                self.details.lock().await.owned_mount = false;
                self.mount_failure.store(true, Ordering::Relaxed);
            }
        }
        result
    }

    async fn unmount_owned(&self, process: &mut OwnedMount) -> Result<(), &'static str> {
        let paths = Paths::default();
        let mountinfo =
            std::fs::read_to_string("/proc/self/mountinfo").map_err(|_| "mount_state_unknown")?;
        if crate::mount_observation(&mountinfo, &paths.mount) != "mounted" {
            return Err("owned_mount_not_observed");
        }
        mount::safe_to_unmount(&process.stats().await?)?;
        tokio::time::sleep(Duration::from_millis(750)).await;
        mount::safe_to_unmount(&process.stats().await?)?;
        mount::unmount_path(&paths).await?;
        if process.wait(Duration::from_secs(5)).await.is_err() {
            process.terminate().await;
        }
        Ok(())
    }

    pub async fn storage_usage(&self) -> Result<StorageUsage, String> {
        let Ok(_guard) = self.operation.try_lock() else {
            return Err("operation_busy".into());
        };
        self.started("storage_usage").await;
        let result = async {
            if !self.is_unlocked().await {
                return Err("configuration_locked");
            }
            if self.demo {
                self.record_remote_result(&Ok(())).await;
                return Ok(StorageUsage {
                    total: 1_000_000_000_000,
                    used: 25_000_000_000,
                    free: 975_000_000_000,
                });
            }
            let credentials = self
                .credentials
                .lock()
                .await
                .clone()
                .ok_or("configuration_locked")?;
            if crate::check_agent(&ProcessRunner, &Paths::default()).await != "loaded" {
                return Err("ssh_key_unavailable");
            }
            let result = async {
                let bytes = auth::run(
                    AuthOperation::StorageUsage,
                    &credentials.config,
                    &credentials.password,
                    &Paths::default(),
                )
                .await?;
                auth::parse_storage(&bytes)
            }
            .await;
            self.record_remote_result(&result).await;
            result
        }
        .await;
        self.finished("storage_usage", result.as_ref().map(|_| ()).map_err(|e| *e))
            .await;
        result.map_err(str::to_owned)
    }

    pub async fn check_connection(&self) -> (bool, String) {
        let Ok(_guard) = self.operation.try_lock() else {
            return (false, "operation_busy".into());
        };
        self.started("check_connection").await;
        let result = self.check_connection_inner().await;
        self.finished("check_connection", result).await
    }

    // The caller holds the operation guard, including when invoked by health_check.
    async fn check_connection_inner(&self) -> Result<(), &'static str> {
        if !self.is_unlocked().await {
            return Err("configuration_locked");
        }
        if self.demo {
            self.record_remote_result(&Ok(())).await;
            return Ok(());
        }
        let credentials = self
            .credentials
            .lock()
            .await
            .clone()
            .ok_or("configuration_locked")?;
        if crate::check_agent(&ProcessRunner, &Paths::default()).await != "loaded" {
            return Err("ssh_key_unavailable");
        }
        let result = async {
            let bytes = auth::run(
                AuthOperation::CheckConnection,
                &credentials.config,
                &credentials.password,
                &Paths::default(),
            )
            .await?;
            #[derive(serde::Deserialize)]
            struct Entry {
                #[serde(rename = "IsDir")]
                is_dir: bool,
            }
            let entry: Entry =
                serde_json::from_slice(&bytes).map_err(|_| "connection_response_invalid")?;
            if !entry.is_dir {
                return Err("connection_response_invalid");
            }
            Ok(())
        }
        .await;
        self.record_remote_result(&result).await;
        result
    }

    pub async fn health_check(&self) -> (bool, String) {
        let Ok(_guard) = self.operation.try_lock() else {
            return (false, "operation_busy".into());
        };
        self.started("health_check").await;
        let result = async {
            let observation = self.observe_fresh().await;
            if observation.state == State::Error {
                return Err("local_health_failed");
            }
            if observation.ssh != "loaded" {
                return Err("ssh_key_unavailable");
            }
            if self.is_unlocked().await {
                self.check_connection_inner().await?;
                Ok("ok")
            } else {
                Ok("local_ok_configuration_locked")
            }
        }
        .await;
        self.finished_with_code("health_check", result).await
    }

    pub async fn check_ssh(&self) -> String {
        let Ok(_guard) = self.operation.try_lock() else {
            return "operation_busy".into();
        };
        self.started("check_ssh").await;
        let observation = self.observe_fresh().await;
        let result = if observation.ssh == "loaded" {
            Ok(())
        } else {
            Err("ssh_key_unavailable")
        };
        self.finished("check_ssh", result).await;
        observation.ssh
    }

    pub async fn open_drive(&self) -> (bool, String) {
        let Ok(_guard) = self.operation.try_lock() else {
            return (false, "operation_busy".into());
        };
        self.started("open_drive").await;
        let result = self.open_drive_inner().await;
        self.finished_with_code("open_drive", result).await
    }

    async fn open_drive_inner(&self) -> Result<&'static str, &'static str> {
        let observation = self.observe().await;
        if observation.mount != "mounted" {
            return Err("drive_not_mounted");
        }
        if self.demo {
            return Ok("demo_open_drive");
        }
        let mounts = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
        if crate::mount_observation(&mounts, &Paths::default().mount) != "mounted" {
            return Err("drive_not_mounted");
        }
        let mut command = Command::new("/usr/bin/dolphin");
        command
            .env_clear()
            .arg("--new-window")
            .arg(&Paths::default().mount)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        secrets::desktop_environment(&mut command);
        match command.spawn() {
            Ok(mut child) => {
                tokio::spawn(async move {
                    let _ = child.wait().await;
                });
                Ok("ok")
            }
            Err(_) => Err("dolphin_unavailable"),
        }
    }

    pub async fn recent_logs(&self) -> Vec<String> {
        let mut logs = if self.demo {
            vec!["2026/09/09 12:20:30 INFO Upload completed".into()]
        } else {
            recent_logs(&Paths::default().log)
        };
        logs.extend(self.events.lock().await.iter().cloned());
        let keep = logs.len().saturating_sub(100);
        logs.drain(..keep);
        logs
    }

    pub async fn backup_status(&self) -> BackupStatus {
        self.backup.status().await
    }

    async fn backup_credentials(&self) -> Result<Credentials, &'static str> {
        if !self.demo && crate::check_agent(&ProcessRunner, &Paths::default()).await != "loaded" {
            return Err("ssh_key_unavailable");
        }
        self.credentials
            .lock()
            .await
            .clone()
            .ok_or("configuration_locked")
    }

    pub async fn initialize_backup_repository(&self) -> (bool, String) {
        if self.demo {
            self.backup.demo_configuration_unlocked(true).await;
            return (true, "ok".into());
        }
        let credentials = match self.backup_credentials().await {
            Ok(credentials) => credentials,
            Err(code) => return (false, code.into()),
        };
        self.backup
            .initialize(&credentials.config, &credentials.password)
            .await
    }

    pub async fn unlock_backup_repository(&self) -> (bool, String) {
        if self.demo {
            self.backup.demo_configuration_unlocked(true).await;
            return (true, "ok".into());
        }
        let credentials = match self.backup_credentials().await {
            Ok(credentials) => credentials,
            Err(code) => return (false, code.into()),
        };
        self.backup
            .unlock(&credentials.config, &credentials.password)
            .await
    }

    pub async fn lock_backup_repository(&self) -> (bool, String) {
        self.backup.lock().await;
        (true, "ok".into())
    }

    pub async fn start_project_backup(&self, project_id: &str) -> (bool, String) {
        if self.demo {
            return self.backup.start_demo(project_id, None).await;
        }
        let credentials = match self.backup_credentials().await {
            Ok(credentials) => credentials,
            Err(code) => return (false, code.into()),
        };
        self.backup
            .start_backup(project_id, credentials.config, credentials.password)
            .await
    }

    pub async fn project_snapshots(
        &self,
        project_id: &str,
    ) -> Result<(Vec<SnapshotEntry>, bool), String> {
        if self.demo {
            return self
                .backup
                .demo_snapshots(project_id)
                .await
                .map_err(str::to_owned);
        }
        let credentials = match self.backup_credentials().await {
            Ok(credentials) => credentials,
            Err(code) => return Err(code.into()),
        };
        self.backup
            .list_snapshots(project_id, &credentials.config, &credentials.password)
            .await
            .map_err(str::to_owned)
    }

    pub async fn snapshot_entries(
        &self,
        project_id: &str,
        snapshot_id: &str,
    ) -> Result<(Vec<SnapshotTreeEntry>, bool), String> {
        if self.demo {
            return self
                .backup
                .demo_snapshot_entries(project_id, snapshot_id)
                .await
                .map_err(str::to_owned);
        }
        let credentials = match self.backup_credentials().await {
            Ok(credentials) => credentials,
            Err(code) => return Err(code.into()),
        };
        self.backup
            .list_snapshot_entries(
                project_id,
                snapshot_id,
                &credentials.config,
                &credentials.password,
            )
            .await
            .map_err(str::to_owned)
    }

    pub async fn start_backup_restore(
        &self,
        project_id: &str,
        snapshot_id: &str,
    ) -> (bool, String) {
        if self.demo {
            return self.backup.start_demo(project_id, Some(snapshot_id)).await;
        }
        let credentials = match self.backup_credentials().await {
            Ok(credentials) => credentials,
            Err(code) => return (false, code.into()),
        };
        self.backup
            .start_restore(
                project_id,
                snapshot_id,
                credentials.config,
                credentials.password,
            )
            .await
    }

    pub async fn start_selective_restore(
        &self,
        project_id: &str,
        snapshot_id: &str,
        selected_path: &str,
    ) -> (bool, String) {
        if self.demo {
            return self
                .backup
                .start_demo_selective_restore(project_id, snapshot_id, selected_path)
                .await;
        }
        let credentials = match self.backup_credentials().await {
            Ok(credentials) => credentials,
            Err(code) => return (false, code.into()),
        };
        self.backup
            .start_selective_restore(
                project_id,
                snapshot_id,
                selected_path,
                credentials.config,
                credentials.password,
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn demo_unlock_and_lock_change_state_without_external_operations() {
        let controller = Controller::new(true);
        assert_eq!(controller.observe().await.state, State::Locked);
        assert!(controller.storage_usage().await.is_err());
        assert!(controller.unlock().await.0);
        assert_eq!(controller.observe().await.state, State::Ready);
        assert!(controller.mount_drive().await.0);
        assert_eq!(controller.observe().await.state, State::Mounted);
        assert!(controller.operation_status().await.owned_mount);
        assert!(!controller.mount_drive().await.0);
        assert!(controller.unmount_drive().await.0);
        assert_eq!(controller.observe().await.state, State::Ready);
        assert!(!controller.operation_status().await.owned_mount);
        assert!(!controller.unmount_drive().await.0);
        assert!(controller.check_connection().await.0);
        assert_eq!(
            controller.storage_usage().await.unwrap().total,
            1_000_000_000_000
        );
        assert!(controller.lock().await.0);
        assert_eq!(controller.observe().await.state, State::Locked);
    }

    #[tokio::test]
    async fn demo_shutdown_releases_an_owned_mount() {
        let controller = Controller::new(true);
        assert!(controller.unlock().await.0);
        assert!(controller.mount_drive().await.0);
        assert!(controller.shutdown().await.0);
        assert_eq!(controller.observe().await.mount, "unmounted");
        assert!(!controller.operation_status().await.owned_mount);
        assert_eq!(
            controller.operation_status().await.last_operation,
            "shutdown"
        );
    }
    #[tokio::test]
    async fn concurrent_operations_are_refused_and_events_are_bounded() {
        let controller = Controller::new(true);
        let guard = controller.operation.lock().await;
        assert_eq!(controller.unlock().await, (false, "operation_busy".into()));
        assert_eq!(
            controller.health_check().await,
            (false, "operation_busy".into())
        );
        assert_eq!(
            controller.open_drive().await,
            (false, "operation_busy".into())
        );
        assert_eq!(controller.check_ssh().await, "operation_busy");
        assert_eq!(
            controller.mount_drive().await,
            (false, "operation_busy".into())
        );
        assert_eq!(
            controller.unmount_drive().await,
            (false, "operation_busy".into())
        );
        drop(guard);
        for _ in 0..110 {
            controller.finished("fixture", Ok(())).await;
        }
        assert_eq!(controller.events.lock().await.len(), 100);
    }

    #[tokio::test]
    async fn failed_remote_check_survives_polling_and_recovers_on_success() {
        let controller = Controller::new(true);
        assert!(controller.unlock().await.0);
        controller
            .record_remote_result(&Err::<(), _>("rclone_timeout"))
            .await;
        for _ in 0..2 {
            let status = controller.observe().await;
            assert_eq!(status.state, State::Degraded);
            assert!(status.diagnostic.contains("remote_check_failed"));
            assert!(status.diagnostic.contains("demo_data"));
        }
        assert!(controller.health_check().await.0);
        assert_eq!(controller.observe().await.state, State::Ready);
        assert!(
            !controller
                .observe()
                .await
                .diagnostic
                .contains("remote_check_failed")
        );
        controller
            .record_remote_result(&Err::<(), _>("rclone_timeout"))
            .await;
        assert!(controller.storage_usage().await.is_ok());
        assert_eq!(controller.observe().await.state, State::Ready);
        controller
            .record_remote_result(&Err::<(), _>("rclone_operation_failed"))
            .await;
        assert!(controller.lock().await.0);
        assert_eq!(controller.observe().await.state, State::Locked);
    }

    #[tokio::test]
    async fn health_and_rejected_actions_are_recorded_with_their_own_names() {
        let controller = Controller::new(true);
        assert_eq!(
            controller.health_check().await,
            (true, "local_ok_configuration_locked".into())
        );
        assert_eq!(
            controller.operation_status().await.last_operation,
            "health_check"
        );
        assert!(controller.operation_status().await.last_error.is_empty());
        assert!(controller.storage_usage().await.is_err());
        assert_eq!(
            controller.operation_status().await.last_error,
            "configuration_locked"
        );
        assert_eq!(
            controller.operation_status().await.last_operation,
            "storage_usage"
        );
        assert!(!controller.open_drive().await.0);
        assert_eq!(
            controller.operation_status().await.last_error,
            "drive_not_mounted"
        );
        assert!(controller.unlock().await.0);
        assert!(controller.health_check().await.0);
        assert_eq!(
            controller.operation_status().await.last_operation,
            "health_check"
        );
        assert!(!controller.operation_status().await.busy);
        assert_eq!(controller.check_ssh().await, "loaded");
        assert_eq!(
            controller.operation_status().await.last_operation,
            "check_ssh"
        );
        assert!(
            controller
                .recent_logs()
                .await
                .iter()
                .any(|event| event.contains("local_ok_configuration_locked"))
        );
    }
}
