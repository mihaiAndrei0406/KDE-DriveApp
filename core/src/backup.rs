//! User-confirmed restic snapshots through a fixed, append-only rclone backend.
use crate::{
    auth::{EncryptedConfig, PasswordChannel, TransportPolicy},
    no_symlink_components, open_owned_file,
    secrets::{self, SessionSecret},
    user_home, validate_path,
};
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Read,
    os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader},
    process::{Child, Command},
    sync::Mutex,
};
use zeroize::{Zeroize, Zeroizing};

pub const REPOSITORY_REMOTE: &str = "hetzner-crypt:HetznerDrive-Backup-Disposable/restic-v1";
pub const REPOSITORY: &str = "rclone:hetzner-crypt:HetznerDrive-Backup-Disposable/restic-v1";
const MAX_REGISTRY_BYTES: u64 = 1024 * 1024;
const MAX_PROJECTS: usize = 32;
const MAX_OUTPUT_LINE: usize = 1024 * 1024;
const MAX_CAPTURE_OUTPUT: usize = 4 * 1024 * 1024;
const MAX_SNAPSHOT_HISTORY: usize = 256;
const MAX_SNAPSHOT_ENTRIES: usize = 2048;
const MAX_SNAPSHOT_PATH: usize = 4096;

#[derive(Clone)]
pub struct BackupPaths {
    pub restic: PathBuf,
    pub rclone: PathBuf,
    pub projects: PathBuf,
    pub mount: PathBuf,
    pub restore_root: PathBuf,
    pub cache: PathBuf,
    pub transport: crate::Paths,
    repository: String,
    helper: Option<PathBuf>,
}

impl Default for BackupPaths {
    fn default() -> Self {
        let home = user_home();
        let config_home = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        Self {
            restic: "/usr/bin/restic".into(),
            rclone: "/usr/local/bin/rclone".into(),
            projects: config_home.join("hetzner-drive/projects.json"),
            mount: home.join("HetznerDrive"),
            restore_root: home.join("HetznerDrive-Restores"),
            cache: home.join(".cache/hetzner-drive-restic"),
            transport: crate::Paths::default(),
            repository: REPOSITORY.into(),
            helper: None,
        }
    }
}

