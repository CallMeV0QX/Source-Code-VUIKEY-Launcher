//! updater.rs: Quản lý tự động cập nhật launcher
//! Giao tiếp với `VUIKEY.Update.exe`, ghi file `apply.json` và khởi chạy trình cài đặt mới
//! kèm các tham số xác minh PID và phiên bản.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;
use update_core::{Release, UpdateChecker, UpdateError};

#[derive(Error, Debug)]
pub enum LauncherUpdateError {
    #[error("Lỗi core updater: {0}")]
    Core(#[from] UpdateError),
    #[error("Kết thúc phiên chơi trước khi cập nhật VUIKEY.")]
    SessionActive,
    #[error("Không xác minh được thư mục cài VUIKEY. Hãy chạy setup mới một lần.")]
    InstallDirNotFound,
    #[error("Bản này cần chạy setup mới một lần để bật tự cập nhật.")]
    SetupRequired,
    #[error("Lỗi I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("Lỗi JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyConfig {
    pub parent_pid: u32,
    pub parent_created: u64,
    pub current_version: String,
    pub target_version: String,
    pub nonce: String,
    pub installer_path: String,
}

pub struct Updater {
    checker: UpdateChecker,
    app_dir: PathBuf,
    current_version: String,
}

impl Updater {
    pub fn new(app_dir: PathBuf, current_version: &str) -> Self {
        Self {
            checker: UpdateChecker::default(),
            app_dir,
            current_version: current_version.to_string(),
        }
    }

    /// Kiểm tra phiên bản mới từ máy chủ
    pub async fn check_for_update(&self) -> Result<Option<Release>, LauncherUpdateError> {
        let release = self.checker.check_latest().await?;
        if release.version != self.current_version {
            Ok(Some(release))
        } else {
            Ok(None)
        }
    }

    /// Tải bản cập nhật và chuẩn bị thực thi
    pub async fn prepare_update(&self, release: &Release) -> Result<PathBuf, LauncherUpdateError> {
        let updates_dir = self.app_dir.join("updates");
        std::fs::create_dir_all(&updates_dir)?;

        let target_exe = updates_dir.join(format!("vuikey-update-{}.exe", release.version));
        self.checker.download_and_verify(release, &target_exe).await?;

        // Ghi apply.json
        let apply_cfg = ApplyConfig {
            parent_pid: std::process::id(),
            parent_created: 0,
            current_version: self.current_version.clone(),
            target_version: release.version.clone(),
            nonce: format!("{:x}", rand_u64()),
            installer_path: target_exe.to_string_lossy().to_string(),
        };

        let apply_file = updates_dir.join("apply.json");
        std::fs::write(&apply_file, serde_json::to_string_pretty(&apply_cfg)?)?;

        Ok(target_exe)
    }

    /// Khởi chạy VUIKEY.Update.exe để hoán đổi tệp thực thi
    pub fn launch_updater(&self, installer_path: &Path) -> Result<(), LauncherUpdateError> {
        let updater_exe = self.app_dir.join("VUIKEY.Update.exe");
        let exe_to_run = if updater_exe.exists() {
            updater_exe
        } else {
            installer_path.to_path_buf()
        };

        Command::new(exe_to_run)
            .arg("--apply-update")
            .arg(format!("--pid={}", std::process::id()))
            .spawn()?;

        Ok(())
    }
}

fn rand_u64() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
}

