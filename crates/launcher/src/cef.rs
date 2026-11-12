//! cef.rs: Can thiệp và tiêm JavaScript vào Steam thông qua Chromium Embedded Framework (CEF)
//! Sử dụng cờ `-cef-enable-debugging` để mở cổng Chrome DevTools (mặc định 127.0.0.1:8080).
//! Kết nối qua WebSocket (Tungstenite) và thực thi 4 payload JS:
//! 1. installVuikeySteam: Chặn menu tài khoản, đăng xuất, đổi mật khẩu, danh sách bạn bè.
//! 2. installVuikeyLibrary: Ẩn tất cả game khác trong kho game MobX, chỉ hiện game thuê.
//! 3. restrictVuikeyStore: Chặn truy cập Steam Store, thanh toán và giỏ hàng.
//! 4. manageVuikeyConnection: Tự động gọi SteamClient.User.GoOffline() khi game tải xong.

use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;
use tungstenite::{connect, Message};
use url::Url;

#[derive(Error, Debug)]
pub enum CefError {
    #[error("Steam đã mở nhưng chưa gắn được CEF: {0}")]
    AttachFailed(String),
    #[error("CEF chưa sẵn sàng")]
    NotReady,
    #[error("Không kết nối được cổng Chrome DevTools của Steam")]
    ConnectionFailed,
    #[error("Lỗi WebSocket: {0}")]
    WsError(#[from] tungstenite::Error),
    #[error("Lỗi HTTP/Reqwest: {0}")]
    HttpError(#[from] reqwest::Error),
    #[error("Lỗi JSON: {0}")]
    JsonError(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Target {
    pub id: String,
    pub title: String,
    #[serde(rename = "type")]
    pub target_type: String,
    #[serde(rename = "webSocketDebuggerUrl")]
    pub ws_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SteamPolicy {
    pub app_id: u32,
    pub hide_account_settings: bool,
    pub disable_remote_play: bool,
    pub restrict_game_launch: bool,
    pub allow_friends: bool,
    pub connection_mode: String, // "online", "offline_after_download", "offline_after_first_launch"
}

pub const STEAM_JS_PAYLOAD: &str = include_str!("../../../extracted_scripts/installVuikeySteam.js");
pub const LIBRARY_JS_PAYLOAD: &str = include_str!("../../../extracted_scripts/installVuikeyLibrary.js");
pub const STORE_JS_PAYLOAD: &str = include_str!("../../../extracted_scripts/restrictVuikeyStore.js");
pub const CONNECTION_JS_PAYLOAD: &str = include_str!("../../../extracted_scripts/manageVuikeyConnection.js");

pub struct CefManager {
    port: u16,
    policy: SteamPolicy,
}

impl CefManager {
    pub fn new(port: u16, policy: SteamPolicy) -> Self {
        Self { port, policy }
    }

    /// Lấy danh sách các targets (cửa sổ web) từ Steam CEF DevTools
    pub async fn get_targets(&self) -> Result<Vec<Target>, CefError> {
        let url = format!("http://127.0.0.1:{}/json", self.port);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(1500))
            .build()?;

        let resp = client.get(&url).send().await?;
        let targets: Vec<Target> = resp.json().await?;
        Ok(targets)
    }

    /// Tiêm mã JavaScript vào target cụ thể qua WebSocket DevTools Protocol
    pub fn inject_script_into_target(&self, ws_url: &str, script: &str) -> Result<(), CefError> {
        let (mut socket, _) = connect(Url::parse(ws_url).map_err(|_| CefError::ConnectionFailed)?)?;

        let evaluate_cmd = serde_json::json!({
            "id": 1,
            "method": "Runtime.evaluate",
            "params": {
                "expression": script,
                "awaitPromise": true,
                "returnByValue": true
            }
        });

        socket.send(Message::Text(evaluate_cmd.to_string()))?;

        // Đợi phản hồi hoàn tất
        let _ = socket.read()?;
        let _ = socket.close(None);
        Ok(())
    }

    /// Tiêm toàn bộ chính sách và kịch bản can thiệp vào Steam
    pub async fn apply_policies(&self) -> Result<(), CefError> {
        let targets = self.get_targets().await?;
        if targets.is_empty() {
            return Err(CefError::NotReady);
        }

        let policy_json = serde_json::json!({
            "version": 1,
            "app_id": self.policy.app_id,
            "enabled": true,
            "hide_account_settings": self.policy.hide_account_settings,
            "disable_remote_play": self.policy.disable_remote_play,
            "restrict_game_launch": self.policy.restrict_game_launch,
            "allow_friends": self.policy.allow_friends,
            "mode": self.policy.connection_mode,
            "native_account": "matched",
            "expected_account": "vuikey_shared"
        });

        let config_str = policy_json.to_string();

        for target in &targets {
            if let Some(ref ws) = target.ws_url {
                // 1. Tiêm Steam general restrictor
                let script_steam = format!("({})({});", STEAM_JS_PAYLOAD, config_str);
                let _ = self.inject_script_into_target(ws, &script_steam);

                // 2. Tiêm Store restrictor
                let script_store = format!("({})({});", STORE_JS_PAYLOAD, config_str);
                let _ = self.inject_script_into_target(ws, &script_store);

                // Nếu là SharedJSContext thì tiêm thêm Library và Connection Manager
                if target.title == "SharedJSContext" {
                    let script_lib = format!("({})({});", LIBRARY_JS_PAYLOAD, config_str);
                    let _ = self.inject_script_into_target(ws, &script_lib);

                    let script_conn = format!("({})({});", CONNECTION_JS_PAYLOAD, config_str);
                    let _ = self.inject_script_into_target(ws, &script_conn);
                }
            }
        }

        Ok(())
    }

    /// Hủy bỏ toàn bộ can thiệp trước khi đóng Steam
    pub async fn dispose_policies(&self) -> Result<(), CefError> {
        let targets = self.get_targets().await?;
        let cleanup_script = "(()=>{window.__vuikeyStoreV1?.dispose();window.__vuikeyLibraryV1?.dispose();window.__vuikeyConnectionV1?.dispose();return window.__vuikeySteamV1?.dispose();})()";

        for target in &targets {
            if let Some(ref ws) = target.ws_url {
                let _ = self.inject_script_into_target(ws, cleanup_script);
            }
        }
        Ok(())
    }
}