#[derive(Clone)]
pub struct BackupStatus {
    pub available: bool,
    pub repository_initialized: bool,
    pub repository_unlocked: bool,
    pub busy: bool,
    pub phase: String,
    pub project_id: String,
    pub bytes_done: u64,
    pub total_bytes: u64,
    pub files_done: u64,
    pub total_files: u64,
    pub last_snapshot: String,
    pub restore_path: String,
    pub last_error: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotEntry {
    pub id: String,
    pub created_at: String,
    pub files: u64,
    pub bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotTreeEntry {
    pub path: String,
    pub kind: String,
    pub size: u64,
}

impl Default for BackupStatus {
    fn default() -> Self {
        Self {
            available: false,
            repository_initialized: false,
            repository_unlocked: false,
            busy: false,
            phase: "unavailable".into(),
            project_id: String::new(),
            bytes_done: 0,
            total_bytes: 0,
            files_done: 0,
            total_files: 0,
            last_snapshot: String::new(),
            restore_path: String::new(),
            last_error: String::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Project {
    id: String,
    name: String,
    path: String,
    interval_minutes: u64,
    quiet_minutes: u64,
    mode: String,
    excludes: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Registry {
    version: u64,
    projects: Vec<Project>,
}

#[derive(Default)]
struct ParsedOutput {
    snapshot_id: Option<String>,
    summary_seen: bool,
    captured: Zeroizing<Vec<u8>>,
}

#[derive(Deserialize)]
struct ResticSnapshot {
    id: String,
    time: String,
    paths: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
    summary: Option<ResticSnapshotSummary>,
}

#[derive(Deserialize)]
struct ResticSnapshotSummary {
    #[serde(default)]
    total_files_processed: u64,
    #[serde(default)]
    total_bytes_processed: u64,
}

#[derive(Deserialize)]
struct ResticLsRecord {
    struct_type: String,
    id: Option<String>,
    paths: Option<Vec<String>>,
    #[serde(default)]
    tags: Vec<String>,
    path: Option<String>,
    #[serde(rename = "type")]
    node_type: Option<String>,
    size: Option<u64>,
}

#[derive(Deserialize)]
struct ProgressMessage {
    #[serde(default)]
    message_type: String,
    bytes_done: Option<u64>,
    bytes_restored: Option<u64>,
    total_bytes: Option<u64>,
    files_done: Option<u64>,
    files_restored: Option<u64>,
    total_files: Option<u64>,
    total_bytes_processed: Option<u64>,
    total_files_processed: Option<u64>,
    snapshot_id: Option<String>,
}

pub struct BackupManager {
    demo: bool,
    paths: BackupPaths,
    repository_secret: Mutex<Option<Arc<SessionSecret>>>,
    snapshot_catalog: Mutex<HashMap<String, HashSet<String>>>,
    snapshot_entry_catalog: Mutex<HashMap<(String, String), HashSet<String>>>,
    status: Mutex<BackupStatus>,
    busy: AtomicBool,
}

impl BackupManager {
    pub fn new(demo: bool) -> Arc<Self> {
        Self::with_paths(demo, BackupPaths::default())
    }

    pub(crate) fn with_paths(demo: bool, paths: BackupPaths) -> Arc<Self> {
        Arc::new(Self {
            demo,
            paths,
            repository_secret: Mutex::new(None),
            snapshot_catalog: Mutex::new(HashMap::new()),
            snapshot_entry_catalog: Mutex::new(HashMap::new()),
            status: Mutex::new(BackupStatus::default()),
            busy: AtomicBool::new(false),
        })
    }

    pub async fn status(&self) -> BackupStatus {
        let mut status = self.status.lock().await;
        status.available = self.demo || executable(&self.paths.restic).is_ok();
        if !status.available && !status.busy {
            status.phase = "unavailable".into();
        } else if !status.busy && status.phase == "unavailable" {
            status.phase = "locked".into();
        }
        status.clone()
    }

    pub async fn demo_configuration_unlocked(&self, unlocked: bool) {
        if !self.demo {
            return;
        }
        let mut status = self.status.lock().await;
        status.available = true;
        status.repository_initialized = unlocked;
        status.repository_unlocked = unlocked;
        status.phase = if unlocked { "ready" } else { "locked" }.into();
    }

    pub async fn lock(&self) {
        self.repository_secret.lock().await.take();
        self.snapshot_catalog.lock().await.clear();
        self.snapshot_entry_catalog.lock().await.clear();
        let mut status = self.status.lock().await;
        status.repository_unlocked = false;
        if !status.busy {
            status.phase = "locked".into();
        }
    }

    pub async fn start_demo(
        self: &Arc<Self>,
        project_id: &str,
        snapshot_id: Option<&str>,
    ) -> (bool, String) {
        if !self.demo || !valid_project_id(project_id) {
            return (false, "project_id_invalid".into());
        }
        let restore = snapshot_id.is_some();
        if let Some(snapshot) = snapshot_id {
            let allowed = valid_snapshot_id(snapshot)
                && self
                    .snapshot_catalog
                    .lock()
                    .await
                    .get(project_id)
                    .is_some_and(|entries| entries.contains(snapshot));
            if !allowed {
                return (false, "backup_snapshot_unavailable".into());
            }
        }
        let phase = if restore { "restoring" } else { "backup" };
        if let Err(code) = self.begin(phase, project_id).await {
            return (false, code.into());
        }
        if restore {
            self.status.lock().await.restore_path = "/tmp/hetzner-drive-demo-restore".into();
        }
        let manager = self.clone();
        let id = project_id.to_owned();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(80)).await;
            manager
                .complete_demo(
                    &id,
                    if restore {
                        "restore_complete"
                    } else {
                        "backup_complete"
                    },
                )
                .await;
        });
        (true, "accepted".into())
    }

    pub async fn demo_snapshots(
        &self,
        project_id: &str,
    ) -> Result<(Vec<SnapshotEntry>, bool), &'static str> {
        if !self.demo || !valid_project_id(project_id) {
            return Err("project_id_invalid");
        }
        let snapshot = "a".repeat(64);
        self.snapshot_catalog
            .lock()
            .await
            .entry(project_id.into())
            .or_default()
            .insert(snapshot.clone());
        Ok((
            vec![SnapshotEntry {
                id: snapshot,
                created_at: "2026-09-10T12:00:00Z".into(),
                files: 3,
                bytes: 1024,
            }],
            false,
        ))
    }

    pub async fn demo_snapshot_entries(
        &self,
        project_id: &str,
        snapshot_id: &str,
    ) -> Result<(Vec<SnapshotTreeEntry>, bool), &'static str> {
        if !self.demo || !valid_project_id(project_id) || !valid_snapshot_id(snapshot_id) {
            return Err("backup_snapshot_unavailable");
        }
        let allowed = self
            .snapshot_catalog
            .lock()
            .await
            .get(project_id)
            .is_some_and(|entries| entries.contains(snapshot_id));
        if !allowed {
            return Err("backup_snapshot_unavailable");
        }
        let entries = vec![
            SnapshotTreeEntry {
                path: "README.md".into(),
                kind: "file".into(),
                size: 384,
            },
            SnapshotTreeEntry {
                path: "src".into(),
                kind: "dir".into(),
                size: 0,
            },
            SnapshotTreeEntry {
                path: "src/main.rs".into(),
                kind: "file".into(),
                size: 640,
            },
        ];
        self.snapshot_entry_catalog.lock().await.insert(
            (project_id.into(), snapshot_id.into()),
            entries.iter().map(|entry| entry.path.clone()).collect(),
        );
        Ok((entries, false))
    }

    pub async fn start_demo_selective_restore(
        self: &Arc<Self>,
        project_id: &str,
        snapshot_id: &str,
        selected_path: &str,
    ) -> (bool, String) {
        if !valid_project_id(project_id)
            || !valid_snapshot_id(snapshot_id)
            || !valid_snapshot_relative_path(selected_path)
        {
            return (false, "backup_entry_unavailable".into());
        }
        let snapshot_allowed = self
            .snapshot_catalog
            .lock()
            .await
            .get(project_id)
            .is_some_and(|entries| entries.contains(snapshot_id));
        let entry_allowed = self
            .snapshot_entry_catalog
            .lock()
            .await
            .get(&(project_id.into(), snapshot_id.into()))
            .is_some_and(|entries| entries.contains(selected_path));
        if !snapshot_allowed || !entry_allowed {
            return (false, "backup_entry_unavailable".into());
        }
        if let Err(code) = self.begin("restoring", project_id).await {
            return (false, code.into());
        }
        let mut status = self.status.lock().await;
        status.last_snapshot = snapshot_id.into();
        status.restore_path = "/tmp/hetzner-drive-demo-restore".into();
        drop(status);
        let manager = self.clone();
        let id = project_id.to_owned();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(80)).await;
            manager.complete_demo(&id, "restore_complete").await;
        });
        (true, "accepted".into())
    }

    pub async fn list_snapshots(
        &self,
        project_id: &str,
        config: &EncryptedConfig,
        config_password: &SessionSecret,
    ) -> Result<(Vec<SnapshotEntry>, bool), &'static str> {
        let Some(repository_password) = self.repository_secret.lock().await.clone() else {
            return Err("backup_repository_locked");
        };
        if !valid_project_id(project_id) {
            return Err("project_id_invalid");
        }
        self.dependencies()?;
        let project = load_project(&self.paths, project_id, false)?;
        self.snapshot_catalog.lock().await.remove(project_id);
        self.snapshot_entry_catalog
            .lock()
            .await
            .retain(|(catalog_project, _), _| catalog_project != project_id);
        self.begin("history", project_id).await?;
        let result = self
            .run_restic_capture(
                vec![
                    "snapshots".into(),
                    "--json".into(),
                    "--tag".into(),
                    format!("hetzner-drive,project:{project_id}"),
                    "--path".into(),
                    project.path.clone(),
                    "--latest".into(),
                    (MAX_SNAPSHOT_HISTORY + 1).to_string(),
                ],
                config,
                config_password,
                &repository_password,
            )
            .await
            .map_err(|_| "backup_history_failed")
            .and_then(|bytes| parse_snapshot_history(&bytes, &project));
        if let Ok((entries, _)) = &result {
            self.snapshot_catalog.lock().await.insert(
                project_id.into(),
                entries.iter().map(|entry| entry.id.clone()).collect(),
            );
        }
        self.busy.store(false, Ordering::Release);
        let mut status = self.status.lock().await;
        status.busy = false;
        match &result {
            Ok(_) => {
                status.phase = "ready".into();
                status.last_error.clear();
            }
            Err(code) => {
                status.phase = "failed".into();
                status.last_error = (*code).into();
            }
        }
        result
    }

    pub async fn list_snapshot_entries(
        &self,
        project_id: &str,
        snapshot_id: &str,
        config: &EncryptedConfig,
        config_password: &SessionSecret,
    ) -> Result<(Vec<SnapshotTreeEntry>, bool), &'static str> {
        let Some(repository_password) = self.repository_secret.lock().await.clone() else {
            return Err("backup_repository_locked");
        };
        if !valid_project_id(project_id) || !valid_snapshot_id(snapshot_id) {
            return Err("backup_snapshot_unavailable");
        }
        let allowed = self
            .snapshot_catalog
            .lock()
            .await
            .get(project_id)
            .is_some_and(|entries| entries.contains(snapshot_id));
        if !allowed {
            return Err("backup_snapshot_unavailable");
        }
        self.snapshot_entry_catalog
            .lock()
            .await
            .remove(&(project_id.into(), snapshot_id.into()));
        self.dependencies()?;
        let project = load_project(&self.paths, project_id, false)?;
        self.begin("contents", project_id).await?;
        let result = self
            .run_restic_capture(
                vec![
                    "ls".into(),
                    "--json".into(),
                    "--recursive".into(),
                    "--sort".into(),
                    "name".into(),
                    snapshot_id.into(),
                    project.path.clone(),
                ],
                config,
                config_password,
                &repository_password,
            )
            .await
            .map_err(|_| "backup_contents_failed")
            .and_then(|bytes| parse_snapshot_entries(&bytes, &project, snapshot_id));
        if let Ok((entries, _)) = &result {
            self.snapshot_entry_catalog.lock().await.insert(
                (project_id.into(), snapshot_id.into()),
                entries.iter().map(|entry| entry.path.clone()).collect(),
            );
        }
        self.busy.store(false, Ordering::Release);
        let mut status = self.status.lock().await;
        status.busy = false;
        match &result {
            Ok(_) => {
                status.phase = "ready".into();
                status.last_error.clear();
            }
            Err(code) => {
                status.phase = "failed".into();
                status.last_error = (*code).into();
            }
        }
        result
    }

    pub async fn initialize(
        &self,
        config: &EncryptedConfig,
        config_password: &SessionSecret,
    ) -> (bool, String) {
        if self.demo {
            self.demo_configuration_unlocked(true).await;
            return (true, "ok".into());
        }
        if let Err(code) = self.begin("initializing", "").await {
            return (false, code.into());
        }
        let result = async {
            self.dependencies()?;
            let password = secrets::prompt_restic_password(false).await?;
            let confirmation = secrets::prompt_restic_password(true).await?;
            if !secrets::secrets_equal(&password, &confirmation) {
                return Err("backup_password_mismatch");
            }
            drop(confirmation);
            self.run_restic(
                vec!["init".into()],
                config,
                config_password,
                &password,
                false,
            )
            .await
            .map_err(|_| "backup_repository_init_failed")?;
            *self.repository_secret.lock().await = Some(password);
            Ok(())
        }
        .await;
        self.finish_setup(result).await
    }

    pub async fn unlock(
        &self,
        config: &EncryptedConfig,
        config_password: &SessionSecret,
    ) -> (bool, String) {
        if self.demo {
            self.demo_configuration_unlocked(true).await;
            return (true, "ok".into());
        }
        if let Err(code) = self.begin("unlocking", "").await {
            return (false, code.into());
        }
        let result = async {
            self.dependencies()?;
            let password = secrets::prompt_restic_password(false).await?;
            self.run_restic(
                vec![
                    "snapshots".into(),
                    "--json".into(),
                    "--latest".into(),
                    "1".into(),
                ],
                config,
                config_password,
                &password,
                false,
            )
            .await
            .map_err(|_| "backup_unlock_failed")?;
            *self.repository_secret.lock().await = Some(password);
            Ok(())
        }
        .await;
        self.finish_setup(result).await
    }

    async fn finish_setup(&self, result: Result<(), &'static str>) -> (bool, String) {
        self.busy.store(false, Ordering::Release);
        let mut status = self.status.lock().await;
        status.busy = false;
        match result {
            Ok(()) => {
                status.repository_initialized = true;
                status.repository_unlocked = true;
                status.phase = "ready".into();
                status.last_error.clear();
                (true, "ok".into())
            }
            Err(code) => {
                status.repository_unlocked = false;
                status.phase = "failed".into();
                status.last_error = code.into();
                (false, code.into())
            }
        }
    }

    pub async fn start_backup(
        self: &Arc<Self>,
        project_id: &str,
        config: Arc<EncryptedConfig>,
        config_password: Arc<SessionSecret>,
    ) -> (bool, String) {
        if self.demo {
            return self.start_demo(project_id, None).await;
        }
        let Some(repository_password) = self.repository_secret.lock().await.clone() else {
            return (false, "backup_repository_locked".into());
        };
        if let Err(code) = self.dependencies() {
            return (false, code.into());
        }
        let project = match load_project(&self.paths, project_id, true) {
            Ok(project) => project,
            Err(code) => return (false, code.into()),
        };
        if let Err(code) = self.begin("backup", project_id).await {
            return (false, code.into());
        }
        if let Err(code) = secrets::confirm_backup_action(false, false).await {
            self.finish_job(Err(code), "backup_complete").await;
            return (false, code.into());
        }
        let manager = self.clone();
        tokio::spawn(async move {
            let result = manager
                .backup_job(project, &config, &config_password, &repository_password)
                .await;
            manager.finish_job(result, "backup_complete").await;
        });
        (true, "accepted".into())
    }

    pub async fn start_restore(
        self: &Arc<Self>,
        project_id: &str,
        snapshot_id: &str,
        config: Arc<EncryptedConfig>,
        config_password: Arc<SessionSecret>,
    ) -> (bool, String) {
        self.start_restore_inner(project_id, snapshot_id, None, config, config_password)
            .await
    }

    pub async fn start_selective_restore(
        self: &Arc<Self>,
        project_id: &str,
        snapshot_id: &str,
        selected_path: &str,
        config: Arc<EncryptedConfig>,
        config_password: Arc<SessionSecret>,
    ) -> (bool, String) {
        self.start_restore_inner(
            project_id,
            snapshot_id,
            Some(selected_path),
            config,
            config_password,
        )
        .await
    }

    async fn start_restore_inner(
        self: &Arc<Self>,
        project_id: &str,
        snapshot_id: &str,
        selected_path: Option<&str>,
        config: Arc<EncryptedConfig>,
        config_password: Arc<SessionSecret>,
    ) -> (bool, String) {
        let Some(repository_password) = self.repository_secret.lock().await.clone() else {
            return (false, "backup_repository_locked".into());
        };
        if !valid_project_id(project_id) {
            return (false, "project_id_invalid".into());
        }
        if !valid_snapshot_id(snapshot_id) {
            return (false, "backup_snapshot_unavailable".into());
        }
        if selected_path.is_some_and(|path| !valid_snapshot_relative_path(path)) {
            return (false, "backup_entry_unavailable".into());
        }
        let project = match load_project(&self.paths, project_id, false) {
            Ok(project) => project,
            Err(code) => return (false, code.into()),
        };
        let allowed = self
            .snapshot_catalog
            .lock()
            .await
            .get(project_id)
            .is_some_and(|entries| entries.contains(snapshot_id));
        if !allowed {
            return (false, "backup_snapshot_unavailable".into());
        }
        if let Some(path) = selected_path {
            let allowed = self
                .snapshot_entry_catalog
                .lock()
                .await
                .get(&(project_id.into(), snapshot_id.into()))
                .is_some_and(|entries| entries.contains(path));
            if !allowed {
                return (false, "backup_entry_unavailable".into());
            }
        }
        let snapshot = snapshot_id.to_owned();
        if let Err(code) = self.begin("restoring", project_id).await {
            return (false, code.into());
        }
        self.status.lock().await.last_snapshot = snapshot.clone();
        if !self.demo
            && let Err(code) = secrets::confirm_backup_action(true, selected_path.is_some()).await
        {
            self.finish_job(Err(code), "restore_complete").await;
            return (false, code.into());
        }
        let target = match self.new_restore_target(project_id) {
            Ok(path) => path,
            Err(code) => {
                self.finish_job(Err(code), "restore_complete").await;
                return (false, code.into());
            }
        };
        self.status.lock().await.restore_path = target.to_string_lossy().into_owned();
        if self.demo {
            let manager = self.clone();
            let id = project_id.to_owned();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(80)).await;
                manager.complete_demo(&id, "restore_complete").await;
            });
            return (true, "accepted".into());
        }
        let manager = self.clone();
        let arguments = restore_arguments(&snapshot, &project.path, selected_path, &target);
        tokio::spawn(async move {
            let result = manager
                .run_restic(
                    arguments,
                    &config,
                    &config_password,
                    &repository_password,
                    true,
                )
                .await
                .and_then(|output| {
                    if output.summary_seen {
                        Ok(())
                    } else {
                        Err("restore_summary_invalid")
                    }
                });
            manager.finish_job(result, "restore_complete").await;
        });
        (true, "accepted".into())
    }

    async fn backup_job(
        &self,
        project: Project,
        config: &EncryptedConfig,
        config_password: &SessionSecret,
        repository_password: &SessionSecret,
    ) -> Result<(), &'static str> {
        let mut arguments = vec![
            "backup".into(),
            project.path,
            "--json".into(),
            "--one-file-system".into(),
            "--tag".into(),
            "hetzner-drive".into(),
            "--tag".into(),
            format!("project:{}", project.id),
        ];
        for pattern in project.excludes {
            arguments.push("--exclude".into());
            arguments.push(pattern);
        }
        let output = self
            .run_restic(
                arguments,
                config,
                config_password,
                repository_password,
                true,
            )
            .await
            .map_err(|_| "backup_failed")?;
        let snapshot = output.snapshot_id.ok_or("backup_summary_invalid")?;
        {
            let mut status = self.status.lock().await;
            status.phase = "checking".into();
            status.last_snapshot = snapshot.clone();
        }
        self.run_restic(
            vec!["check".into(), "--with-cache".into()],
            config,
            config_password,
            repository_password,
            false,
        )
        .await
        .map_err(|_| "backup_check_failed")?;
        self.snapshot_catalog
            .lock()
            .await
            .entry(project.id.clone())
            .or_default()
            .insert(snapshot);
        Ok(())
    }

    async fn complete_demo(&self, project_id: &str, phase: &str) {
        {
            let mut status = self.status.lock().await;
            status.project_id = project_id.into();
            status.last_snapshot = "a".repeat(64);
        }
        self.snapshot_catalog
            .lock()
            .await
            .entry(project_id.into())
            .or_default()
            .insert("a".repeat(64));
        self.finish_job(Ok(()), phase).await;
    }

    async fn finish_job(&self, result: Result<(), &'static str>, success_phase: &str) {
        self.busy.store(false, Ordering::Release);
        let mut status = self.status.lock().await;
        status.busy = false;
        match result {
            Ok(()) => {
                status.phase = success_phase.into();
                status.last_error.clear();
            }
            Err(code) => {
                status.phase = "failed".into();
                status.last_error = code.into();
            }
        }
    }

    async fn begin(&self, phase: &str, project_id: &str) -> Result<(), &'static str> {
        if self
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("backup_busy");
        }
        let mut status = self.status.lock().await;
        status.busy = true;
        status.phase = phase.into();
        status.project_id = project_id.into();
        status.bytes_done = 0;
        status.total_bytes = 0;
        status.files_done = 0;
        status.total_files = 0;
        status.last_error.clear();
        if phase != "restoring" {
            status.restore_path.clear();
        }
        Ok(())
    }

    fn dependencies(&self) -> Result<(), &'static str> {
        executable(&self.paths.restic).map(|_| ())?;
        executable(&self.paths.rclone).map(|_| ())?;
        Ok(())
    }

    fn new_restore_target(&self, project_id: &str) -> Result<PathBuf, &'static str> {
        ensure_private_directory(&self.paths.restore_root)?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "restore_target_failed")?
            .as_secs();
        let target = self.paths.restore_root.join(format!(
            "restore-{}-{timestamp}-{}",
            &project_id[..8],
            secrets::random_name()?
        ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&target)
            .map_err(|_| "restore_target_failed")?;
        Ok(target)
    }

    async fn run_restic(
        &self,
        arguments: Vec<String>,
        config: &EncryptedConfig,
        config_password: &SessionSecret,
        repository_password: &SessionSecret,
        progress: bool,
    ) -> Result<ParsedOutput, &'static str> {
        self.run_restic_inner(
            arguments,
            config,
            config_password,
            repository_password,
            progress,
            false,
        )
        .await
    }

    async fn run_restic_capture(
        &self,
        arguments: Vec<String>,
        config: &EncryptedConfig,
        config_password: &SessionSecret,
        repository_password: &SessionSecret,
    ) -> Result<Zeroizing<Vec<u8>>, &'static str> {
        Ok(self
            .run_restic_inner(
                arguments,
                config,
                config_password,
                repository_password,
                false,
                true,
            )
            .await?
            .captured)
    }

    async fn run_restic_inner(
        &self,
        arguments: Vec<String>,
        config: &EncryptedConfig,
        config_password: &SessionSecret,
        repository_password: &SessionSecret,
        progress: bool,
        capture: bool,
    ) -> Result<ParsedOutput, &'static str> {
        let restic = executable(&self.paths.restic)?;
        let rclone = executable(&self.paths.rclone)?;
        let core = self
            .paths
            .helper
            .clone()
            .map(Ok)
            .unwrap_or_else(std::env::current_exe)
            .map_err(|_| "helper_unavailable")?
            .canonicalize()
            .map_err(|_| "helper_unavailable")?;
        let core_text = safe_executable_text(&core)?;
        let restic_password_command = format!("\"{core_text}\" --restic-password-helper");
        let runtime_config = config.runtime_copy()?;
        let config_path = safe_executable_text(runtime_config.path())?;
        ensure_private_directory(&self.paths.cache)?;
        let transport = TransportPolicy::from_paths(&self.paths.transport)?;
        let rclone_args = format!(
            "serve restic --stdio --append-only --config {config_path} \
             --ask-password=false --log-level ERROR --retries 1 --low-level-retries 1 \
             --contimeout 10s --timeout 20s {}",
            transport.rclone_fragment()
        );
        let repository_channel = PasswordChannel::new()?;
        let rclone_channel = PasswordChannel::new()?;
        let mut command = Command::new(&restic);
        command
            .env_clear()
            .env("LC_ALL", "C")
            .env("HOME", user_home())
            .env("PATH", "/usr/local/bin:/usr/bin:/bin")
            .env(
                "RCLONE_PASSWORD_COMMAND",
                format!("{core_text} --password-helper"),
            );
        if let Some(socket) = std::env::var_os("SSH_AUTH_SOCK") {
            command.env("SSH_AUTH_SOCK", socket);
        }
        repository_channel.configure_environment(&mut command, "HETZNER_RESTIC_PASSWORD_SOCKET");
        rclone_channel.configure_environment(&mut command, "HETZNER_PASSWORD_SOCKET");
        command.args([
            "--repo",
            &self.paths.repository,
            "--cache-dir",
            &self.paths.cache.to_string_lossy(),
            "--no-lock",
            "--password-command",
            &restic_password_command,
            "--option",
            &format!("rclone.program={}", rclone.to_string_lossy()),
            "--option",
            &format!("rclone.args={rclone_args}"),
        ]);
        command
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|_| "restic_start_failed")?;
        let restic_pid = child.id().ok_or("restic_start_failed")?;
        let delivery = tokio::time::timeout(Duration::from_secs(20), async {
            tokio::try_join!(
                repository_channel.deliver_to_restic_child(
                    restic_pid,
                    &restic,
                    repository_password,
                ),
                rclone_channel.deliver_to_rclone_grandchild(
                    restic_pid,
                    &restic,
                    &rclone,
                    config_password,
                )
            )?;
            Ok::<(), &'static str>(())
        })
        .await
        .unwrap_or(Err("backup_password_timeout"));
        if let Err(error) = delivery {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(error);
        }
        let result = self.collect_restic(&mut child, progress, capture).await;
        if result.is_err() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        result
    }

    async fn collect_restic(
        &self,
        child: &mut Child,
        progress: bool,
        capture: bool,
    ) -> Result<ParsedOutput, &'static str> {
        let stdout = child.stdout.take().ok_or("process_io")?;
        let stderr = child.stderr.take().ok_or("process_io")?;
        let output = read_restic_output(stdout, &self.status, progress, capture);
        let discarded = discard_output(stderr);
        let waited = async { child.wait().await.map_err(|_| "process_wait_failed") };
        let (status, output, ()) = tokio::try_join!(waited, output, discarded)?;
        if !status.success() {
            return Err("restic_operation_failed");
        }
        Ok(output)
    }
}

