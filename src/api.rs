//! Thin HTTP client for the RATSA.ai platform.
//!
//! Two credential flavours are supported, mirroring the server:
//!
//! * **API key** (`X-API-Key-Id` / `X-API-Key`) — what an Agent carries. Scoped
//!   by the permission table, optionally bound to an account handle.
//! * **JWT session** (`Authorization: Bearer …`) — what a human uses to create
//!   keys, list them and publish.
//!
//! The CLI never talks to any RATSA-internal service: it only calls the public
//! HTTP API of the configured `base_url`, so a customer can point it at a
//! self-hosted deployment.

use std::io::Read;

use serde_json::Value;

use crate::config::Config;

pub struct Client {
    pub base: String,
    pub key_id: String,
    pub secret: String,
    pub session: String,
    agent: ureq::Agent,
}

pub struct Response {
    pub status: u16,
    pub body: String,
}

impl Response {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }

    /// Human-facing error message: the platform returns `{"error": "…"}` plus a
    /// machine `code` for the interesting cases (missing_scope, login_required…).
    pub fn error_message(&self) -> String {
        let v = self.json();
        let msg = v
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("");
        let code = v.get("code").and_then(|e| e.as_str()).unwrap_or("");
        match (msg.is_empty(), code.is_empty()) {
            (false, false) => format!("{msg}（{code}）"),
            (false, true) => msg.to_string(),
            (true, false) => code.to_string(),
            (true, true) => truncate(&self.body, 300),
        }
    }
}

pub fn truncate(s: &str, n: usize) -> String {
    let t = s.trim();
    if t.chars().count() <= n {
        return t.to_string();
    }
    let head: String = t.chars().take(n).collect();
    format!("{head}…")
}

impl Client {
    pub fn new(cfg: &Config) -> Client {
        let (key_id, secret) = cfg.credentials();
        Client {
            base: cfg.base(),
            key_id,
            secret,
            session: cfg.session_token.clone(),
            agent: ureq::AgentBuilder::new()
                .timeout_connect(std::time::Duration::from_secs(15))
                .timeout_read(std::time::Duration::from_secs(120))
                .user_agent(concat!("ratsa-harness/", env!("CARGO_PKG_VERSION")))
                .build(),
        }
    }

    fn decorate(&self, mut req: ureq::Request) -> ureq::Request {
        if !self.key_id.is_empty() && !self.secret.is_empty() {
            req = req
                .set("X-API-Key-Id", &self.key_id)
                .set("X-API-Key", &self.secret);
        }
        if !self.session.is_empty() {
            req = req.set("Authorization", &format!("Bearer {}", self.session));
        }
        req
    }

    fn send(&self, req: ureq::Request, body: Option<&Value>) -> Result<Response, String> {
        let req = self.decorate(req);
        let result = match body {
            Some(v) => req.send_json(v.clone()),
            None => req.call(),
        };
        match result {
            Ok(resp) => read(resp),
            Err(ureq::Error::Status(_, resp)) => read(resp),
            Err(ureq::Error::Transport(t)) => Err(format!("网络错误：{t}（base_url={}）", self.base)),
        }
    }

    pub fn get(&self, path: &str) -> Result<Response, String> {
        let url = format!("{}{}", self.base, path);
        self.send(self.agent.get(&url), None)
    }

    /// GET a path and save the body to disk, returning the written path.
    /// Returns `(path, bytes, headers)` so callers can report size/checksum.
    pub fn download_to(
        &self,
        path: &str,
        dir: &std::path::Path,
        default_name: &str,
    ) -> Result<(std::path::PathBuf, usize, Headers), String> {
        let (bytes, headers) = self.download(path)?;
        std::fs::create_dir_all(dir).map_err(|e| format!("无法创建 {}: {e}", dir.display()))?;
        let file = dir.join(headers.filename(default_name));
        std::fs::write(&file, &bytes).map_err(|e| format!("写入 {} 失败: {e}", file.display()))?;
        Ok((file, bytes.len(), headers))
    }

