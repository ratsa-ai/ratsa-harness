//! `ratsa host` component management: generate a Host Server config, install
//! the `rthost` binary, run it in the foreground, and supervise it with
//! systemd (Linux) or launchd (macOS).
//!
//! The platform only ever sees the Host URL + capability set; the local token
//! and rthd/rtm credentials live in `~/.ratsa/host/host.toml` (0600).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

pub struct InitOptions {
    pub config: Option<String>,
    pub force: bool,
    pub bind: String,
    pub profile: String,
    pub capabilities: Vec<String>,
    pub rthd: String,
    pub rthd_token: Option<String>,
    pub rtm: String,
    pub rtm_token: Option<String>,
    pub token: Option<String>,
    pub memory_store: Option<String>,
    pub sof_api: Option<String>,
    pub no_ota_signature: bool,
    pub json: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HostFile {
    host: HostSection,
    rthd: EndpointSection,
    rtm: EndpointSection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HostSection {
    bind: String,
    #[serde(default)]
    token: String,
    #[serde(default = "default_profile")]
    profile: String,
    #[serde(default = "default_true")]
    require_ota_signature: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    memory_store: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sof_api: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EndpointSection {
    url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    token: Option<String>,
}

fn default_profile() -> String {
    "operator".into()
}

fn default_true() -> bool {
    true
}

fn ratsa_home() -> PathBuf {
    if let Some(path) = std::env::var_os("RATSA_HOME") {
        return PathBuf::from(path);
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".ratsa");
    }
    PathBuf::from(".ratsa")
}

fn default_config_path() -> PathBuf {
    ratsa_home().join("host").join("host.toml")
}

#[cfg(target_os = "linux")]
fn user_config_home() -> PathBuf {
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(path);
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".config");
    }
    ratsa_home().join("config")
}

fn config_path(arg: Option<&str>) -> PathBuf {
    arg.map(PathBuf::from)
        .map(absolute_path)
        .unwrap_or_else(default_config_path)
}

fn absolute_path(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        return path;
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(&path))
        .unwrap_or(path)
}

fn installed_bin() -> PathBuf {
    ratsa_home().join("bin").join("rthost")
}

fn log_path() -> PathBuf {
    ratsa_home().join("host").join("rthost.log")
}

pub fn init(opts: InitOptions) -> Result<String, String> {
    let path = config_path(opts.config.as_deref());
    if path.exists() && !opts.force {
        return Err(format!("{} 已存在（用 --force 覆盖）", path.display()));
    }
    let profile = opts.profile.trim().to_ascii_lowercase();
    if !matches!(
        profile.as_str(),
        "operator" | "full" | "mobile" | "monitor" | "custom"
    ) {
        return Err(format!(
            "未知 profile：{}（可选 operator | full | mobile | monitor | custom）",
            opts.profile
        ));
    }
    if profile == "custom" && opts.capabilities.is_empty() {
        return Err("profile=custom 需要 --capability（可重复）".into());
    }
    if opts.bind.trim().is_empty() {
        return Err("--bind 不能为空".into());
    }
    let token = match opts.token.filter(|t| !t.trim().is_empty()) {
        Some(token) => token,
        None => random_token()?,
    };
    let memory_store = opts.memory_store.or_else(|| {
        Some(
            ratsa_home()
                .join("host")
                .join("memory.json")
                .display()
                .to_string(),
        )
    });
    let file = HostFile {
        host: HostSection {
            bind: opts.bind,
            token,
            profile,
            require_ota_signature: !opts.no_ota_signature,
            memory_store,
            sof_api: opts.sof_api,
            capabilities: opts.capabilities,
        },
        rthd: EndpointSection {
            url: opts.rthd,
            token: opts.rthd_token.filter(|t| !t.trim().is_empty()),
        },
        rtm: EndpointSection {
            url: opts.rtm,
            token: opts.rtm_token.filter(|t| !t.trim().is_empty()),
        },
    };
    let raw = toml::to_string_pretty(&file).map_err(|e| format!("生成配置失败：{e}"))?;
    write_private(&path, &raw)?;
    if opts.json {
        return Ok(format!(
            "{}\n",
            serde_json::to_string_pretty(&serde_json::json!({
                "config": path.display().to_string(),
                "bind": file.host.bind,
                "profile": file.host.profile,
                "token_saved": true,
                "binary": installed_bin().display().to_string(),
            }))
            .unwrap_or_default()
        ));
    }
    Ok(format!(
        "✓ 已生成 {}\n  bind    : {}\n  profile : {}\n  token   : 已写入配置（0600；`ratsa host config show --reveal` 查看）\n\n下一步：\n  ratsa host install --from-source <rthcore 目录>   # 构建并安装 rthost 组件\n  ratsa host doctor                                  # 检查配置 / 组件 / rthd / rtm\n  ratsa host run                                     # 前台运行\n  ratsa host service install                         # 安装为 systemd/launchd 服务\n",
        path.display(),
        file.host.bind,
        file.host.profile
    ))
}