async fn read_restic_output<R: AsyncRead + Unpin>(
    source: R,
    status: &Mutex<BackupStatus>,
    progress: bool,
    capture: bool,
) -> Result<ParsedOutput, &'static str> {
    let mut reader = BufReader::new(source);
    let mut parsed = ParsedOutput::default();
    let mut line = Vec::with_capacity(8192);
    loop {
        line.clear();
        let count = reader
            .read_until(b'\n', &mut line)
            .await
            .map_err(|_| "process_io")?;
        if count == 0 {
            break;
        }
        if line.len() > MAX_OUTPUT_LINE {
            line.zeroize();
            return Err("restic_output_limit");
        }
        if capture {
            if parsed.captured.len().saturating_add(line.len()) > MAX_CAPTURE_OUTPUT {
                line.zeroize();
                return Err("restic_output_limit");
            }
            parsed.captured.extend_from_slice(&line);
        }
        if progress && let Ok(value) = serde_json::from_slice::<ProgressMessage>(&line) {
            match value.message_type.as_str() {
                "status" => {
                    let mut current = status.lock().await;
                    current.bytes_done = value
                        .bytes_done
                        .or(value.bytes_restored)
                        .unwrap_or(current.bytes_done);
                    current.total_bytes = value.total_bytes.unwrap_or(current.total_bytes);
                    current.files_done = value
                        .files_done
                        .or(value.files_restored)
                        .unwrap_or(current.files_done);
                    current.total_files = value.total_files.unwrap_or(current.total_files);
                }
                "summary" => {
                    parsed.summary_seen = true;
                    if let Some(snapshot) = value.snapshot_id
                        && valid_snapshot_id(&snapshot)
                    {
                        parsed.snapshot_id = Some(snapshot);
                    }
                    let mut current = status.lock().await;
                    current.bytes_done = value
                        .total_bytes_processed
                        .or(value.bytes_restored)
                        .unwrap_or(current.bytes_done);
                    current.total_bytes = current.total_bytes.max(current.bytes_done);
                    current.files_done = value
                        .total_files_processed
                        .or(value.files_restored)
                        .unwrap_or(current.files_done);
                    current.total_files = current.total_files.max(current.files_done);
                }
                _ => (),
            }
        }
        line.zeroize();
    }
    Ok(parsed)
}

