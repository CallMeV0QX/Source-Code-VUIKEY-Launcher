//! steam.rs: Quản lý phát hiện, cấu hình và khởi chạy tiến trình Steam
//! Tìm kiếm đường dẫn cài đặt Steam từ Windows Registry (`Software\Valve\Steam`),
//! cấu hình tự động đăng nhập vào tài khoản được thuê, và khởi chạy với `-cef-enable-debugging`.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SteamError {
    #[error("Chưa tìm thấy Steam. Cài Steam hoặc chỉ định steam_exe trong launcher.json.")]
    NotFound,
    #[error("Thiếu thư mục Steam")]
    MissingSteamDir,
    #[error("Không thể ghi Steam config; kiểm tra quyền truy cập")]
    ConfigWriteFailed,
    #[error("Mật khẩu Steam không hợp lệ")]
    InvalidPassword,
    #[error("Trò chơi này không hỗ trợ lựa chọn mở offline")]
    OfflineNotSupported,
    #[error("Steam chưa ghi nhớ tài khoản trên máy này. Bỏ tick và mở online một lần trước.")]
    AccountNotRemembered,
    #[error("Game chưa tải xong trên máy này. Bỏ tick mở online trước.")]
    GameNotReady,
    #[error("Cổng 127.0.0.1:8080 đang dùng. Đóng app dùng cổng này hoặc đặt steam_ui=false.")]
    PortInUse,
    #[error("Không thể khởi động Steam")]
    LaunchFailed,
    #[error("Không thể khôi phục config khi Steam đang chạy")]
    SteamRunning,
    #[error("Hãy thoát game/Steam đang chạy thử lại; launcher không cưỡng bức tắt game.")]
    ActiveProcessConflict,
    #[error("Lỗi I/O: {0}")]
    Io(#[from] std::io::Error),
}

pub struct SteamManager {
    pub steam_dir: PathBuf,
    pub steam_exe: PathBuf,
}

impl SteamManager {
    /// Tìm vị trí cài đặt của Steam từ Windows Registry hoặc đường dẫn mặc định
    pub fn locate(custom_exe: Option<&str>) -> Result<Self, SteamError> {
        if let Some(exe_str) = custom_exe {
            let p = PathBuf::from(exe_str);
            if p.exists() {
                let dir = p.parent().unwrap_or(Path::new("")).to_path_buf();
                return Ok(Self {
                    steam_dir: dir,
                    steam_exe: p,
                });
            }
        }

        #[cfg(windows)]
        {
            if let Ok(dir_str) = read_registry_string(
                windows_sys::Win32::System::Registry::HKEY_CURRENT_USER,
                "Software\\Valve\\Steam",
                "SteamPath",
            ) {
                let dir = PathBuf::from(dir_str);
                let exe = dir.join("steam.exe");
                if exe.exists() {
                    return Ok(Self {
                        steam_dir: dir,
                        steam_exe: exe,
                    });
                }
            }
        }

        // Kiểm tra Program Files mặc định
        let default_paths = [
            r"C:\Program Files (x86)\Steam\steam.exe",
            r"C:\Program Files\Steam\steam.exe",
        ];

        for p_str in default_paths {
            let p = PathBuf::from(p_str);
            if p.exists() {
                return Ok(Self {
                    steam_dir: p.parent().unwrap().to_path_buf(),
                    steam_exe: p,
                });
            }
        }

        Err(SteamError::NotFound)
    }

    /// Khởi chạy tiến trình Steam với cờ can thiệp CEF Debugging
    pub fn launch(
        &self,
        account_name: &str,
        enable_cef_debug: bool,
        force_offline: bool,
        override_package_url: Option<&str>,
    ) -> Result<Child, SteamError> {
        let mut cmd = Command::new(&self.steam_exe);

        // Bỏ qua driver streaming để tránh xung đột
        cmd.arg("-skipstreamingdrivers");

        if enable_cef_debug {
            cmd.arg("-cef-enable-debugging");
        }

        if force_offline {
            cmd.arg("-offline");
        }

        // Tự động đăng nhập vào tài khoản được thuê
        if !account_name.is_empty() {
            cmd.arg("-login").arg(account_name);
        }

        // Nếu có mirror local loopback updater
        if let Some(url) = override_package_url {
            cmd.arg("-overridepackageurl").arg(url);
        }

        let child = cmd.spawn().map_err(|_| SteamError::LaunchFailed)?;
        Ok(child)
    }

    /// Kiểm tra xem cổng 8080 có đang bị chiếm dụng không
    pub fn check_debug_port_free() -> bool {
        std::net::TcpListener::bind("127.0.0.1:8080").is_ok()
    }
}

#[cfg(windows)]
fn read_registry_string(
    hkey: windows_sys::Win32::System::Registry::HKEY,
    sub_key: &str,
    value_name: &str,
) -> Result<String, ()> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::Registry::{RegCloseKey, RegOpenKeyExW, RegQueryValueExW, KEY_READ};

    let sub_key_w: Vec<u16> = OsStr::new(sub_key)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let value_name_w: Vec<u16> = OsStr::new(value_name)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut key_handle = 0;
    unsafe {
        let status = RegOpenKeyExW(hkey, sub_key_w.as_ptr(), 0, KEY_READ, &mut key_handle);
        if status != 0 {
            return Err(());
        }

        let mut buf = [0u16; 512];
        let mut buf_len = (buf.len() * 2) as u32;
        let mut val_type = 0;

        let query_status = RegQueryValueExW(
            key_handle,
            value_name_w.as_ptr(),
            std::ptr::null_mut(),
            &mut val_type,
            buf.as_mut_ptr() as _,
            &mut buf_len,
        );

        RegCloseKey(key_handle);

        if query_status != 0 {
            return Err(());
        }

        let len = (buf_len / 2) as usize;
        let slice = if len > 0 && buf[len - 1] == 0 {
            &buf[..len - 1]
        } else {
            &buf[..len]
        };

        String::from_utf16(slice).map_err(|_| ())
    }
}

