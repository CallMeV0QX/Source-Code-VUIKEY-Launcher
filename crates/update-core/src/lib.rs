//! update-core: Crate quản lý tải và xác minh cập nhật của VUIKEY
//! Tự động lấy manifest từ https://api.vuikey.com/api/installer/latest
//! và kiểm tra chữ ký Ed25519 trước khi thực thi file cập nhật.

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use thiserror::Error;

pub const DEFAULT_UPDATE_URL: &str = "https://api.vuikey.com/api/installer/latest";
pub const PUBLIC_VERIFY_KEY_HEX: &str =
    "408046a190d76594e2b890b6cd6cce27ebaeffaa51b1de632f37623ba8d3f36d";

#[derive(Error, Debug)]
pub enum UpdateError {
    #[error("Không kết nối được máy chủ VUIKEY. Kiểm tra mạng rồi thử lại.")]
    NetworkError(#[from] reqwest::Error),
    #[error("Kết nối tải bị gián đoạn. Chạy lại setup để thử lại.")]
    ConnectionInterrupted,
    #[error("Máy chủ trả về bản cài đang bận. Thử lại sau.")]
    ServerBusy,
    #[error("Máy chủ trả về dung lượng bản cài không khớp.")]
    ServerLengthMismatch,
    #[error("Dung lượng bản cài không khớp. Không chạy tệp đã tải.")]
    DownloadedLengthMismatch,
    #[error("Bản cài bị thiếu hoặc sai mã SHA-256. Không chạy tệp đã tải.")]
    ChecksumMismatch,
    #[error("Đã hủy tải bản cài.")]
    Cancelled,
    #[error("Không tải được bản cài. Kiểm tra mạng rồi thử lại.")]
    DownloadFailed,
    #[error("Setup thiếu khóa xác minh.")]
    MissingVerifyKey,
    #[error("Khóa xác minh không hợp lệ.")]
    InvalidVerifyKey,
    #[error("Chữ ký bản cập nhật không hợp lệ hoặc bị giả mạo.")]
    InvalidSignature,
    #[error("Lỗi I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("Lỗi JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Release {
    pub schema: u32,
    pub channel: String,
    pub platform: String,
    pub min_bootstrap: String,
    pub version: String,
    pub published_at: String,
    pub url: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub payload: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reply {
    pub ok: bool,
    pub channel: Option<String>,
    pub version: Option<String>,
    pub url: Option<String>,
    pub error: Option<String>,
    pub release: Option<Release>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub url: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Code {
    pub status: u32,
    pub message: String,
    pub valid_until: Option<u64>,
}

pub struct UpdateChecker {
    pub api_url: String,
    pub verify_key: [u8; 32],
}

impl Default for UpdateChecker {
    fn default() -> Self {
        let mut key = [0u8; 32];
        if let Ok(bytes) = hex::decode(PUBLIC_VERIFY_KEY_HEX) {
            if bytes.len() == 32 {
                key.copy_from_slice(&bytes);
            }
        }
        Self {
            api_url: DEFAULT_UPDATE_URL.to_string(),
            verify_key: key,
        }
    }
}

impl UpdateChecker {
    pub fn new(api_url: Option<String>, verify_key_hex: Option<&str>) -> Result<Self, UpdateError> {
        let mut key = [0u8; 32];
        let key_str = verify_key_hex.unwrap_or(PUBLIC_VERIFY_KEY_HEX);
        let bytes = hex::decode(key_str).map_err(|_| UpdateError::InvalidVerifyKey)?;
        if bytes.len() != 32 {
            return Err(UpdateError::InvalidVerifyKey);
        }
        key.copy_from_slice(&bytes);

        Ok(Self {
            api_url: api_url.unwrap_or_else(|| DEFAULT_UPDATE_URL.to_string()),
            verify_key: key,
        })
    }

    /// Lấy và xác thực bản cập nhật mới nhất từ máy chủ
    pub async fn check_latest(&self) -> Result<Release, UpdateError> {
        let client = reqwest::Client::builder()
            .user_agent("VUIKEY-Setup/1.0.0")
            .build()?;

        let resp = client.get(&self.api_url).send().await?;
        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
            || resp.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE
        {
            return Err(UpdateError::ServerBusy);
        }

        let envelope: Envelope = resp.json().await?;
        self.verify_payload(&envelope.payload, &envelope.signature)?;

        let release: Release = serde_json::from_str(&envelope.payload)?;
        Ok(release)
    }

    /// Kiểm tra chữ ký Ed25519 của payload JSON
    pub fn verify_payload(&self, payload: &str, signature_hex: &str) -> Result<(), UpdateError> {
        let verifying_key = VerifyingKey::from_bytes(&self.verify_key)
            .map_err(|_| UpdateError::InvalidVerifyKey)?;
        let sig_bytes = hex::decode(signature_hex).map_err(|_| UpdateError::InvalidSignature)?;
        if sig_bytes.len() != 64 {
            return Err(UpdateError::InvalidSignature);
        }
        let mut sig_arr = [0u8; 64];
        sig_arr.copy_from_slice(&sig_bytes);
        let signature = Signature::from_bytes(&sig_arr);

        verifying_key
            .verify(payload.as_bytes(), &signature)
            .map_err(|_| UpdateError::InvalidSignature)?;

        Ok(())
    }

    /// Tải file cập nhật và kiểm tra checksum SHA-256
    pub async fn download_and_verify(
        &self,
        release: &Release,
        dest_path: &Path,
    ) -> Result<(), UpdateError> {
        let client = reqwest::Client::new();
        let mut resp = client.get(&release.url).send().await?;

        if let Some(content_len) = resp.content_length() {
            if content_len != release.bytes {
                return Err(UpdateError::ServerLengthMismatch);
            }
        }

        let mut file = tokio::fs::File::create(dest_path).await?;
        let mut hasher = Sha256::new();
        let mut downloaded_bytes: u64 = 0;

        use tokio::io::AsyncWriteExt;
        while let Some(chunk) = resp.chunk().await.map_err(|_| UpdateError::ConnectionInterrupted)? {
            hasher.update(&chunk);
            downloaded_bytes += chunk.len() as u64;
            file.write_all(&chunk).await?;
        }
        file.flush().await?;

        if downloaded_bytes != release.bytes {
            let _ = tokio::fs::remove_file(dest_path).await;
            return Err(UpdateError::DownloadedLengthMismatch);
        }

        let calculated_hash = format!("{:x}", hasher.finalize());
        if calculated_hash.to_lowercase() != release.sha256.to_lowercase() {
            let _ = tokio::fs::remove_file(dest_path).await;
            return Err(UpdateError::ChecksumMismatch);
        }

        Ok(())
    }
}