pub fn config_show(config: Option<&str>, reveal: bool) -> Result<String, String> {
    let path = config_path(config);
    let raw = fs::read_to_string(&path).map_err(|e| {
        format!(
            "读取 {} 失败：{e}（先运行 `ratsa host init`）",
            path.display()
        )
    })?;
    if reveal {
        return Ok(raw);
    }
    let mut out = String::new();
    let mut in_devices = false;
    let mut in_telemetry = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_devices = trimmed == "[telemetry.devices]";
            in_telemetry = trimmed.starts_with("[telemetry");
        }
        let is_secret = trimmed.starts_with("token")
            || (in_telemetry && trimmed.starts_with("devices"))
            || (in_devices && !trimmed.is_empty() && !trimmed.starts_with('['));
        if is_secret {
            if let Some(eq) = line.find('=') {
                out.push_str(&format!("{} = \"***\"\n", line[..eq].trim_end()));
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    Ok(out)
}

pub fn install(
    bin: Option<&str>,
    from_source: Option<&str>,
    force: bool,
) -> Result<String, String> {
    let source = if let Some(dir) = from_source {
        build_from_source(dir)?
    } else if let Some(bin) = bin {
        let path = PathBuf::from(bin);
        if !path.exists() {
            return Err(format!("{} 不存在", path.display()));
        }
        path
    } else if let Some(found) = find_bundled("rthost").or_else(|| find_in_path("rthost")) {
        found
    } else {
        return Err(
            "未找到 rthost。用 `--bin <path>` 指定，或用 `--from-source <rthcore 目录>` 现场构建。"
                .into(),
        );
    };
    let target = installed_bin();
    if source == target {
        return Ok(format!("✓ rthost 已安装：{}\n", target.display()));
    }
    if target.exists() && !force {
        return Err(format!("{} 已存在（用 --force 覆盖）", target.display()));
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("无法创建 {}: {e}", parent.display()))?;
    }
    fs::copy(&source, &target)
        .map_err(|e| format!("复制 {} → {} 失败：{e}", source.display(), target.display()))?;
    set_mode(&target, 0o755)?;
    Ok(format!(
        "✓ 已安装 rthost 组件\n  from: {}\n  to  : {}\n",
        source.display(),
        target.display()
    ))
}

pub fn run(config: Option<&str>, bin: Option<&str>) -> Result<String, String> {
    let path = config_path(config);
    if !path.exists() {
        return Err(format!(
            "{} 不存在（先运行 `ratsa host init`）",
            path.display()
        ));
    }
    let bin = resolve_bin(bin)?;
    let mut command = Command::new(&bin);
    command.arg("--config").arg(&path);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = command.exec();
        Err(format!("启动 {} 失败：{error}", bin.display()))
    }
    #[cfg(not(unix))]
    {
        let status = command
            .status()
            .map_err(|e| format!("启动 {} 失败：{e}", bin.display()))?;
        if !status.success() {
            return Err(format!("rthost 退出：{status}"));
        }
        Ok(String::new())
    }
}