async fn discard_output<R: AsyncRead + Unpin>(mut source: R) -> Result<(), &'static str> {
    let mut buffer = [0u8; 8192];
    loop {
        let count = source.read(&mut buffer).await.map_err(|_| "process_io")?;
        buffer[..count].zeroize();
        if count == 0 {
            return Ok(());
        }
    }
}

fn parse_snapshot_history(
    bytes: &[u8],
    project: &Project,
) -> Result<(Vec<SnapshotEntry>, bool), &'static str> {
    let snapshots: Vec<ResticSnapshot> =
        serde_json::from_slice(bytes).map_err(|_| "backup_history_invalid")?;
    let project_tag = format!("project:{}", project.id);
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for snapshot in snapshots {
        if snapshot.paths.as_slice() != [project.path.as_str()]
            || !snapshot.tags.iter().any(|tag| tag == "hetzner-drive")
            || !snapshot.tags.iter().any(|tag| tag == &project_tag)
        {
            continue;
        }
        if !valid_snapshot_id(&snapshot.id)
            || !safe_snapshot_time(&snapshot.time)
            || !seen.insert(snapshot.id.clone())
        {
            return Err("backup_history_invalid");
        }
        let (files, bytes) = snapshot
            .summary
            .map(|summary| (summary.total_files_processed, summary.total_bytes_processed))
            .unwrap_or_default();
        entries.push(SnapshotEntry {
            id: snapshot.id,
            created_at: snapshot.time,
            files,
            bytes,
        });
    }
    entries.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    let truncated = entries.len() > MAX_SNAPSHOT_HISTORY;
    entries.truncate(MAX_SNAPSHOT_HISTORY);
    Ok((entries, truncated))
}

