//! steam_first_launch.rs: Giám sát tiến độ tải và khởi chạy lần đầu cho game Steam
//! Đọc file `steamapps/appmanifest_{app_id}.acf` để theo dõi BytesDownloaded vs BytesToDownload.
//! Khi tải xong hoặc khởi chạy lần đầu thành công, chuyển đổi trạng thái mạng của Steam
//! sang Offline nhằm qua mặt hạn chế kích hoạt của Denuvo và tránh xung đột nhiều người chơi cùng lúc.

use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ManifestError {
    #[error("Không tìm thấy appmanifest_{0}.acf")]
    NotFound(u32),
    #[error("Lỗi cú pháp file ACF")]
    ParseError,
    #[error("Không mở được trình duyệt đăng nhập Google")]
    BrowserLaunchFailed,
    #[error("Lỗi I/O: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Default)]
pub struct AppManifest {
    pub app_id: u32,
    pub install_dir: String,
    pub build_id: u64,
    pub bytes_to_download: u64,
    pub bytes_downloaded: u64,
    pub bytes_to_stage: u64,
    pub bytes_staged: u64,
    pub update_result: u32,
    pub target_build_id: u64,
}

impl AppManifest {
    /// Kiểm tra xem game đã tải xong chưa
    pub fn is_download_complete(&self) -> bool {
        self.bytes_to_download > 0 && self.bytes_downloaded >= self.bytes_to_download
    }
}

pub struct DownloadWatcher {
    steam_path: PathBuf,
    app_id: u32,
}

impl DownloadWatcher {
    pub fn new(steam_path: PathBuf, app_id: u32) -> Self {
        Self { steam_path, app_id }
    }

    /// Đọc và phân tích `appmanifest_{app_id}.acf`
    pub fn read_manifest(&self) -> Result<AppManifest, ManifestError> {
        let manifest_path = self
            .steam_path
            .join("steamapps")
            .join(format!("appmanifest_{}.acf", self.app_id));

        if !manifest_path.exists() {
            return Err(ManifestError::NotFound(self.app_id));
        }

        let content = std::fs::read_to_string(&manifest_path)?;
        let mut manifest = AppManifest {
            app_id: self.app_id,
            ..Default::default()
        };

        for line in content.lines() {
            let line = line.trim();
            if let Some((k, v)) = parse_acf_line(line) {
                match k.to_lowercase().as_str() {
                    "installdir" => manifest.install_dir = v.to_string(),
                    "buildid" => manifest.build_id = v.parse().unwrap_or(0),
                    "bytestodownload" => manifest.bytes_to_download = v.parse().unwrap_or(0),
                    "bytesdownloaded" => manifest.bytes_downloaded = v.parse().unwrap_or(0),
                    "bytestostage" => manifest.bytes_to_stage = v.parse().unwrap_or(0),
                    "bytesstaged" => manifest.bytes_staged = v.parse().unwrap_or(0),
                    "updateresult" => manifest.update_result = v.parse().unwrap_or(0),
                    "targetbuildid" => manifest.target_build_id = v.parse().unwrap_or(0),
                    _ => {}
                }
            }
        }

        Ok(manifest)
    }

    /// Mở trình duyệt ngoài cho luồng đăng nhập Google OAuth
    pub fn open_google_auth_url(url: &str) -> Result<(), ManifestError> {
        #[cfg(windows)]
        {
            use std::process::Command;
            Command::new("cmd")
                .args(["/C", "start", "", url])
                .spawn()
                .map_err(|_| ManifestError::BrowserLaunchFailed)?;
            Ok(())
        }
        #[cfg(not(windows))]
        {
            let _ = url;
            Err(ManifestError::BrowserLaunchFailed)
        }
    }
}

fn parse_acf_line(line: &str) -> Option<(&str, &str)> {
    let parts: Vec<&str> = line.split('"').collect();
    if parts.len() >= 4 {
        Some((parts[1], parts[3]))
    } else {
        None
    }
}