pub fn doctor(config: Option<&str>, bin: Option<&str>) -> Result<String, String> {
    let path = config_path(config);
    let mut out = String::new();
    let mut failures = 0usize;
    let mut check = |ok: bool, label: &str, detail: String| {
        out.push_str(&format!(
            "{} {label}{}\n",
            if ok { "✓" } else { "✗" },
            if detail.is_empty() {
                String::new()
            } else {
                format!(" — {detail}")
            }
        ));
        if !ok {
            failures += 1;
        }
    };

    let bin_path = resolve_bin(bin);
    match &bin_path {
        Ok(path) => {
            let version = Command::new(path)
                .arg("--version")
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_else(|| "unknown".into());
            check(
                true,
                "rthost 组件",
                format!("{} ({version})", path.display()),
            );
        }
        Err(error) => check(false, "rthost 组件", error.clone()),
    }

    let file = match read_config(&path) {
        Ok(file) => {
            check(true, "配置", path.display().to_string());
            Some(file)
        }
        Err(error) => {
            check(false, "配置", error);
            None
        }
    };

    if let (Ok(bin), Some(_)) = (&bin_path, &file) {
        match Command::new(bin)
            .arg("--config")
            .arg(&path)
            .arg("--check-config")
            .output()
        {
            Ok(output) if output.status.success() => {
                let summary = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_string();
                check(true, "配置校验", summary);
            }
            Ok(output) => check(
                false,
                "配置校验",
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ),
            Err(error) => check(false, "配置校验", format!("无法执行：{error}")),
        }
    }

    if let Some(file) = &file {
        match health_probe(&file.host.bind, &file.host.token) {
            Ok(v) => check(
                true,
                "Host /health",
                format!(
                    "{} {}",
                    v.get("version").and_then(|x| x.as_str()).unwrap_or("-"),
                    v.get("profile").and_then(|x| x.as_str()).unwrap_or("-")
                ),
            ),
            Err(error) => check(false, "Host /health", error),
        }
        for (name, endpoint) in [("rthd", &file.rthd), ("rtm", &file.rtm)] {
            match endpoint_probe(endpoint) {
                Ok(status) => check(
                    (200..400).contains(&status),
                    &format!("{name} /health"),
                    format!("HTTP {status} {}", endpoint.url),
                ),
                Err(error) => check(false, &format!("{name} /health"), error),
            }
        }
    }

    if failures > 0 {
        return Err(format!(
            "doctor: {failures} 项检查未通过\n{}",
            out.trim_end()
        ));
    }
    Ok(out)
}

/// Register the local component on the platform (URL + capabilities only).
pub fn publish(
    client: &crate::api::Client,
    name: &str,
    config: Option<&str>,
    url: Option<&str>,
    description: &str,
    json: bool,
) -> Result<String, String> {
    let path = config_path(config);
    let file = read_config(&path)?;
    let url = match url.filter(|u| !u.trim().is_empty()) {
        Some(url) => url.to_string(),
        None => {
            if file.host.bind.starts_with("0.0.0.0:")
                || file.host.bind.starts_with("[::]:")
                || file.host.bind.starts_with(':')
            {
                return Err(
                    "Host 绑定在通配地址；请用 `--url https://host.example.com:9780` 指定外部可访问地址"
                        .into(),
                );
            }
            format!("http://{}", file.host.bind)
        }
    };
    let caps = profile_capabilities(&file.host.profile, &file.host.capabilities)?;
    crate::commands::host_add(client, name, &url, description, &caps.join(","), json)
}

fn profile_capabilities(profile: &str, custom: &[String]) -> Result<Vec<String>, String> {
    let all = [
        "read_state",
        "read_logs",
        "read_events",
        "telemetry",
        "control_programs",
        "diagnostics",
        "ota",
        "rollback",
        "harness_control",
        "agentic",
        "configure_harness",
        "commands",
        "shutdown",
    ];
    let items: &[&str] = match profile.trim().to_ascii_lowercase().as_str() {
        "" | "operator" | "full" => &all,
        "mobile" => &[
            "read_state",
            "read_logs",
            "telemetry",
            "ota",
            "rollback",
            "harness_control",
            "agentic",
            "configure_harness",
        ],
        "monitor" => &["read_state", "read_logs", "read_events", "telemetry"],
        "custom" => {
            if custom.is_empty() {
                return Err("profile=custom 但配置中没有 capabilities".into());
            }
            return Ok(custom.to_vec());
        }
        other => return Err(format!("未知 profile：{other}")),
    };
    Ok(items.iter().map(|s| s.to_string()).collect())
}

pub fn service_install(
    config: Option<&str>,
    bin: Option<&str>,
    force: bool,
    no_start: bool,
    dry_run: bool,
) -> Result<String, String> {
    let cfg = config_path(config);
    ensure_component(&cfg, bin)?;
    #[cfg(target_os = "linux")]
    {
        return linux_service_install(&cfg, bin, force, no_start, dry_run);
    }
    #[cfg(target_os = "macos")]
    {
        return mac_service_install(&cfg, bin, force, no_start, dry_run);
    }
    #[allow(unreachable_code)]
    Err("当前系统不支持服务安装（仅 Linux systemd / macOS launchd）".into())
}