fn parse_snapshot_entries(
    bytes: &[u8],
    project: &Project,
    snapshot_id: &str,
) -> Result<(Vec<SnapshotTreeEntry>, bool), &'static str> {
    let project_tag = format!("project:{}", project.id);
    let project_root = Path::new(&project.path);
    let mut snapshot_seen = false;
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let record: ResticLsRecord =
            serde_json::from_slice(line).map_err(|_| "backup_contents_invalid")?;
        match record.struct_type.as_str() {
            "snapshot" => {
                if snapshot_seen
                    || record.id.as_deref() != Some(snapshot_id)
                    || record
                        .paths
                        .as_ref()
                        .is_none_or(|paths| paths.as_slice() != [project.path.as_str()])
                    || !record.tags.iter().any(|tag| tag == "hetzner-drive")
                    || !record.tags.iter().any(|tag| tag == &project_tag)
                {
                    return Err("backup_contents_invalid");
                }
                snapshot_seen = true;
            }
            "node" => {
                if !snapshot_seen {
                    return Err("backup_contents_invalid");
                }
                let path = record.path.ok_or("backup_contents_invalid")?;
                let full_path = Path::new(&path);
                let relative = full_path
                    .strip_prefix(project_root)
                    .map_err(|_| "backup_contents_invalid")?;
                if relative.as_os_str().is_empty() {
                    continue;
                }
                let relative = relative.to_str().ok_or("backup_contents_invalid")?;
                if !valid_snapshot_relative_path(relative) {
                    return Err("backup_contents_invalid");
                }
                let Some(kind) = record.node_type.as_deref() else {
                    return Err("backup_contents_invalid");
                };
                if !matches!(kind, "file" | "dir") {
                    continue;
                }
                if !seen.insert(relative.to_owned()) {
                    return Err("backup_contents_invalid");
                }
                let size = if kind == "file" {
                    record.size.ok_or("backup_contents_invalid")?
                } else {
                    record.size.unwrap_or(0)
                };
                entries.push(SnapshotTreeEntry {
                    path: relative.into(),
                    kind: kind.into(),
                    size,
                });
            }
            _ => return Err("backup_contents_invalid"),
        }
    }
    if !snapshot_seen {
        return Err("backup_contents_invalid");
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    let truncated = entries.len() > MAX_SNAPSHOT_ENTRIES;
    entries.truncate(MAX_SNAPSHOT_ENTRIES);
    Ok((entries, truncated))
}

fn safe_snapshot_time(value: &str) -> bool {
    (20..=64).contains(&value.len())
        && value.contains('T')
        && value.bytes().all(|byte| {
            byte.is_ascii_digit() || matches!(byte, b'-' | b':' | b'.' | b'+' | b'T' | b'Z')
        })
}

fn executable(path: &Path) -> Result<PathBuf, &'static str> {
    let canonical = fs::canonicalize(path).map_err(|_| {
        if path.ends_with("restic") {
            "restic_unavailable"
        } else {
            "rclone_unavailable"
        }
    })?;
    let metadata = fs::metadata(&canonical).map_err(|_| "backup_dependency_unsafe")?;
    let uid = unsafe { libc::geteuid() };
    if !metadata.is_file()
        || !no_symlink_components(&canonical)
        || (metadata.uid() != 0 && metadata.uid() != uid)
        || metadata.permissions().mode() & 0o022 != 0
    {
        return Err("backup_dependency_unsafe");
    }
    Ok(canonical)
}

fn safe_executable_text(path: &Path) -> Result<&str, &'static str> {
    let text = path.to_str().ok_or("helper_path")?;
    if text
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-'))
    {
        Ok(text)
    } else {
        Err("helper_path")
    }
}

