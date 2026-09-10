//! Local configuration for RATSA-Harness.
//!
//! Everything lives in a single JSON file so an Agent (or a human) can inspect
//! and edit it:
//!
//! ```json
//! {
//!   "base_url": "https://ratsa.ai",
//!   "key_id": "rtsk_...",
//!   "secret": "rtsk_s_...",
//!   "scopes": ["sof:read", "package:pull"],
//!   "bound_slug": "demo",
//!   "acting_as": "demo",
//!   "session_token": "<JWT, only while `login` is active>",
//!   "user": { "id": 2, "name": "演示厂商", "slug": "demo", "plan": "pro" }
//! }
//! ```
//!
//! The file is written with `0600` permissions on Unix because it holds a
//! secret. The JWT session is kept separately from the API key on purpose: the
//! session is what mints/rotates keys, the key is what Agents carry.

use std::env;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const DEFAULT_BASE_URL: &str = "https://ratsa.ai";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserInfo {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub plan: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_base_url")]
    pub base_url: String,
    /// API key id (`X-API-Key-Id`). Empty when the CLI has a session only.
    #[serde(default)]
    pub key_id: String,
    /// API key secret (`X-API-Key`).
    #[serde(default)]
    pub secret: String,
    /// Permission table of the active key (informational, refreshed by whoami).
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Account handle this key acts as, when the key is bound.
    #[serde(default)]
    pub bound_slug: String,
    /// Resolved handle reported by the server (`acting_as`).
    #[serde(default)]
    pub acting_as: String,
    #[serde(default)]
    pub key_name: String,
    /// JWT session used to manage keys / publish. Not needed for pulls.
    #[serde(default)]
    pub session_token: String,
    #[serde(default)]
    pub user: Option<UserInfo>,
}

fn default_base_url() -> String {
    DEFAULT_BASE_URL.to_string()
}

impl Default for Config {
    fn default() -> Self {
        Config {
            base_url: default_base_url(),
            key_id: String::new(),
            secret: String::new(),
            scopes: Vec::new(),
            bound_slug: String::new(),
            acting_as: String::new(),
            key_name: String::new(),
            session_token: String::new(),
            user: None,
        }
    }
}

/// `~/.ratsa` (override with `RATSA_HOME`).
pub fn home_dir() -> PathBuf {
    if let Ok(dir) = env::var("RATSA_HOME") {
        return PathBuf::from(dir);
    }
    if let Ok(dir) = env::var("HOME") {
        return PathBuf::from(dir).join(".ratsa");
    }
    PathBuf::from(".ratsa")
}

pub fn config_path() -> PathBuf {
    if let Ok(p) = env::var("RATSA_CONFIG") {
        return PathBuf::from(p);
    }
    home_dir().join("config.json")
}

impl Config {
    pub fn load() -> Config {
        match fs::read_to_string(config_path()) {
            Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
            Err(_) => Config::default(),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("无法创建 {}: {e}", parent.display()))?;
        }
        let raw = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let mut file = fs::File::create(&path).map_err(|e| format!("无法写入 {}: {e}", path.display()))?;
        file.write_all(raw.as_bytes()).map_err(|e| e.to_string())?;
        file.write_all(b"\n").map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    /// The key the CLI should call the API with, honouring env overrides so CI
    /// and Agents can run without a config file.
    pub fn credentials(&self) -> (String, String) {
        let key_id = env::var("RATSA_KEY_ID").unwrap_or_else(|_| self.key_id.clone());
        let secret = env::var("RATSA_KEY_SECRET").unwrap_or_else(|_| self.secret.clone());
        (key_id, secret)
    }

    pub fn base(&self) -> String {
        let raw = env::var("RATSA_BASE_URL").unwrap_or_else(|_| self.base_url.clone());
        raw.trim_end_matches('/').to_string()
    }

    pub fn has_key(&self) -> bool {
        let (id, secret) = self.credentials();
        !id.is_empty() && !secret.is_empty()
    }
}