pub fn service_start(config: Option<&str>, bin: Option<&str>) -> Result<String, String> {
    let cfg = config_path(config);
    ensure_component(&cfg, bin)?;
    #[cfg(target_os = "linux")]
    {
        return linux_systemctl(&["start", "ratsa-rthost"], "启动");
    }
    #[cfg(target_os = "macos")]
    {
        return mac_service_start(&cfg);
    }
    #[allow(unreachable_code)]
    Err("当前系统不支持服务管理".into())
}

pub fn service_stop() -> Result<String, String> {
    #[cfg(target_os = "linux")]
    {
        return linux_systemctl(&["stop", "ratsa-rthost"], "停止");
    }
    #[cfg(target_os = "macos")]
    {
        return mac_service_stop();
    }
    #[allow(unreachable_code)]
    Err("当前系统不支持服务管理".into())
}

pub fn service_restart(config: Option<&str>, bin: Option<&str>) -> Result<String, String> {
    let cfg = config_path(config);
    ensure_component(&cfg, bin)?;
    #[cfg(target_os = "linux")]
    {
        return linux_systemctl(&["restart", "ratsa-rthost"], "重启");
    }
    #[cfg(target_os = "macos")]
    {
        return mac_service_restart(&cfg);
    }
    #[allow(unreachable_code)]
    Err("当前系统不支持服务管理".into())
}

pub fn service_status(config: Option<&str>) -> Result<String, String> {
    let cfg = config_path(config);
    let mut out = String::new();
    #[cfg(target_os = "linux")]
    {
        let active = Command::new("systemctl")
            .args(["--user", "is-active", "ratsa-rthost"])
            .output();
        match active {
            Ok(output) => {
                let state = String::from_utf8_lossy(&output.stdout).trim().to_string();
                out.push_str(&format!("systemd : {state}\n"));
                if state != "active" {
                    out.push_str(
                        "hint    : ratsa host service install / ratsa host service start\n",
                    );
                }
            }
            Err(error) => out.push_str(&format!("systemd : 不可用（{error}）\n")),
        }
    }
    #[cfg(target_os = "macos")]
    {
        let loaded = mac_loaded();
        out.push_str(&format!(
            "launchd : {}\n",
            if loaded { "loaded" } else { "not loaded" }
        ));
        if !loaded {
            out.push_str("hint    : ratsa host service install / ratsa host service start\n");
        }
    }
    if let Ok(file) = read_config(&cfg) {
        match health_probe(&file.host.bind, &file.host.token) {
            Ok(v) => out.push_str(&format!(
                "health  : ok {} · {}\n",
                v.get("version").and_then(|x| x.as_str()).unwrap_or("-"),
                v.get("profile").and_then(|x| x.as_str()).unwrap_or("-")
            )),
            Err(error) => out.push_str(&format!("health  : unavailable（{error}）\n")),
        }
    } else {
        out.push_str(&format!("config  : {} 不存在\n", cfg.display()));
    }
    Ok(out)
}

pub fn service_logs(lines: usize, follow: bool) -> Result<String, String> {
    #[cfg(target_os = "linux")]
    {
        let mut args = vec![
            "--user".to_string(),
            "-u".to_string(),
            "ratsa-rthost".to_string(),
            "-n".to_string(),
            lines.to_string(),
            "--no-pager".to_string(),
        ];
        if follow {
            args.push("-f".into());
        }
        let status = Command::new("journalctl")
            .args(&args)
            .status()
            .map_err(|e| format!("无法运行 journalctl：{e}"))?;
        if !status.success() {
            return Err("journalctl 读取失败；确认服务已安装".into());
        }
        return Ok(String::new());
    }
    #[cfg(target_os = "macos")]
    {
        return mac_service_logs(lines, follow);
    }
    #[allow(unreachable_code)]
    Err("当前系统不支持服务日志".into())
}

pub fn service_uninstall() -> Result<String, String> {
    #[cfg(target_os = "linux")]
    {
        let _ = Command::new("systemctl")
            .args(["--user", "disable", "--now", "ratsa-rthost"])
            .status();
        let path = systemd_unit_path();
        if path.exists() {
            fs::remove_file(&path).map_err(|e| format!("删除 {} 失败：{e}", path.display()))?;
        }
        let _ = Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status();
        return Ok("✓ 已卸载 rthost 服务（配置与日志保留）\n".into());
    }
    #[cfg(target_os = "macos")]
    {
        return mac_service_uninstall();
    }
    #[allow(unreachable_code)]
    Err("当前系统不支持服务管理".into())
}