fn load_project(
    paths: &BackupPaths,
    project_id: &str,
    require_available: bool,
) -> Result<Project, &'static str> {
    if !valid_project_id(project_id) {
        return Err("project_id_invalid");
    }
    let file = open_owned_file(&paths.projects).map_err(|_| "project_registry_unavailable")?;
    let metadata = file
        .metadata()
        .map_err(|_| "project_registry_unavailable")?;
    if metadata.permissions().mode() & 0o777 != 0o600 || metadata.len() > MAX_REGISTRY_BYTES {
        return Err("project_registry_unsafe");
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_REGISTRY_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "project_registry_unavailable")?;
    if bytes.len() as u64 > MAX_REGISTRY_BYTES {
        return Err("project_registry_unsafe");
    }
    let registry: Registry =
        serde_json::from_slice(&bytes).map_err(|_| "project_registry_invalid")?;
    if registry.version != 1 || registry.projects.len() > MAX_PROJECTS {
        return Err("project_registry_invalid");
    }
    let mut roots = Vec::with_capacity(registry.projects.len());
    let mut selected = None;
    for project in registry.projects {
        validate_project_schema(&project, &paths.mount)?;
        let root = PathBuf::from(&project.path);
        if roots.iter().any(|other: &PathBuf| {
            root == *other || root.starts_with(other) || other.starts_with(&root)
        }) {
            return Err("project_registry_overlap");
        }
        roots.push(root);
        if project.id == project_id {
            if selected.is_some() {
                return Err("project_registry_invalid");
            }
            selected = Some(project);
        }
    }
    let project = selected.ok_or("project_not_found")?;
    if require_available {
        let root = Path::new(&project.path);
        let canonical = fs::canonicalize(root).map_err(|_| "project_path_unavailable")?;
        let metadata = fs::symlink_metadata(root).map_err(|_| "project_path_unavailable")?;
        if canonical != root
            || !metadata.is_dir()
            || metadata.uid() != unsafe { libc::geteuid() }
            || !no_symlink_components(root)
        {
            return Err("project_path_unsafe");
        }
    }
    Ok(project)
}

fn validate_project_schema(project: &Project, mount: &Path) -> Result<(), &'static str> {
    let root = Path::new(&project.path);
    if !valid_project_id(&project.id)
        || project.name.trim().is_empty()
        || project.name.len() > 80
        || project.name.chars().any(char::is_control)
        || !validate_path(root)
        || root == mount
        || root.starts_with(mount)
        || mount.starts_with(root)
        || !(15..=7 * 24 * 60).contains(&project.interval_minutes)
        || !(1..=120).contains(&project.quiet_minutes)
        || project.mode != "ask"
        || project.excludes.len() > 64
        || !project.excludes.iter().all(|pattern| safe_pattern(pattern))
    {
        return Err("project_registry_invalid");
    }
    Ok(())
}

fn safe_pattern(pattern: &str) -> bool {
    (1..=64).contains(&pattern.len())
        && pattern.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'.' | b'_' | b'*' | b'?' | b'~' | b'#' | b'-')
        })
}