    pub fn post(&self, path: &str, body: &Value) -> Result<Response, String> {
        let url = format!("{}{}", self.base, path);
        self.send(self.agent.post(&url), Some(body))
    }

    pub fn delete(&self, path: &str) -> Result<Response, String> {
        let url = format!("{}{}", self.base, path);
        self.send(self.agent.delete(&url), None)
    }

    /// Download a binary artifact (Harness Package archive, Eval Repo archive).
    /// Returns the bytes plus the interesting response headers so the caller can
    /// verify the server-provided checksum.
    pub fn download(&self, path: &str) -> Result<(Vec<u8>, Headers), String> {
        let url = if path.starts_with("http://") || path.starts_with("https://") {
            path.to_string()
        } else {
            format!("{}{}", self.base, path)
        };
        let req = self.decorate(self.agent.get(&url));
        let resp = match req.call() {
            Ok(r) => r,
            Err(ureq::Error::Status(_, r)) => r,
            Err(ureq::Error::Transport(t)) => return Err(format!("网络错误：{t}")),
        };
        let status = resp.status();
        let headers = Headers {
            checksum: resp.header("X-Ratsa-Checksum").unwrap_or("").to_string(),
            level: resp.header("X-Ratsa-Package-Level").unwrap_or("").to_string(),
            disposition: resp.header("Content-Disposition").unwrap_or("").to_string(),
        };
        let mut buf = Vec::new();
        let mut reader = resp.into_reader();
        reader.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        if status >= 400 {
            return Err(format!(
                "下载失败（HTTP {status}）：{}",
                truncate(&String::from_utf8_lossy(&buf), 300)
            ));
        }
        Ok((buf, headers))
    }
}

#[derive(Debug, Default, Clone)]
pub struct Headers {
    pub checksum: String,
    pub level: String,
    pub disposition: String,
}

impl Headers {
    /// Filename suggested by the server, falling back to `default_name`.
    pub fn filename(&self, default_name: &str) -> String {
        if let Some(idx) = self.disposition.find("filename=") {
            let raw = self.disposition[idx + 9..].trim_matches(|c| c == '"' || c == ' ');
            if !raw.is_empty() {
                return raw.to_string();
            }
        }
        default_name.to_string()
    }
}

fn read(resp: ureq::Response) -> Result<Response, String> {
    let status = resp.status();
    let body = resp.into_string().map_err(|e| e.to_string())?;
    Ok(Response { status, body })
}

// ------------------------------------------------------------------ helpers

/// Build the scope array sent to `POST /api/keys`.
pub fn parse_scopes(raw: &[String]) -> Vec<String> {
    raw.iter()
        .flat_map(|s| s.split(','))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Normalize an account handle: `@demo` and `demo` are the same thing.
pub fn normalize_slug(raw: &str) -> String {
    raw.trim().trim_start_matches('@').to_string()
}

/// Turn a slug or URL into a bare slug. Accepts what a user copies out of the
/// browser (`https://ratsa.ai/device/xyz`, `…/api/harness/sof/xyz/package`,
/// `?ref=…`) as well as a bare slug.
pub fn slug_or_url(raw: &str) -> String {
    let s = raw.trim();
    let path = if let Some(rest) = s.split("//").nth(1) {
        rest.split_once('/').map(|(_, p)| p.to_string()).unwrap_or_default()
    } else {
        s.to_string()
    };
    let path = path.split(['?', '#']).next().unwrap_or("");
    let mut parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    // Drop trailing action words so `/api/harness/sof/xyz/package` → `xyz`.
    const ACTIONS: [&str; 6] = ["package", "download", "file", "manifest", "harness", "meta"];
    while parts
        .last()
        .map(|last| ACTIONS.contains(last))
        .unwrap_or(false)
    {
        parts.pop();
    }
    parts
        .last()
        .map(|s| (*s).to_string())
        .unwrap_or_else(|| s.to_string())
}