fn ensure_component(cfg: &Path, bin: Option<&str>) -> Result<PathBuf, String> {
    if !cfg.exists() {
        return Err(format!(
            "{} 不存在（先运行 `ratsa host init`）",
            cfg.display()
        ));
    }
    resolve_bin(bin)
}

fn resolve_bin(explicit: Option<&str>) -> Result<PathBuf, String> {
    if let Some(path) = explicit.filter(|p| !p.trim().is_empty()) {
        let path = absolute_path(PathBuf::from(path));
        if path.exists() {
            return Ok(path);
        }
        return Err(format!("{} 不存在", path.display()));
    }
    if let Some(path) = std::env::var_os("RTHOST_BIN") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Ok(path);
        }
    }
    let installed = installed_bin();
    if installed.exists() {
        return Ok(installed);
    }
    find_bundled("rthost")
        .or_else(|| find_in_path("rthost"))
        .ok_or_else(|| {
        "未找到 rthost 组件。安装：`ratsa host install --from-source <rthcore 目录>`，或 `--bin <path>`，或设置 RTHOST_BIN。".into()
        })
}

fn find_bundled(name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [dir.join(name), dir.join(format!("{name}.exe"))]
        .into_iter()
        .find(|candidate| candidate.is_file())
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn build_from_source(dir: &str) -> Result<PathBuf, String> {
    let dir = PathBuf::from(dir);
    if !dir.join("Cargo.toml").exists() {
        return Err(format!(
            "{} 不是 Cargo 工作区（缺少 Cargo.toml）",
            dir.display()
        ));
    }
    let status = Command::new("cargo")
        .args(["build", "--release", "-p", "rthost", "--features", "bin"])
        .current_dir(&dir)
        .status()
        .map_err(|e| format!("无法运行 cargo：{e}"))?;
    if !status.success() {
        return Err("cargo build --release -p rthost --features bin 失败".into());
    }
    let bin = dir.join("target").join("release").join("rthost");
    if !bin.exists() {
        return Err(format!("构建完成但未找到 {}", bin.display()));
    }
    Ok(bin)
}

fn read_config(path: &Path) -> Result<HostFile, String> {
    let raw = fs::read_to_string(path).map_err(|e| format!("读取 {} 失败：{e}", path.display()))?;
    toml::from_str(&raw).map_err(|e| format!("解析 {} 失败：{e}", path.display()))
}

fn write_private(path: &Path, raw: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("无法创建 {}: {e}", parent.display()))?;
        set_mode(parent, 0o700)?;
    }
    fs::write(path, raw).map_err(|e| format!("写入 {} 失败：{e}", path.display()))?;
    set_mode(path, 0o600)
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|e| format!("设置 {} 权限失败：{e}", path.display()))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> Result<(), String> {
    Ok(())
}

fn random_token() -> Result<String, String> {
    use ring::rand::{SecureRandom, SystemRandom};
    let rng = SystemRandom::new();
    let mut bytes = [0u8; 32];
    rng.fill(&mut bytes)
        .map_err(|_| "生成 token 失败".to_string())?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn loopback_addr(bind: &str) -> String {
    let bind = bind.trim();
    if let Some(rest) = bind.strip_prefix("0.0.0.0:") {
        return format!("127.0.0.1:{rest}");
    }
    if let Some(rest) = bind.strip_prefix("[::]:") {
        return format!("[::1]:{rest}");
    }
    if let Some(rest) = bind.strip_prefix(':') {
        return format!("127.0.0.1:{rest}");
    }
    bind.to_string()
}

fn health_probe(bind: &str, token: &str) -> Result<serde_json::Value, String> {
    let base = format!("http://{}", loopback_addr(bind));
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(2))
        .timeout_read(std::time::Duration::from_secs(3))
        .build();
    let mut req = agent.get(&format!("{base}/health"));
    if !token.trim().is_empty() {
        req = req.set("Authorization", &format!("Bearer {token}"));
    }
    match req.call() {
        Ok(resp) => Ok(
            serde_json::from_str(&resp.into_string().unwrap_or_default())
                .unwrap_or_else(|_| serde_json::json!({})),
        ),
        Err(ureq::Error::Status(status, _)) => Err(format!("HTTP {status} ({base}/health)")),
        Err(ureq::Error::Transport(error)) => Err(format!("{error} ({base}/health)")),
    }
}