fn valid_project_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_snapshot_id(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_snapshot_relative_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_SNAPSHOT_PATH
        && !value.chars().any(char::is_control)
        && Path::new(value)
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

fn escape_restic_pattern(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(character, '\\' | '*' | '?' | '[') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

fn restore_arguments(
    snapshot_id: &str,
    project_path: &str,
    selected_path: Option<&str>,
    target: &Path,
) -> Vec<String> {
    let mut arguments = vec!["restore".into(), format!("{snapshot_id}:{project_path}")];
    if let Some(path) = selected_path {
        arguments.extend([
            "--include".into(),
            format!("/{}", escape_restic_pattern(path)),
        ]);
    }
    arguments.extend([
        "--target".into(),
        target.to_string_lossy().into_owned(),
        "--verify".into(),
        "--json".into(),
    ]);
    arguments
}

fn ensure_private_directory(path: &Path) -> Result<(), &'static str> {
    if !validate_path(path) {
        return Err("restore_target_failed");
    }
    if !path.exists() {
        fs::DirBuilder::new()
            .mode(0o700)
            .create(path)
            .map_err(|_| "restore_target_failed")?;
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| "restore_target_failed")?;
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o077 != 0
        || !no_symlink_components(path)
    {
        return Err("restore_target_unsafe");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::process::Command as StdCommand;

    fn private_file(path: &Path, contents: &str) {
        let mut file = fs::File::create(path).unwrap();
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .unwrap();
        file.write_all(contents.as_bytes()).unwrap();
    }

    fn fixture_transport() -> crate::Paths {
        crate::Paths {
            key_file: user_home().join(".ssh/fixture-storagebox"),
            sftp_host: "u12345.your-storagebox.de".into(),
            sftp_user: "u12345".into(),
            ..crate::Paths::default()
        }
    }

    #[test]
    fn registry_accepts_only_fixed_strict_project_ids_and_safe_roots() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        fs::create_dir(&project).unwrap();
        let registry = directory.path().join("projects.json");
        let paths = BackupPaths {
            projects: registry.clone(),
            mount: directory.path().join("mount"),
            restic: "/bin/false".into(),
            rclone: "/bin/false".into(),
            restore_root: directory.path().join("restore"),
            cache: directory.path().join("cache"),
            transport: fixture_transport(),
            repository: REPOSITORY.into(),
            helper: None,
        };
        private_file(
            &registry,
            &serde_json::json!({
                "version": 1,
                "projects": [{
                    "id": "a".repeat(32), "name": "Fixture", "path": project,
                    "interval_minutes": 120, "quiet_minutes": 10, "mode": "ask",
                    "excludes": ["target", "*.tmp"]
                }]
            })
            .to_string(),
        );
        assert_eq!(
            load_project(&paths, &"a".repeat(32), true).unwrap().name,
            "Fixture"
        );
        assert_eq!(
            load_project(&paths, "../unsafe", true).unwrap_err(),
            "project_id_invalid"
        );
        private_file(
            &registry,
            &serde_json::json!({"version": 1, "projects": [], "unknown": true}).to_string(),
        );
        assert_eq!(
            load_project(&paths, &"a".repeat(32), true).unwrap_err(),
            "project_registry_invalid"
        );
    }

    #[test]
    fn command_policy_has_a_disposable_append_only_remote() {
        assert!(REPOSITORY.starts_with("rclone:hetzner-crypt:"));
        assert!(REPOSITORY.contains("Disposable"));
        for forbidden in ["forget", "prune", "delete", "purge"] {
            assert!(!REPOSITORY.contains(forbidden));
        }
        assert!(safe_pattern("node_modules"));
        assert!(!safe_pattern("../../escape"));
        assert!(!safe_pattern("name with spaces"));
    }

    #[tokio::test]
    async fn available_engine_starts_locked_instead_of_unavailable() {
        let manager = BackupManager::new(true);
        let status = manager.status().await;
        assert!(status.available);
        assert!(!status.repository_unlocked);
        assert_eq!(status.phase, "locked");
    }

    #[test]
    fn snapshot_history_filters_by_exact_project_and_rejects_unsafe_entries() {
        let project = Project {
            id: "a".repeat(32),
            name: "Fixture".into(),
            path: "/tmp/fixture-project".into(),
            interval_minutes: 120,
            quiet_minutes: 10,
            mode: "ask".into(),
            excludes: Vec::new(),
        };
        let history = serde_json::json!([{
            "id": "b".repeat(64),
            "time": "2026-09-10T12:00:00Z",
            "paths": [project.path.clone()],
            "tags": ["hetzner-drive", format!("project:{}", project.id)],
            "summary": {"total_files_processed": 3, "total_bytes_processed": 1024}
        }, {
            "id": "c".repeat(64),
            "time": "2026-09-10T13:00:00Z",
            "paths": ["/tmp/a-different-project"],
            "tags": ["hetzner-drive", format!("project:{}", project.id)]
        }]);
        let (entries, truncated) =
            parse_snapshot_history(history.to_string().as_bytes(), &project).unwrap();
        assert!(!truncated);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "b".repeat(64));
        assert_eq!(entries[0].files, 3);
        assert_eq!(entries[0].bytes, 1024);

        let unsafe_history = serde_json::json!([{
            "id": "../unsafe",
            "time": "2026-09-10T12:00:00Z",
            "paths": [project.path.clone()],
            "tags": ["hetzner-drive", format!("project:{}", project.id)]
        }]);
        assert_eq!(
            parse_snapshot_history(unsafe_history.to_string().as_bytes(), &project).unwrap_err(),
            "backup_history_invalid"
        );
    }

    #[test]
    fn snapshot_contents_are_bounded_typed_and_project_relative() {
        let project = Project {
            id: "b".repeat(32),
            name: "Fixture".into(),
            path: "/home/example/project".into(),
            interval_minutes: 120,
            quiet_minutes: 10,
            mode: "ask".into(),
            excludes: Vec::new(),
        };
        let snapshot = "c".repeat(64);
        let records = [
            serde_json::json!({
                "struct_type": "snapshot", "id": snapshot,
                "paths": [project.path], "tags": ["hetzner-drive", format!("project:{}", project.id)]
            }),
            serde_json::json!({
                "struct_type": "node", "path": "/home/example/project",
                "type": "dir", "size": 0
            }),
            serde_json::json!({
                "struct_type": "node", "path": "/home/example/project/src",
                "type": "dir", "size": 0
            }),
            serde_json::json!({
                "struct_type": "node", "path": "/home/example/project/src/main.rs",
                "type": "file", "size": 42
            }),
            serde_json::json!({
                "struct_type": "node", "path": "/home/example/project/link",
                "type": "symlink", "size": 0
            }),
        ];
        let output = records
            .iter()
            .map(serde_json::Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        let (entries, truncated) =
            parse_snapshot_entries(output.as_bytes(), &project, &snapshot).unwrap();
        assert!(!truncated);
        assert_eq!(
            entries,
            vec![
                SnapshotTreeEntry {
                    path: "src".into(),
                    kind: "dir".into(),
                    size: 0,
                },
                SnapshotTreeEntry {
                    path: "src/main.rs".into(),
                    kind: "file".into(),
                    size: 42,
                },
            ]
        );
        for unsafe_record in [
            serde_json::json!({
                "struct_type": "node", "path": "/home/example/elsewhere/file",
                "type": "file", "size": 1
            }),
            serde_json::json!({
                "struct_type": "node", "path": "/home/example/project/../escape",
                "type": "file", "size": 1
            }),
        ] {
            let changed = format!("{}\n{}", records[0], unsafe_record);
            assert_eq!(
                parse_snapshot_entries(changed.as_bytes(), &project, &snapshot),
                Err("backup_contents_invalid")
            );
        }
        assert!(!valid_snapshot_relative_path("../escape"));
        assert!(!valid_snapshot_relative_path("/absolute"));
        assert!(!valid_snapshot_relative_path("line\nbreak"));
        let mut large = vec![records[0].to_string()];
        for index in 0..=MAX_SNAPSHOT_ENTRIES {
            large.push(
                serde_json::json!({
                    "struct_type": "node",
                    "path": format!("/home/example/project/file-{index:04}"),
                    "type": "file",
                    "size": index,
                })
                .to_string(),
            );
        }
        let (entries, truncated) =
            parse_snapshot_entries(large.join("\n").as_bytes(), &project, &snapshot).unwrap();
        assert!(truncated);
        assert_eq!(entries.len(), MAX_SNAPSHOT_ENTRIES);
        let arguments = restore_arguments(
            &snapshot,
            &project.path,
            Some("draft/[copy]*?.txt"),
            Path::new("/tmp/restore"),
        );
        assert_eq!(
            arguments,
            vec![
                "restore".to_owned(),
                format!("{snapshot}:{}", project.path),
                "--include".to_owned(),
                "/draft/\\[copy]\\*\\?.txt".to_owned(),
                "--target".to_owned(),
                "/tmp/restore".to_owned(),
                "--verify".to_owned(),
                "--json".to_owned(),
            ]
        );
        assert!(!arguments.iter().any(|argument| argument == "--delete"));
        assert_eq!(
            escape_restic_pattern("literal[*]?\\name"),
            r"literal\[\*]\?\\name"
        );
    }

    #[tokio::test]
    async fn demo_restore_requires_a_snapshot_authorized_by_history() {
        let manager = BackupManager::new(true);
        manager.demo_configuration_unlocked(true).await;
        let project_id = "a".repeat(32);
        let snapshot_id = "a".repeat(64);
        assert_eq!(
            manager.start_demo(&project_id, Some(&snapshot_id)).await,
            (false, "backup_snapshot_unavailable".into())
        );
        manager.demo_snapshots(&project_id).await.unwrap();
        manager.lock().await;
        assert_eq!(
            manager.start_demo(&project_id, Some(&snapshot_id)).await,
            (false, "backup_snapshot_unavailable".into())
        );
        manager.demo_configuration_unlocked(true).await;
        manager.demo_snapshots(&project_id).await.unwrap();
        assert_eq!(
            manager.start_demo(&project_id, Some(&snapshot_id)).await,
            (true, "accepted".into())
        );
    }

    #[tokio::test]
    async fn demo_selective_restore_requires_a_listed_entry() {
        let manager = BackupManager::new(true);
        let project_id = "b".repeat(32);
        let snapshot_id = "a".repeat(64);
        manager.demo_configuration_unlocked(true).await;
        manager.demo_snapshots(&project_id).await.unwrap();
        assert_eq!(
            manager
                .start_demo_selective_restore(&project_id, &snapshot_id, "src/main.rs")
                .await,
            (false, "backup_entry_unavailable".into())
        );
        manager
            .demo_snapshot_entries(&project_id, &snapshot_id)
            .await
            .unwrap();
        assert_eq!(
            manager
                .start_demo_selective_restore(&project_id, &snapshot_id, "not-listed.txt")
                .await,
            (false, "backup_entry_unavailable".into())
        );
        manager.lock().await;
        assert_eq!(
            manager
                .start_demo_selective_restore(&project_id, &snapshot_id, "src/main.rs")
                .await,
            (false, "backup_entry_unavailable".into())
        );
        manager.demo_configuration_unlocked(true).await;
        manager.demo_snapshots(&project_id).await.unwrap();
        manager
            .demo_snapshot_entries(&project_id, &snapshot_id)
            .await
            .unwrap();
        assert_eq!(
            manager
                .start_demo_selective_restore(&project_id, &snapshot_id, "src/main.rs")
                .await,
            (true, "accepted".into())
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn real_restic_uses_both_peer_checked_helpers_and_restores_verified_content() {
        if !Path::new("/usr/bin/restic").is_file() || !Path::new("/usr/local/bin/rclone").is_file()
        {
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let helper = directory.path().join("secret-helper.py");
        fs::write(
            &helper,
            r#"#!/usr/bin/python3
import os, socket, struct, sys
if "--fixture-password" in sys.argv:
    sys.stdout.write("synthetic-config-password")
    raise SystemExit(0)
variable = "HETZNER_RESTIC_PASSWORD_SOCKET" if "--restic-password-helper" in sys.argv else "HETZNER_PASSWORD_SOCKET"
connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
connection.connect(os.environ[variable])
pid, uid, _ = struct.unpack("3i", connection.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))
if pid != int(os.environ["HETZNER_CONTROLLER_PID"]) or uid != os.getuid():
    raise SystemExit(3)
value = bytearray()
while True:
    part = connection.recv(4096)
    if not part:
        break
    value.extend(part)
sys.stdout.buffer.write(value + b"\n")
for index in range(len(value)):
    value[index] = 0
"#,
        )
        .unwrap();
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();

        let config_path = directory.path().join("rclone.conf");
        private_file(&config_path, "[unused]\ntype = local\n");
        let encryption = StdCommand::new("/usr/local/bin/rclone")
            .env_clear()
            .env("LC_ALL", "C")
            .args([
                "--password-command",
                &format!("{} --fixture-password", helper.display()),
                "config",
                "encryption",
                "set",
                "--ask-password=false",
                "--config",
            ])
            .arg(&config_path)
            .output()
            .unwrap();
        assert!(
            encryption.status.success(),
            "{}",
            String::from_utf8_lossy(&encryption.stderr)
        );
        let config = EncryptedConfig::capture(&config_path).unwrap();
        let config_password = SessionSecret::new(b"synthetic-config-password").unwrap();
        let repository_password = SessionSecret::new(b"synthetic-repository-password").unwrap();
        let source = directory.path().join("source");
        fs::DirBuilder::new().mode(0o700).create(&source).unwrap();
        private_file(&source.join("document.txt"), "synthetic restic content\n");
        fs::DirBuilder::new()
            .mode(0o700)
            .create(source.join("draft"))
            .unwrap();
        private_file(
            &source.join("draft/[copy]*?.txt"),
            "selected wildcard name\n",
        );
        private_file(
            &source.join("draft/copy-secret.txt"),
            "unselected sibling\n",
        );
        let remote = directory.path().join("remote");
        fs::DirBuilder::new().mode(0o700).create(&remote).unwrap();
        let restore = directory.path().join("restore");
        fs::DirBuilder::new().mode(0o700).create(&restore).unwrap();
        let paths = BackupPaths {
            restic: "/usr/bin/restic".into(),
            rclone: "/usr/local/bin/rclone".into(),
            projects: directory.path().join("projects.json"),
            mount: directory.path().join("mount"),
            restore_root: directory.path().join("restore-root"),
            cache: directory.path().join("cache"),
            transport: fixture_transport(),
            repository: format!("rclone:{}", remote.display()),
            helper: Some(helper),
        };
        let manager = BackupManager::with_paths(false, paths);
        manager
            .run_restic(
                vec!["init".into()],
                &config,
                &config_password,
                &repository_password,
                false,
            )
            .await
            .unwrap();
        let backup = manager
            .run_restic(
                vec![
                    "backup".into(),
                    source.to_string_lossy().into_owned(),
                    "--json".into(),
                    "--one-file-system".into(),
                    "--tag".into(),
                    "hetzner-drive".into(),
                    "--tag".into(),
                    format!("project:{}", "b".repeat(32)),
                ],
                &config,
                &config_password,
                &repository_password,
                true,
            )
            .await
            .unwrap();
        let snapshot = backup.snapshot_id.unwrap();
        manager
            .run_restic(
                vec!["check".into(), "--with-cache".into()],
                &config,
                &config_password,
                &repository_password,
                false,
            )
            .await
            .unwrap();
        let history_bytes = manager
            .run_restic_capture(
                vec![
                    "snapshots".into(),
                    "--json".into(),
                    "--tag".into(),
                    format!("hetzner-drive,project:{}", "b".repeat(32)),
                    "--path".into(),
                    source.to_string_lossy().into_owned(),
                ],
                &config,
                &config_password,
                &repository_password,
            )
            .await
            .unwrap();
        let history_project = Project {
            id: "b".repeat(32),
            name: "Fixture".into(),
            path: source.to_string_lossy().into_owned(),
            interval_minutes: 120,
            quiet_minutes: 10,
            mode: "ask".into(),
            excludes: Vec::new(),
        };
        let (history, truncated) =
            parse_snapshot_history(&history_bytes, &history_project).unwrap();
        assert!(!truncated);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].id, snapshot);
        assert_eq!(history[0].files, 3);
        assert_eq!(
            history[0].bytes,
            (b"synthetic restic content\n".len()
                + b"selected wildcard name\n".len()
                + b"unselected sibling\n".len()) as u64
        );
        let contents_bytes = manager
            .run_restic_capture(
                vec![
                    "ls".into(),
                    "--json".into(),
                    "--recursive".into(),
                    "--sort".into(),
                    "name".into(),
                    snapshot.clone(),
                    source.to_string_lossy().into_owned(),
                ],
                &config,
                &config_password,
                &repository_password,
            )
            .await
            .unwrap();
        let (contents, contents_truncated) =
            parse_snapshot_entries(&contents_bytes, &history_project, &snapshot).unwrap();
        assert!(!contents_truncated);
        assert!(contents.iter().any(|entry| {
            entry.path == "draft/[copy]*?.txt"
                && entry.kind == "file"
                && entry.size == b"selected wildcard name\n".len() as u64
        }));
        assert!(
            contents
                .iter()
                .any(|entry| entry.path == "draft" && entry.kind == "dir")
        );
        let selective_restore = directory.path().join("selective-restore");
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&selective_restore)
            .unwrap();
        let selectively_restored = manager
            .run_restic(
                restore_arguments(
                    &snapshot,
                    &source.to_string_lossy(),
                    Some("draft/[copy]*?.txt"),
                    &selective_restore,
                ),
                &config,
                &config_password,
                &repository_password,
                true,
            )
            .await
            .unwrap();
        assert!(selectively_restored.summary_seen);
        assert_eq!(
            fs::read(selective_restore.join("draft/[copy]*?.txt")).unwrap(),
            b"selected wildcard name\n"
        );
        assert!(!selective_restore.join("draft/copy-secret.txt").exists());
        let directory_restore = directory.path().join("directory-restore");
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory_restore)
            .unwrap();
        let directory_restored = manager
            .run_restic(
                restore_arguments(
                    &snapshot,
                    &source.to_string_lossy(),
                    Some("draft"),
                    &directory_restore,
                ),
                &config,
                &config_password,
                &repository_password,
                true,
            )
            .await
            .unwrap();
        assert!(directory_restored.summary_seen);
        assert_eq!(
            fs::read(directory_restore.join("draft/[copy]*?.txt")).unwrap(),
            b"selected wildcard name\n"
        );
        assert_eq!(
            fs::read(directory_restore.join("draft/copy-secret.txt")).unwrap(),
            b"unselected sibling\n"
        );
        let restored = manager
            .run_restic(
                vec![
                    "restore".into(),
                    format!("{snapshot}:{}", source.display()),
                    "--target".into(),
                    restore.to_string_lossy().into_owned(),
                    "--verify".into(),
                    "--json".into(),
                ],
                &config,
                &config_password,
                &repository_password,
                true,
            )
            .await
            .unwrap();
        assert!(restored.summary_seen);
        assert_eq!(
            fs::read(restore.join("document.txt")).unwrap(),
            b"synthetic restic content\n"
        );
        let locks = remote.join("locks");
        assert!(!locks.is_dir() || fs::read_dir(locks).unwrap().next().is_none());
    }
}
