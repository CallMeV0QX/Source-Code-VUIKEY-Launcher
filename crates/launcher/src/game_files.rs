//! game_files.rs: Can thiệp và chỉnh sửa file game gốc sau khi tải từ Steam
//! Chuyên dụng cho AppID 1174180 (Red Dead Redemption 2 - RDR2):
//! Thay thế `Launcher.exe`, patch các DLL như `nvngx_dlss.dll`, `oo2core_5_win64.dll`
//! để bẻ khóa (crack/bypass) Rockstar Games Launcher và DRM Denuvo.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const RDR2_APP_ID: u32 = 1174180;

#[derive(Error, Debug)]
pub enum GameFileError {
    #[error("Thiếu bộ file đi kèm. Cài lại VUIKEY để khôi phục.")]
    MissingCompanionFiles,
    #[error("Bộ file đi kèm bị thiếu hoặc thay đổi. Cài lại VUIKEY để khôi phục.")]
    CorruptedCompanionFiles,
    #[error("Không tìm thấy thư viện game.")]
    GameLibraryNotFound,
    #[error("Không tìm thấy thư mục VUIKEY.")]
    VuikeyFolderNotFound,
    #[error("Đường dẫn không đúng loại file.")]
    InvalidFileType,
    #[error("Không chép qua liên kết thư mục/file.")]
    SymlinkBlocked,
    #[error("Lỗi I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("Lỗi JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileTarget {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupReceipt {
    pub app_id: u32,
    pub original_files: Vec<FileTarget>,
    pub created_at: u64,
}

pub struct GamePatcher {
    vuikey_dir: PathBuf,
}

impl GamePatcher {
    pub fn new(vuikey_dir: PathBuf) -> Self {
        Self { vuikey_dir }
    }

    /// Danh sách file cần kiểm tra hoặc thay thế đối với Red Dead Redemption 2
    pub fn rdr2_expected_files() -> Vec<FileTarget> {
        vec![
            FileTarget {
                path: "Launcher.exe".into(),
                bytes: 0, // Placeholder
                sha256: "".into(),
            },
            FileTarget {
                path: "nvngx_dlss.dll".into(),
                bytes: 14445424,
                sha256: "864a4cdd495cb499e28ddb8bf9b3e5fc67ccf365436a9548bc43a634a8baa1ff".into(),
            },
            FileTarget {
                path: "oo2core_5_win64.dll".into(),
                bytes: 945648,
                sha256: "3d93de36c473413110e4bf65174445790f469401a6d77db3a53858527088f611".into(),
            },
            FileTarget {
                path: "RDR2.exe".into(),
                bytes: 89562608,
                sha256: "9b8b8454616a6ef91c8d5655b231e5cd21aa2236ab8cb04c34fce09248a55fdc".into(),
            },
        ]
    }

    /// Can thiệp vào thư mục cài đặt game sau khi tải xong (rdr2-post-install)
    pub fn apply_rdr2_post_install(&self, game_dir: &Path) -> Result<(), GameFileError> {
        if !game_dir.exists() {
            return Err(GameFileError::GameLibraryNotFound);
        }

        let companion_dir = self.vuikey_dir.join("game-files").join("1174180");
        if !companion_dir.exists() {
            return Err(GameFileError::MissingCompanionFiles);
        }

        let backup_dir = game_dir.join(".vuikey-backups").join("1174180");
        std::fs::create_dir_all(&backup_dir)?;

        // Thay thế Launcher.exe bằng bản crack để bỏ qua Rockstar Launcher
        let target_launcher = game_dir.join("Launcher.exe");
        let crack_launcher = companion_dir.join("Launcher.exe");

        if target_launcher.exists() && crack_launcher.exists() {
            let backup_launcher = backup_dir.join("Launcher.exe.bak");
            if !backup_launcher.exists() {
                std::fs::copy(&target_launcher, &backup_launcher)?;
            }
            std::fs::copy(&crack_launcher, &target_launcher)?;
        }

        // Ghi biên lai sao lưu receipt.json
        let receipt = BackupReceipt {
            app_id: RDR2_APP_ID,
            original_files: Self::rdr2_expected_files(),
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        };
        let receipt_file = backup_dir.join("receipt.json");
        std::fs::write(&receipt_file, serde_json::to_string_pretty(&receipt)?)?;

        Ok(())
    }

    /// Khôi phục file game nguyên bản
    pub fn restore_original_files(&self, game_dir: &Path) -> Result<(), GameFileError> {
        let backup_dir = game_dir.join(".vuikey-backups").join("1174180");
        let backup_launcher = backup_dir.join("Launcher.exe.bak");
        let target_launcher = game_dir.join("Launcher.exe");

        if backup_launcher.exists() {
            std::fs::copy(&backup_launcher, &target_launcher)?;
            let _ = std::fs::remove_file(backup_launcher);
        }
        let _ = std::fs::remove_dir_all(&backup_dir);
        Ok(())
    }
}