fn endpoint_probe(endpoint: &EndpointSection) -> Result<u16, String> {
    let url = format!("{}/health", endpoint.url.trim_end_matches('/'));
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(2))
        .timeout_read(std::time::Duration::from_secs(3))
        .build();
    let mut req = agent.get(&url);
    if let Some(token) = endpoint.token.as_deref().filter(|t| !t.trim().is_empty()) {
        req = req.set("Authorization", &format!("Bearer {token}"));
    }
    match req.call() {
        Ok(resp) => Ok(resp.status()),
        Err(ureq::Error::Status(status, _)) => Ok(status),
        Err(ureq::Error::Transport(error)) => Err(format!("{error} ({url})")),
    }
}

#[cfg(target_os = "linux")]
fn run_status(command: &mut Command, what: &str) -> Result<String, String> {
    let output = command
        .output()
        .map_err(|e| format!("无法运行 {what}：{e}"))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Err(if stderr.is_empty() { stdout } else { stderr })
}

// ------------------------------------------------------------------- Linux

#[cfg(target_os = "linux")]
fn systemd_unit_path() -> PathBuf {
    user_config_home()
        .join("systemd")
        .join("user")
        .join("ratsa-rthost.service")
}

#[cfg(target_os = "linux")]
fn systemd_unit(bin: &Path, cfg: &Path) -> String {
    format!(
        "[Unit]\nDescription=RATSA rthost Host Server\nAfter=network-online.target\nWants=network-online.target\n\n[Service]\nType=simple\nExecStart=\"{}\" --config \"{}\"\nRestart=on-failure\nRestartSec=3\n\n[Install]\nWantedBy=default.target\n",
        bin.display(),
        cfg.display()
    )
}

#[cfg(target_os = "linux")]
fn linux_service_install(
    cfg: &Path,
    bin: Option<&str>,
    force: bool,
    no_start: bool,
    dry_run: bool,
) -> Result<String, String> {
    let bin = resolve_bin(bin)?;
    let unit = systemd_unit(&bin, cfg);
    if dry_run {
        return Ok(unit);
    }
    let path = systemd_unit_path();
    if path.exists() && !force {
        return Err(format!("{} 已存在（用 --force 覆盖）", path.display()));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建 {} 失败：{e}", parent.display()))?;
    }
    fs::write(&path, &unit).map_err(|e| format!("写入 {} 失败：{e}", path.display()))?;
    let reload = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .output();
    if let Ok(output) = reload {
        if !output.status.success() {
            return Err(format!(
                "systemctl --user daemon-reload 失败：{}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
    }
    let mut command = Command::new("systemctl");
    command.args(["--user", "enable"]);
    if !no_start {
        command.arg("--now");
    }
    command.arg("ratsa-rthost");
    run_status(&mut command, "systemctl")?;
    Ok(format!(
        "✓ 已安装 systemd 用户服务 ratsa-rthost\n  unit: {}\n  bin : {}\n  cfg : {}\n\nhint: 需要退出登录后仍运行：loginctl enable-linger $USER\n",
        path.display(),
        bin.display(),
        cfg.display()
    ))
}

#[cfg(target_os = "linux")]
fn linux_systemctl(args: &[&str], what: &str) -> Result<String, String> {
    let mut command = Command::new("systemctl");
    command.arg("--user").args(args);
    run_status(&mut command, "systemctl")?;
    Ok(format!("✓ 已{what} ratsa-rthost\n"))
}

// ------------------------------------------------------------------- macOS

#[cfg(target_os = "macos")]
fn mac_label() -> &'static str {
    "ai.ratsa.rthost"
}

#[cfg(target_os = "macos")]
fn mac_plist_path() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{}.plist", mac_label()))
}

#[cfg(target_os = "macos")]
fn mac_domain() -> String {
    let uid = Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "501".into());
    format!("gui/{uid}")
}

#[cfg(target_os = "macos")]
fn xml_escape(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(target_os = "macos")]
fn mac_plist(bin: &Path, cfg: &Path, no_start: bool) -> String {
    let log = log_path();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{label}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{bin}</string>
    <string>--config</string>
    <string>{cfg}</string>
  </array>
  <key>RunAtLoad</key><{run}/>
  <key>KeepAlive</key><true/>
  <key>WorkingDirectory</key><string>{dir}</string>
  <key>StandardOutPath</key><string>{log}</string>
  <key>StandardErrorPath</key><string>{log}</string>
</dict>
</plist>
"#,
        label = mac_label(),
        bin = xml_escape(&bin.display().to_string()),
        cfg = xml_escape(&cfg.display().to_string()),
        run = if no_start { "false" } else { "true" },
        dir = xml_escape(&ratsa_home().join("host").display().to_string()),
        log = xml_escape(&log.display().to_string()),
    )
}

