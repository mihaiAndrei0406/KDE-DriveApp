use hetzner_drive_core::{BUS, OBJECT, controller::Controller};
use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

struct Service {
    controller: Arc<Controller>,
}

#[zbus::interface(name = "ro.mihai.HetznerDrive1")]
impl Service {
    #[zbus(out_args(
        "state",
        "mount",
        "ssh",
        "config",
        "version",
        "mount_path",
        "cache_bytes",
        "cache_complete",
        "diagnostic"
    ))]
    async fn get_status(
        &self,
    ) -> (
        String,
        String,
        String,
        String,
        String,
        String,
        u64,
        bool,
        String,
    ) {
        let s = self.controller.observe().await;
        (
            format!("{:?}", s.state),
            s.mount,
            s.ssh,
            s.config,
            s.version,
            s.mount_path,
            s.cache_bytes,
            s.cache_complete,
            s.diagnostic,
        )
    }

    async fn check_ssh_agent(&self) -> String {
        self.controller.check_ssh().await
    }

    #[zbus(out_args("available", "total", "used", "free", "reason"))]
    async fn get_storage_usage(&self) -> (bool, u64, u64, u64, String) {
        match self.controller.storage_usage().await {
            Ok(usage) => (true, usage.total, usage.used, usage.free, "ok".into()),
            Err(reason) => (false, 0, 0, 0, reason),
        }
    }

    async fn get_recent_logs(&self) -> Vec<String> {
        self.controller.recent_logs().await
    }

    #[zbus(out_args("success", "code"))]
    async fn unlock_configuration(&self) -> (bool, String) {
        self.controller.unlock().await
    }

    #[zbus(out_args("success", "code"))]
    async fn lock_configuration(&self) -> (bool, String) {
        self.controller.lock().await
    }

    #[zbus(out_args("success", "code"))]
    async fn check_connection(&self) -> (bool, String) {
        self.controller.check_connection().await
    }

    #[zbus(out_args("success", "code"))]
    async fn run_health_check(&self) -> (bool, String) {
        self.controller.health_check().await
    }

    #[zbus(out_args("success", "code"))]
    async fn open_drive(&self) -> (bool, String) {
        self.controller.open_drive().await
    }

    #[zbus(out_args("success", "code"))]
    async fn mount_drive(&self) -> (bool, String) {
        self.controller.mount_drive().await
    }

    #[zbus(out_args("success", "code"))]
    async fn unmount_drive(&self) -> (bool, String) {
        self.controller.unmount_drive().await
    }

    #[zbus(out_args("busy", "last_operation", "last_error", "owned_mount"))]
    async fn get_operation_status(&self) -> (bool, String, String, bool) {
        let status = self.controller.operation_status().await;
        (
            status.busy,
            status.last_operation,
            status.last_error,
            status.owned_mount,
        )
    }

    #[zbus(out_args(
        "available",
        "uploads_queued",
        "uploads_in_progress",
        "errored_files",
        "out_of_space",
        "reason"
    ))]
    async fn get_mount_activity(&self) -> (bool, u64, u64, u64, bool, String) {
        let status = self.controller.mount_activity().await;
        (
            status.available,
            status.uploads_queued,
            status.uploads_in_progress,
            status.errored_files,
            status.out_of_space,
            status.reason,
        )
    }

    #[zbus(out_args(
        "available",
        "repository_initialized",
        "repository_unlocked",
        "busy",
        "phase",
        "project_id",
        "bytes_done",
        "total_bytes",
        "files_done",
        "total_files",
        "last_snapshot",
        "restore_path",
        "last_error"
    ))]
    async fn get_backup_status(
        &self,
    ) -> (
        bool,
        bool,
        bool,
        bool,
        String,
        String,
        u64,
        u64,
        u64,
        u64,
        String,
        String,
        String,
    ) {
        let status = self.controller.backup_status().await;
        (
            status.available,
            status.repository_initialized,
            status.repository_unlocked,
            status.busy,
            status.phase,
            status.project_id,
            status.bytes_done,
            status.total_bytes,
            status.files_done,
            status.total_files,
            status.last_snapshot,
            status.restore_path,
            status.last_error,
        )
    }

    #[zbus(out_args(
        "success",
        "truncated",
        "snapshot_ids",
        "created_at",
        "files",
        "bytes",
        "reason"
    ))]
    async fn get_project_snapshots(
        &self,
        project_id: String,
    ) -> (
        bool,
        bool,
        Vec<String>,
        Vec<String>,
        Vec<u64>,
        Vec<u64>,
        String,
    ) {
        match self.controller.project_snapshots(&project_id).await {
            Ok((entries, truncated)) => {
                let mut snapshot_ids = Vec::with_capacity(entries.len());
                let mut created_at = Vec::with_capacity(entries.len());
                let mut files = Vec::with_capacity(entries.len());
                let mut bytes = Vec::with_capacity(entries.len());
                for entry in entries {
                    snapshot_ids.push(entry.id);
                    created_at.push(entry.created_at);
                    files.push(entry.files);
                    bytes.push(entry.bytes);
                }
                (
                    true,
                    truncated,
                    snapshot_ids,
                    created_at,
                    files,
                    bytes,
                    "ok".into(),
                )
            }
            Err(reason) => (
                false,
                false,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                reason,
            ),
        }
    }

    #[zbus(out_args("success", "code"))]
    async fn initialize_backup_repository(&self) -> (bool, String) {
        self.controller.initialize_backup_repository().await
    }

    #[zbus(out_args("success", "code"))]
    async fn unlock_backup_repository(&self) -> (bool, String) {
        self.controller.unlock_backup_repository().await
    }

    #[zbus(out_args("success", "code"))]
    async fn lock_backup_repository(&self) -> (bool, String) {
        self.controller.lock_backup_repository().await
    }

    #[zbus(out_args("success", "code"))]
    async fn start_project_backup(&self, project_id: String) -> (bool, String) {
        self.controller.start_project_backup(&project_id).await
    }

    #[zbus(out_args("success", "code"))]
    async fn start_backup_restore(
        &self,
        project_id: String,
        snapshot_id: String,
    ) -> (bool, String) {
        self.controller
            .start_backup_restore(&project_id, &snapshot_id)
            .await
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if unsafe { libc::geteuid() } == 0 {
        return Err("Do not run this application as root".into());
    }
    unsafe {
        libc::umask(0o077);
        let limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if libc::setrlimit(libc::RLIMIT_CORE, &limit) != 0 {
            return Err("Cannot disable core dumps".into());
        }
    }
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--password-helper"] {
        return hetzner_drive_core::auth::password_helper()
            .await
            .map_err(Into::into);
    }
    if args == ["--restic-password-helper"] {
        return hetzner_drive_core::auth::restic_password_helper()
            .await
            .map_err(Into::into);
    }
    let demo = args.iter().any(|a| a == "--demo");
    if args
        .iter()
        .any(|a| !["--demo", "--inspect"].contains(&a.as_str()))
    {
        return Err("Usage: hetzner-drive-core [--demo] [--inspect]".into());
    }
    let controller = Arc::new(Controller::new(demo));
    if args.iter().any(|a| a == "--inspect") {
        println!(
            "{}",
            serde_json::to_string_pretty(&controller.observe().await)?
        );
        return Ok(());
    }
    let service = Service {
        controller: controller.clone(),
    };
    let _connection = zbus::connection::Builder::session()?
        .allow_name_replacements(false)
        .replace_existing_names(false)
        .name(BUS)?
        .serve_at(OBJECT, service)?
        .build()
        .await?;
    eprintln!(
        "{}",
        serde_json::json!({"timestamp": SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(), "level": "INFO", "operation": "service_started", "demo": demo})
    );
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
    let (safe, code) = controller.shutdown().await;
    if !safe {
        eprintln!(
            "{}",
            serde_json::json!({"timestamp": SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(), "level": "ERROR", "operation": "shutdown_refused", "result": code})
        );
        std::future::pending::<()>().await;
    }
    Ok(())
}
