//! `ratsa upgrade` —— 从发布通道拉取最新二进制，替换本地那一份。
//!
//! 为什么需要它：npm 包的版本号与 CLI 的 tag 是**相互独立**的（见 `npm/README.md`
//! 的版本策略）。代价是只改二进制时不改包版本，npm 便一直判定 "up to date"、
//! `postinstall` 不再重跑，已经装过的用户就永远停在安装那一刻的二进制上。
//! 这个子命令是那条通道之外唯一的更新入口，也因此不能依赖 npm。
//!
//! 约定与 `install.sh` / `npm/install.js` 完全一致，三者共用同一批资产名：
//!   * 资产名 `ratsa-latest-<os>-<arch>[.exe]`（版本无关的稳定别名）
//!   * 同目录的 `checksums.txt` 作为校验依据
//!   * 基址取自 `RATSA_RELEASE_BASE`，默认 `https://ratsa.ai/downloads`
//!
//! 补齐时**不带任何鉴权头**：这条通道是公开的静态资源（nginx 302 → 对象存储），
//! 没有理由把用户的 API key 发给一个 CDN。

use std::env;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

use crate::commands::sha256_hex;
use crate::config;

const DEFAULT_BASE: &str = "https://ratsa.ai/downloads";

/// 上一轮升级在 Windows 上留下的旧文件（运行中的 exe 删不掉，只能改名）。
/// 每次进来先试着清掉。
fn stale_names() -> [&'static str; 2] {
    if cfg!(windows) {
        ["ratsa.old.exe", "ratsa.new.exe"]
    } else {
        ["ratsa.old", "ratsa.new"]
    }
}

fn release_base() -> String {
    env::var("RATSA_RELEASE_BASE")
        .unwrap_or_else(|_| DEFAULT_BASE.to_string())
        .trim_end_matches('/')
        .to_string()
}

/// `linux-x64` / `darwin-arm64` / `windows-x64` —— 与 npm/lib/platform.js 的
/// platformTag() 必须逐字一致，否则拼出来的资产名在通道里不存在。
fn platform_tag() -> Result<String, String> {
    let os = match env::consts::OS {
        "linux" => "linux",
        "macos" => "darwin",
        "windows" => "windows",
        other => return Err(format!("暂不支持的系统：{other}（请用 npm 或 cargo 安装）")),
    };
    let arch = match env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => return Err(format!("暂不支持的架构：{other}")),
    };
    Ok(format!("{os}-{arch}"))
}

fn asset_name(tag: &str) -> String {
    let ext = if cfg!(windows) { ".exe" } else { "" };
    format!("ratsa-latest-{tag}{ext}")
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(180))
        .user_agent(concat!("ratsa-harness/", env!("CARGO_PKG_VERSION")))
        .build()
}

fn fetch(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>, String> {
    let resp = match agent.get(url).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(code, _)) => return Err(format!("{url} 返回 HTTP {code}")),
        Err(ureq::Error::Transport(t)) => return Err(format!("网络错误：{t}")),
    };
    let mut buf = Vec::new();
    resp.into_reader()
        .read_to_end(&mut buf)
        .map_err(|e| format!("读取响应失败：{e}"))?;
    Ok(buf)
}

/// 从 checksums.txt 里取出 `asset` 的期望 sha256。行为与 install.sh 对齐：
/// 清单缺失或没有该行都属于「无法校验」，由调用方决定是放行还是拒绝。
fn expected_checksum(manifest: &str, asset: &str) -> Option<String> {
    for line in manifest.lines() {
        // `sha256sum` 输出「<hex>␣␣<name>」；shasum -b 会多一个 `*` 前缀。
        let mut parts = line.split_whitespace();
        let (hash, name) = match (parts.next(), parts.next()) {
            (Some(h), Some(n)) => (h, n),
            _ => continue,
        };
        if name.trim_start_matches('*') == asset {
            return Some(hash.to_string());
        }
    }
    None
}

/// 就地把 `bytes` 装到 `target`。
///
/// Windows 不允许覆盖或删除**正在运行**的 exe，但允许改名。所以顺序是
/// 「先写新文件 → 把旧的改名挪开 → 把新的改名落位」，而不是直接覆盖。
/// 落位失败时把旧的改回去 —— 宁可没更新，也不能让用户手里没有可执行文件。
fn install_binary(target: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = target.parent().ok_or("目标路径没有父目录")?;
    fs::create_dir_all(dir).map_err(|e| format!("无法创建 {}：{e}", dir.display()))?;

    let [old_name, new_name] = stale_names();
    let new_path = dir.join(new_name);
    let old_path = dir.join(old_name);

    fs::write(&new_path, bytes).map_err(|e| format!("写入 {} 失败：{e}", new_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&new_path, fs::Permissions::from_mode(0o755));
    }

    if target.exists() {
        let _ = fs::remove_file(&old_path); // 上一轮的残留，能删就删
        fs::rename(target, &old_path)
            .map_err(|e| format!("无法移开旧文件 {}：{e}", target.display()))?;
    }

    match fs::rename(&new_path, target) {
        Ok(()) => {
            // 运行中的旧文件在 Windows 上删不掉，留给下次进来时清理。
            let _ = fs::remove_file(&old_path);
            Ok(())
        }
        Err(e) => {
            let _ = fs::rename(&old_path, target);
            let _ = fs::remove_file(&new_path);
            Err(format!("无法落位到 {}：{e}", target.display()))
        }
    }
}

pub fn run() -> Result<(), String> {
    let tag = platform_tag()?;
    let asset = asset_name(&tag);
    let base = release_base();
    let dir = config::home_dir().join("bin");
    let target = dir.join(crate::install::bin_file_name());

    println!("发布通道：{base}");
    println!("目标资产：{asset}");
    println!("安装位置：{}", target.display());

    for name in stale_names() {
        let _ = fs::remove_file(dir.join(name));
    }

    let agent = agent();
    let bin = fetch(&agent, &format!("{base}/{asset}")).map_err(|e| {
        format!(
            "{e}\n  产物可能还没发布到这个通道，或本机网络不通。\n  当前版本：{}",
            env!("CARGO_PKG_VERSION")
        )
    })?;

    // 校验。与 install.sh / install.js 保持同一口径：有清单就必须匹配（不匹配是致命的），
    // 没有清单只警告放行 —— 否则自建的 GitHub 前缀（不带 checksums.txt）会没法用。
    match fetch(&agent, &format!("{base}/checksums.txt"))
        .ok()
        .and_then(|m| expected_checksum(&String::from_utf8_lossy(&m), &asset))
    {
        Some(want) => {
            let got = sha256_hex(&bin);
            if !want.eq_ignore_ascii_case(&got) {
                return Err(format!(
                    "校验失败，已放弃升级。\n  期望 {want}\n  实际 {got}\n\
                     这份文件与清单不一致，不要使用；若反复出现请联系发布方。"
                ));
            }
            println!("校验  ：sha256 一致");
        }
        None => {
            println!("校验  ：通道未提供 checksums.txt 的对应条目，跳过（这不是正常状态）");
        }
    }

    // 比对再写：通道只提供版本无关的别名，没有单独的版本清单，所以「是否需要更新」
    // 只能靠内容比对得出。好处是绝不会因为版本号相同而漏掉重建的二进制。
    if let Ok(cur) = fs::read(&target) {
        if cur == bin {
            println!("\n已是最新，无需改动。");
            return Ok(());
        }
    }

    install_binary(&target, &bin)?;
    println!(
        "\n已更新：{}\n（本次运行仍是旧进程，下次启动生效）",
        target.display()
    );
    Ok(())
}