#[cfg(target_os = "macos")]
fn mac_loaded() -> bool {
    Command::new("launchctl")
        .args(["print", &format!("{}/{}", mac_domain(), mac_label())])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn mac_service_install(
    cfg: &Path,
    bin: Option<&str>,
    force: bool,
    no_start: bool,
    dry_run: bool,
) -> Result<String, String> {
    let bin = resolve_bin(bin)?;
    let plist = mac_plist(&bin, cfg, no_start);
    if dry_run {
        return Ok(plist);
    }
    let path = mac_plist_path();
    if path.exists() && !force {
        return Err(format!("{} 已存在（用 --force 覆盖）", path.display()));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建 {} 失败：{e}", parent.display()))?;
    }
    if let Some(parent) = log_path().parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建 {} 失败：{e}", parent.display()))?;
    }
    fs::write(&path, &plist).map_err(|e| format!("写入 {} 失败：{e}", path.display()))?;
    let _ = Command::new("launchctl")
        .args(["bootout", &format!("{}/{}", mac_domain(), mac_label())])
        .status();
    let domain = mac_domain();
    let output = Command::new("launchctl")
        .args(["bootstrap", &domain])
        .arg(&path)
        .output()
        .map_err(|e| format!("无法运行 launchctl：{e}"))?;
    if !output.status.success() {
        return Err(format!(
            "launchctl bootstrap 失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    if !no_start {
        let _ = Command::new("launchctl")
            .args([
                "kickstart",
                "-k",
                &format!("{}/{}", mac_domain(), mac_label()),
            ])
            .status();
    }
    Ok(format!(
        "✓ 已安装 launchd 服务 {}\n  plist: {}\n  bin  : {}\n  cfg  : {}\n  log  : {}\n",
        mac_label(),
        path.display(),
        bin.display(),
        cfg.display(),
        log_path().display()
    ))
}

#[cfg(target_os = "macos")]
fn mac_service_start(_cfg: &Path) -> Result<String, String> {
    let target = format!("{}/{}", mac_domain(), mac_label());
    if !mac_loaded() {
        let path = mac_plist_path();
        if !path.exists() {
            return Err("launchd 服务未安装（先运行 `ratsa host service install`）".into());
        }
        let output = Command::new("launchctl")
            .args(["bootstrap", &mac_domain()])
            .arg(&path)
            .output()
            .map_err(|e| format!("无法运行 launchctl：{e}"))?;
        if !output.status.success() {
            return Err(format!(
                "launchctl bootstrap 失败：{}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
    }
    let output = Command::new("launchctl")
        .args(["kickstart", "-k", &target])
        .output()
        .map_err(|e| format!("无法运行 launchctl：{e}"))?;
    if !output.status.success() {
        return Err(format!(
            "launchctl kickstart 失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok("✓ 已启动 rthost（launchd）\n".into())
}

#[cfg(target_os = "macos")]
fn mac_service_stop() -> Result<String, String> {
    let target = format!("{}/{}", mac_domain(), mac_label());
    let output = Command::new("launchctl")
        .args(["bootout", &target])
        .output()
        .map_err(|e| format!("无法运行 launchctl：{e}"))?;
    if !output.status.success() && mac_loaded() {
        return Err(format!(
            "launchctl bootout 失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok("✓ 已停止 rthost（launchd）\n".into())
}

#[cfg(target_os = "macos")]
fn mac_service_restart(_cfg: &Path) -> Result<String, String> {
    if !mac_loaded() {
        return mac_service_start(_cfg);
    }
    let target = format!("{}/{}", mac_domain(), mac_label());
    let output = Command::new("launchctl")
        .args(["kickstart", "-k", &target])
        .output()
        .map_err(|e| format!("无法运行 launchctl：{e}"))?;
    if !output.status.success() {
        return Err(format!(
            "launchctl kickstart 失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok("✓ 已重启 rthost（launchd）\n".into())
}

#[cfg(target_os = "macos")]
fn mac_service_logs(lines: usize, follow: bool) -> Result<String, String> {
    let path = log_path();
    if follow {
        let status = Command::new("tail")
            .arg("-n")
            .arg(lines.to_string())
            .arg("-f")
            .arg(&path)
            .status()
            .map_err(|e| format!("无法读取 {}：{e}", path.display()))?;
        if !status.success() {
            return Err(format!("读取 {} 失败", path.display()));
        }
        return Ok(String::new());
    }
    let raw = fs::read_to_string(&path).unwrap_or_default();
    let tail: Vec<&str> = raw.lines().rev().take(lines).collect();
    let mut out = String::new();
    for line in tail.into_iter().rev() {
        out.push_str(line);
        out.push('\n');
    }
    Ok(out)
}

#[cfg(target_os = "macos")]
fn mac_service_uninstall() -> Result<String, String> {
    let target = format!("{}/{}", mac_domain(), mac_label());
    let _ = Command::new("launchctl")
        .args(["bootout", &target])
        .status();
    let path = mac_plist_path();
    if path.exists() {
        fs::remove_file(&path).map_err(|e| format!("删除 {} 失败：{e}", path.display()))?;
    }
    Ok("✓ 已卸载 rthost launchd 服务（配置与日志保留）\n".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrip_redacts_tokens() {
        let dir = std::env::temp_dir().join(format!("ratsa-host-{}", std::process::id()));
        let path = dir.join("host.toml");
        let _ = fs::remove_dir_all(&dir);
        let out = init(InitOptions {
            config: Some(path.display().to_string()),
            force: true,
            bind: "127.0.0.1:9780".into(),
            profile: "operator".into(),
            capabilities: vec![],
            rthd: "http://127.0.0.1:9770".into(),
            rthd_token: Some("rthd-token".into()),
            rtm: "http://127.0.0.1:9771".into(),
            rtm_token: None,
            token: Some("host-token".into()),
            memory_store: None,
            sof_api: None,
            no_ota_signature: false,
            json: false,
        })
        .expect("init");
        assert!(out.contains("127.0.0.1:9780"));
        let shown = config_show(Some(&path.display().to_string()), false).expect("show");
        assert!(shown.contains("***"));
        assert!(!shown.contains("host-token"));
        assert!(!shown.contains("rthd-token"));
        let revealed = config_show(Some(&path.display().to_string()), true).expect("reveal");
        assert!(revealed.contains("host-token"));
        let file = read_config(&path).expect("parse");
        assert_eq!(file.host.profile, "operator");
        assert!(file.host.require_ota_signature);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn loopback_normalizes_wildcards() {
        assert_eq!(loopback_addr("0.0.0.0:9780"), "127.0.0.1:9780");
        assert_eq!(loopback_addr("[::]:9780"), "[::1]:9780");
        assert_eq!(loopback_addr("127.0.0.1:9780"), "127.0.0.1:9780");
    }

    #[test]
    fn profile_capabilities_match_rthost_profiles() {
        let operator = profile_capabilities("operator", &[]).expect("operator");
        assert!(operator.contains(&"shutdown".to_string()));
        let mobile = profile_capabilities("mobile", &[]).expect("mobile");
        assert!(mobile.contains(&"agentic".to_string()));
        assert!(!mobile.contains(&"control_programs".to_string()));
        let custom = profile_capabilities("custom", &["read_state".into(), "agentic".into()])
            .expect("custom");
        assert_eq!(custom, vec!["read_state", "agentic"]);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn launchd_plist_references_component_and_config() {
        let plist = mac_plist(Path::new("/tmp/rthost"), Path::new("/tmp/host.toml"), true);
        assert!(plist.contains("<string>ai.ratsa.rthost</string>"));
        assert!(plist.contains("<string>--config</string>"));
        assert!(plist.contains("<string>/tmp/host.toml</string>"));
        assert!(plist.contains("<key>RunAtLoad</key><false/>"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn systemd_unit_lives_in_the_user_config_dir() {
        assert!(systemd_unit_path()
            .to_string_lossy()
            .ends_with("/.config/systemd/user/ratsa-rthost.service"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn systemd_unit_quotes_execstart() {
        let unit = systemd_unit(Path::new("/tmp/rthost"), Path::new("/tmp/host.toml"));
        assert!(unit.contains("ExecStart=\"/tmp/rthost\" --config \"/tmp/host.toml\""));
        assert!(unit.contains("WantedBy=default.target"));
    }
}
