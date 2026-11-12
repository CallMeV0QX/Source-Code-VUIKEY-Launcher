//! main.rs: Điểm vào chính của VUIKEY Launcher
//! Kết hợp giao diện người dùng WebView (Tao + Wry) với các module can thiệp:
//! Steam CEF Injection, VDF Manipulator, Native Guard Helper, Game File Patcher.

mod cef;
mod game_files;
mod native_guard;
mod steam;
mod steam_first_launch;
mod storage;
mod updater;
mod vdf;

use cef::{CefManager, SteamPolicy};
use clap::Parser;
use game_files::GamePatcher;
use native_guard::{NativeGuardController, Request as GuardRequest};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use steam::SteamManager;
use storage::StorageManager;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop};
use tao::window::WindowBuilder;
use wry::WebViewBuilder;

const LAUNCHER_VERSION: &str = "1.0.0";
const BRIDGE_JS: &str = include_str!("../../../extracted_scripts/webview_bridge.js");

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[arg(long)]
    steam_guard_helper: bool,

    #[arg(long)]
    apply_update: bool,

    #[arg(long)]
    pid: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LauncherConfig {
    pub url: String,
    pub title: String,
    #[serde(default = "default_true")]
    pub steam_ui: bool,
    pub steam_exe: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeRequest {
    pub id: String,
    pub action: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeResponse {
    pub id: String,
    pub ok: bool,
    pub data: Option<serde_json::Value>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    pub launcher_version: String,
    pub product_access_version: String,
    pub steam_installed: bool,
    pub steam_running: bool,
    pub busy: bool,
    pub last_error: Option<String>,
    pub attached: bool,
    pub policy_applied: bool,
    pub connection_phase: String,
    pub connection_error: Option<String>,
    pub processed_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchLease {
    pub login_name: String,
    pub password: Option<String>,
    pub lease_token: String,
    pub expires_at: u64,
    pub server_now: u64,
    pub steam_policy: SteamPolicy,
    pub guard_enabled: bool,
    pub renewable: bool,
    pub revoke_session: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // 1. Chế độ tiến trình phụ Steam Guard Helper
    if cli.steam_guard_helper {
        native_guard::run_helper_loop()?;
        return Ok(());
    }

    // 2. Tải file cấu hình launcher.json
    let config_path = Path::new("launcher.json");
    let launcher_config: LauncherConfig = if config_path.exists() {
        let content = std::fs::read_to_string(config_path)?;
        serde_json::from_str(&content)?
    } else {
        LauncherConfig {
            url: "https://api.vuikey.com".into(),
            title: "VUIKEY — Play your next story".into(),
            steam_ui: true,
            steam_exe: None,
        }
    };

    println!("[*] Starting {}", launcher_config.title);

    let storage_mgr = Arc::new(StorageManager::new()?);
    let install_id = storage_mgr.get_or_create_install_id()?;
    println!("[*] Vuikey Install ID: {}", install_id);

    // 3. Khởi tạo Tao Window & EventLoop
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title(&launcher_config.title)
        .with_inner_size(tao::dpi::LogicalSize::new(1280.0, 720.0))
        .build(&event_loop)?;

    // 4. Khởi tạo Wry WebView và gắn IPC Handler
    let storage_clone = storage_mgr.clone();
    let config_clone = launcher_config.clone();

    let webview = WebViewBuilder::new(&window)
        .with_initialization_script(BRIDGE_JS)
        .with_url(&launcher_config.url)
        .with_ipc_handler(move |window, msg| {
            let req: BridgeRequest = match serde_json::from_str(&msg) {
                Ok(r) => r,
                Err(_) => return,
            };

            let req_id = req.id.clone();
            let action = req.action.clone();
            let payload = req.payload.clone();
            let cfg = config_clone.clone();
            let storage = storage_clone.clone();

            tokio::spawn(async move {
                let res = handle_ipc_action(&action, payload, &cfg, &storage).await;
                let response = match res {
                    Ok(val) => BridgeResponse {
                        id: req_id,
                        ok: true,
                        data: Some(val),
                        error: None,
                    },
                    Err(err) => BridgeResponse {
                        id: req_id,
                        ok: false,
                        data: None,
                        error: Some(err),
                    },
                };

                let js_cb = format!(
                    "window.__vuikeyReceive({});",
                    serde_json::to_string(&response).unwrap_or_default()
                );
                // Gửi phản hồi lại WebView
                println!("[IPC Response] {}", js_cb);
            });
        })
        .build()?;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                println!("[*] Window close requested. Ending session...");
                *control_flow = ControlFlow::Exit;
            }
            _ => (),
        }
    });

    Ok(())
}

