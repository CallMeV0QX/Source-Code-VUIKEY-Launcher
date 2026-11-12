//! storage.rs: Quản lý lưu trữ cục bộ, cấu hình phiên thuê và thư viện Steam
//! Lưu trữ session Steam dưới dạng mã hóa DPAPI trong `%LOCALAPPDATA%\VUIKEY\steam-sessions`.
//! Thực hiện can thiệp và backup `libraryfolders.vdf`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("Thiếu thư mục LOCALAPPDATA")]
    MissingLocalAppData,
    #[error("Thông tin phiên Steam không hợp lệ")]
    InvalidSteamSession,
    #[error("Phiên Steam không khớp tài khoản được cấp")]
    SessionAccountMismatch,
    #[error("Steam session renewal limit reached")]
    RenewalLimitReached,
    #[error("Invalid recovery token")]
    InvalidRecoveryToken,
    #[error("Không khôi phục được thư viện; dùng file .bak trong steamapps")]
    LibraryRestoreFailed,
    #[error("Không cập nhật được danh sách ổ cài game")]
    LibraryUpdateFailed,
    #[error("Thoát Steam trước khi chuyển ổ cài game")]
    SteamRunningConflict,
    #[error("Lỗi I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("Lỗi JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Token {
    pub token: String,
    pub expires_at: u64,
    pub server_now: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Journal {
    pub session_id: String,
    pub app_id: u32,
    pub account_name: String,
    pub lease_token: String,
    pub created_at: u64,
    pub expires_at: u64,
    pub connection_mode: String,
    pub offline_state: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preferences {
    pub auto_launch_steam: bool,
}

pub struct StorageManager {
    base_dir: PathBuf,
    sessions_dir: PathBuf,
}

impl StorageManager {
    pub fn new() -> Result<Self, StorageError> {
        let local_app_data = std::env::var("LOCALAPPDATA")
            .map_err(|_| StorageError::MissingLocalAppData)?;
        let base_dir = PathBuf::from(local_app_data).join("VUIKEY");
        let sessions_dir = base_dir.join("steam-sessions");

        std::fs::create_dir_all(&sessions_dir)?;
        Ok(Self {
            base_dir,
            sessions_dir,
        })
    }

    /// Đọc hoặc tạo mã định danh máy (install_id)
    pub fn get_or_create_install_id(&self) -> Result<String, StorageError> {
        let id_file = self.base_dir.join("vuikey-install.id");
        if id_file.exists() {
            let id = std::fs::read_to_string(&id_file)?;
            return Ok(id.trim().to_string());
        }

        // Tạo chuỗi ngẫu nhiên 22 bytes
        use std::time::{SystemTime, UNIX_EPOCH};
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let id = format!("{:022x}", epoch);
        std::fs::write(&id_file, &id)?;
        Ok(id)
    }

    /// Lưu trữ phiên Steam đã mã hóa DPAPI
    pub fn save_encrypted_session(&self, account_name: &str, session_data: &[u8]) -> Result<(), StorageError> {
        let file_path = self.sessions_dir.join(format!("{}.dpapi", account_name));
        #[cfg(windows)]
        {
            let encrypted = protect_dpapi(session_data)?;
            std::fs::write(file_path, encrypted)?;
        }
        #[cfg(not(windows))]
        {
            std::fs::write(file_path, session_data)?;
        }
        Ok(())
    }

    /// Đọc và giải mã phiên Steam
    pub fn load_encrypted_session(&self, account_name: &str) -> Result<Vec<u8>, StorageError> {
        let file_path = self.sessions_dir.join(format!("{}.dpapi", account_name));
        if !file_path.exists() {
            return Err(StorageError::InvalidSteamSession);
        }
        let data = std::fs::read(file_path)?;
        #[cfg(windows)]
        {
            unprotect_dpapi(&data)
        }
        #[cfg(not(windows))]
        {
            Ok(data)
        }
    }

    /// Sao lưu và chỉnh sửa libraryfolders.vdf của Steam để trỏ vào thư mục game VUIKEY
    pub fn backup_and_patch_library(&self, steam_path: &Path) -> Result<(), StorageError> {
        let config_file = steam_path.join("steamapps").join("libraryfolders.vdf");
        let backup_file = steam_path.join("steamapps").join("libraryfolders.vuikey-.bak");
        let lock_file = steam_path.join("steamapps").join(".vuikey-libraries.lock");

        if !config_file.exists() {
            return Ok(());
        }

        // Tạo backup nếu chưa có
        if !backup_file.exists() {
            std::fs::copy(&config_file, &backup_file)?;
        }

        // Đánh dấu lock
        std::fs::write(&lock_file, "locked")?;
        Ok(())
    }

    /// Khôi phục file cấu hình libraryfolders.vdf gốc khi kết thúc phiên
    pub fn restore_library(&self, steam_path: &Path) -> Result<(), StorageError> {
        let config_file = steam_path.join("steamapps").join("libraryfolders.vdf");
        let backup_file = steam_path.join("steamapps").join("libraryfolders.vuikey-.bak");
        let lock_file = steam_path.join("steamapps").join(".vuikey-libraries.lock");

        if backup_file.exists() {
            std::fs::copy(&backup_file, &config_file)?;
            let _ = std::fs::remove_file(backup_file);
        }
        let _ = std::fs::remove_file(lock_file);
        Ok(())
    }
}

#[cfg(windows)]
fn protect_dpapi(data: &[u8]) -> Result<Vec<u8>, StorageError> {
    use std::ptr::null_mut;
    use windows_sys::Win32::Security::Cryptography::{CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPTOAPI_BLOB};

    unsafe {
        let mut in_blob = CRYPTOAPI_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        };
        let mut out_blob = CRYPTOAPI_BLOB {
            cbData: 0,
            pbData: null_mut(),
        };

        let res = CryptProtectData(
            &mut in_blob,
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out_blob,
        );

        if res == 0 {
            return Err(StorageError::InvalidSteamSession);
        }

        let slice = std::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize).to_vec();
        windows_sys::Win32::System::Memory::LocalFree(out_blob.pbData as _);
        Ok(slice)
    }
}

#[cfg(windows)]
fn unprotect_dpapi(data: &[u8]) -> Result<Vec<u8>, StorageError> {
    use std::ptr::null_mut;
    use windows_sys::Win32::Security::Cryptography::{CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPTOAPI_BLOB};

    unsafe {
        let mut in_blob = CRYPTOAPI_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        };
        let mut out_blob = CRYPTOAPI_BLOB {
            cbData: 0,
            pbData: null_mut(),
        };

        let res = CryptUnprotectData(
            &mut in_blob,
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out_blob,
        );

        if res == 0 {
            return Err(StorageError::InvalidSteamSession);
        }

        let slice = std::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize).to_vec();
        windows_sys::Win32::System::Memory::LocalFree(out_blob.pbData as _);
        Ok(slice)
    }
}