/// Xử lý các lệnh IPC từ giao diện web của VUIKEY
async fn handle_ipc_action(
    action: &str,
    payload: serde_json::Value,
    config: &LauncherConfig,
    storage: &StorageManager,
) -> Result<serde_json::Value, String> {
    match action {
        "status" => {
            let steam_found = SteamManager::locate(config.steam_exe.as_deref()).is_ok();
            let st = Status {
                launcher_version: LAUNCHER_VERSION.into(),
                product_access_version: "1.0.0".into(),
                steam_installed: steam_found,
                steam_running: false,
                busy: false,
                last_error: None,
                attached: false,
                policy_applied: false,
                connection_phase: "ready".into(),
                connection_error: None,
                processed_bytes: 0,
            };
            Ok(serde_json::to_value(st).map_err(|e| e.to_string())?)
        }
        "launch" => {
            let app_id = payload
                .get("app_id")
                .and_then(|v| v.as_u64())
                .ok_or("Thiếu app_id hợp lệ")? as u32;

            println!("[*] Yêu cầu mở game app_id: {}", app_id);

            // 1. Gọi API /api/launcher/redeem để lấy tài khoản và mật khẩu
            let client = reqwest::Client::new();
            let redeem_url = format!("{}/api/launcher/redeem", config.url);
            let redeem_resp = client
                .post(&redeem_url)
                .json(&serde_json::json!({
                    "app_id": app_id,
                    "install_id": storage.get_or_create_install_id().map_err(|e| e.to_string())?
                }))
                .send()
                .await
                .map_err(|e| format!("Không kết nối được Cloudflare Worker: {}", e))?;

            let lease: LaunchLease = redeem_resp
                .json()
                .await
                .map_err(|e| format!("Worker không trả về JSON hợp lệ: {}", e))?;

            println!(
                "[*] Nhận được tài khoản được cấp: {} (Hết hạn: {})",
                lease.login_name, lease.expires_at
            );

            // 2. Tìm kiếm và cấu hình Steam
            let steam_mgr = SteamManager::locate(config.steam_exe.as_deref())
                .map_err(|e| e.to_string())?;

            let loginusers_path = steam_mgr.steam_dir.join("config").join("loginusers.vdf");
            if loginusers_path.exists() {
                let _ = vdf::patch_loginusers(
                    &loginusers_path,
                    &lease.login_name,
                    lease.steam_policy.connection_mode != "online",
                );
            }

            // 3. Khởi động Steam kèm cờ CEF Debugging
            let steam_child = steam_mgr
                .launch(&lease.login_name, config.steam_ui, false, None)
                .map_err(|e| e.to_string())?;

            let steam_pid = steam_child.id();

            // 4. Nếu có Steam Guard, khởi tạo Native Guard Helper
            if lease.guard_enabled {
                println!("[*] Tài khoản yêu cầu Steam Guard 2FA, khởi tạo helper...");
                let current_exe = std::env::current_exe().map_err(|e| e.to_string())?;
                let mut guard_ctrl = NativeGuardController::spawn_helper(
                    &current_exe,
                    &steam_mgr.steam_dir,
                    steam_pid,
                    &lease.login_name,
                )
                .map_err(|e| e.to_string())?;

                // Thăm dò mã Guard từ server
                let guard_url = format!("{}/api/launcher/guard-code", config.url);
                if let Ok(guard_resp) = client
                    .get(&guard_url)
                    .query(&[("lease_token", &lease.lease_token)])
                    .send()
                    .await
                {
                    if let Ok(guard_data) = guard_resp.json::<serde_json::Value>().await {
                        if let Some(code) = guard_data.get("code").and_then(|v| v.as_str()) {
                            println!("[*] Tự động gửi mã Guard: {}", code);
                            let _ = guard_ctrl.send_code(code);
                        }
                    }
                }
            }

            // 5. Kết nối CEF và tiêm các kịch bản kiểm soát
            if config.steam_ui {
                println!("[*] Đang kết nối Steam CEF DevTools...");
                tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
                let cef_mgr = CefManager::new(8080, lease.steam_policy.clone());
                if let Err(e) = cef_mgr.apply_policies().await {
                    eprintln!("[!] Cảnh báo CEF injection: {}", e);
                }
            }

            // 6. Nếu là RDR2 (App ID 1174180), áp dụng patch bẻ khóa Launcher
            if app_id == 1174180 {
                let game_patcher = GamePatcher::new(
                    std::env::current_dir().unwrap_or(PathBuf::from(".")),
                );
                let game_path = steam_mgr
                    .steam_dir
                    .join("steamapps")
                    .join("common")
                    .join("Red Dead Redemption 2");
                if game_path.exists() {
                    let _ = game_patcher.apply_rdr2_post_install(&game_path);
                }
            }

            Ok(serde_json::json!({
                "status": "launched",
                "app_id": app_id,
                "account": lease.login_name,
                "expires_at": lease.expires_at
            }))
        }
        "finish" => {
            println!("[*] Kết thúc phiên chơi.");
            Ok(serde_json::json!({ "status": "finished" }))
        }
        "open_google_login" => {
            let auth_url = format!("{}/api/auth/google/open", config.url);
            steam_first_launch::DownloadWatcher::open_google_auth_url(&auth_url)
                .map_err(|e| e.to_string())?;
            Ok(serde_json::json!({ "opened": true }))
        }
        _ => Err(format!("Lệnh không được hỗ trợ: {}", action)),
    }
}

