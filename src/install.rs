//! Agent-side installers.
//!
//! One command wires RATSA into whichever Agent the user already runs. Each
//! target is a *thin* integration: a skill/rules file that teaches the Agent how
//! to talk to RATSA, plus (where the Agent supports it) an MCP server entry so
//! the same capabilities are available as tools.
//!
//! Targets:
//!
//! | agent   | files |
//! |---------|-------|
//! | claude  | `.claude/skills/ratsa/SKILL.md` |
//! | mcp     | `.mcp.json` (`mcpServers.ratsa`) |
//! | cursor  | `.cursor/rules/ratsa.mdc` + `.cursor/mcp.json` |
//! | copilot | `.github/copilot-instructions.md` + `.github/skills/ratsa/SKILL.md` + `.vscode/mcp.json` |
//! | agents  | `AGENTS.md` |
//! | codex   | `~/.codex/config.toml` (`[mcp_servers.ratsa]`) |
//!
//! `--global` installs into the user's home instead of the current project, so
//! the Agent learns RATSA in every workspace.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::config::Config;

pub const TARGETS: [&str; 6] = ["claude", "mcp", "cursor", "copilot", "agents", "codex"];

/// One installable Agent integration. This table is the **single source of
/// truth** for `--agent` values, `ratsa agents`, the `--help` text and the
/// public Harness page on ratsa.ai — keep them in sync when adding a target.
#[derive(Debug, Clone, Copy)]
pub struct TargetInfo {
    pub id: &'static str,
    pub label: &'static str,
    /// 接入形态：skill | rules | instructions | agents | mcp | mcp+skill
    pub shape: &'static str,
    /// 落盘路径（`--global` 时为用户目录；`~` 表示 home）
    pub files: &'static [&'static str],
    /// 是否注册 MCP(stdin/stdout) 服务
    pub mcp: bool,
}

pub fn target_table() -> &'static [TargetInfo] {
    &[
        TargetInfo {
            id: "claude",
            label: "Claude Code",
            shape: "skill",
            files: &[".claude/skills/ratsa/SKILL.md"],
            mcp: false,
        },
        TargetInfo {
            id: "mcp",
            label: "MCP client (generic)",
            shape: "mcp",
            files: &[".mcp.json"],
            mcp: true,
        },
        TargetInfo {
            id: "cursor",
            label: "Cursor",
            shape: "rules+mcp",
            files: &[".cursor/rules/ratsa.mdc", ".cursor/mcp.json"],
            mcp: true,
        },
        TargetInfo {
            id: "copilot",
            label: "GitHub Copilot (VS Code)",
            shape: "instructions+mcp",
            files: &[
                ".github/copilot-instructions.md",
                ".github/skills/ratsa/SKILL.md",
                ".vscode/mcp.json",
            ],
            mcp: true,
        },
        TargetInfo {
            id: "agents",
            label: "AGENTS.md (universal)",
            shape: "agents",
            files: &["AGENTS.md"],
            mcp: false,
        },
        TargetInfo {
            id: "codex",
            label: "OpenAI Codex CLI",
            shape: "mcp+agents",
            files: &["~/.codex/config.toml", "~/.codex/RATSA.md"],
            mcp: true,
        },
    ]
}

pub struct Options {
    pub agent: String,
    pub dry_run: bool,
    pub global: bool,
    pub force: bool,
    pub dir: Option<PathBuf>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            agent: "all".into(),
            dry_run: false,
            global: false,
            force: false,
            dir: None,
        }
    }
}

/// A rendered file ready to be written (or shown under `--dry-run`).
struct Payload {
    path: PathBuf,
    content: String,
    merge: Option<MergeSpec>,
}

struct MergeSpec {
    /// Dotted path of the object that must contain `key`, e.g. `mcpServers`.
    parent: Vec<String>,
    key: String,
    value: Value,
}

struct Report {
    lines: Vec<String>,
    warnings: Vec<String>,
}

#[derive(Clone)]
struct Env {
    bin: String,
    base_url: String,
    scopes: Vec<String>,
    bound_slug: String,
    key_name: String,
}

pub fn run(cfg: &Config, opt: &Options) -> Result<(), String> {
    let targets = expand(&opt.agent)?;
    let root = match opt.dir.clone() {
        Some(d) => d,
        None if opt.global => home(),
        None => env::current_dir().map_err(|e| e.to_string())?,
    };
    let env_info = Env {
        bin: current_bin()?,
        base_url: cfg.base(),
        scopes: cfg.scopes.clone(),
        bound_slug: if !cfg.acting_as.is_empty() {
            cfg.acting_as.clone()
        } else {
            cfg.bound_slug.clone()
        },
        key_name: cfg.key_name.clone(),
    };

    let mut report = Report {
        lines: Vec::new(),
        warnings: Vec::new(),
    };

    println!("RATSA-Harness 安装器");
    println!("  目标 Agent : {}", targets.join(", "));
    println!(
        "  安装位置   : {}{}",
        root.display(),
        if opt.global { "（全局）" } else { "（当前项目）" }
    );
    println!("  可执行文件 : {}", env_info.bin);
    println!("  base_url   : {}", env_info.base_url);
    if cfg.has_key() {
        let scope = if env_info.scopes.is_empty() {
            "(未缓存，可运行 `ratsa-harness whoami` 刷新)".to_string()
        } else {
            env_info.scopes.join(", ")
        };
        println!("  key        : {} [{}]", cfg.key_name, scope);
        if !env_info.bound_slug.is_empty() {
            println!("  归属绑定   : @{}（该 key 以该账号身份行动）", env_info.bound_slug);
        }
    } else {
        println!("  凭证       : 尚未配置 —— 先运行 `ratsa-harness login` 或 `ratsa-harness key set`");
    }
    println!();

    for t in &targets {
        let payloads = payloads(t, &root, &env_info, opt);
        for p in payloads {
            apply(&p, opt, &mut report);
        }
    }

    println!();
    for line in &report.lines {
        println!("{line}");
    }
    if !report.warnings.is_empty() {
        println!();
        for w in &report.warnings {
            println!("注意：{w}");
        }
    }
    if opt.dry_run {
        println!();
        println!("（--dry-run：未写入任何文件）");
    } else {
        println!();
        println!("完成。让 Agent 直接问「用 RATSA 找一个舵机并拉取 Harness 包」即可。");
        if !cfg.has_key() {
            println!("记得先配置凭证：`ratsa-harness login --email <你的邮箱>`");
        }
    }
    Ok(())
}

fn expand(agent: &str) -> Result<Vec<String>, String> {
    let a = agent.trim().to_lowercase();
    if a == "all" {
        return Ok(TARGETS.iter().map(|s| s.to_string()).collect());
    }
    let parts: Vec<String> = a
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    for p in &parts {
        if !TARGETS.contains(&p.as_str()) {
            return Err(format!(
                "未知 Agent「{p}」，可选：{} 或 all",
                TARGETS.join(" | ")
            ));
        }
    }
    if parts.is_empty() {
        return Err("请指定 --agent".into());
    }
    Ok(parts)
}

/// `ratsa agents` — print (or emit as JSON) the supported Agent integrations.
pub fn print_agents(json_out: bool) -> Result<(), String> {
    let bin = current_bin().unwrap_or_else(|_| "ratsa".into());
    if json_out {
        let rows: Vec<Value> = target_table()
            .iter()
            .map(|t| {
                json!({
                    "id": t.id,
                    "label": t.label,
                    "shape": t.shape,
                    "files": t.files,
                    "mcp": t.mcp,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "bin": bin,
                "install": "ratsa install --agent all",
                "mcp_command": format!("{bin} mcp"),
                "agents": rows,
            }))
            .unwrap_or_default()
        );
        return Ok(());
    }
    println!("支持的 Agent（`ratsa install --agent <id>`，可逗号多选或 all）\n");
    for t in target_table() {
        println!("{:<10} {:<26} {:<18} {}", t.id, t.label, t.shape, t.files.join(" + "));
        if t.mcp {
            println!("{:<10} └─ MCP: {} mcp", "", bin);
        }
    }
    println!();
    println!("当前项目安装：  ratsa install --agent all");
    println!("用户目录安装：  ratsa install --agent all --global");
    println!("先看会写什么：  ratsa install --agent all --dry-run");
    Ok(())
}

fn payloads(target: &str, root: &Path, env_info: &Env, opt: &Options) -> Vec<Payload> {
    let skill = render(SKILL_MD, env_info);
    let mcp_json = json!({
        "command": env_info.bin,
        "args": ["mcp"],
    });
    match target {
        "claude" => vec![Payload {
            path: root.join(".claude/skills/ratsa/SKILL.md"),
            content: skill,
            merge: None,
        }],
        "mcp" => vec![Payload {
            path: root.join(".mcp.json"),
            content: String::new(),
            merge: Some(MergeSpec {
                parent: vec!["mcpServers".into()],
                key: "ratsa".into(),
                value: mcp_json,
            }),
        }],
        "cursor" => vec![
            Payload {
                path: root.join(".cursor/rules/ratsa.mdc"),
                content: render(CURSOR_MDC, env_info),
                merge: None,
            },
            Payload {
                path: root.join(".cursor/mcp.json"),
                content: String::new(),
                merge: Some(MergeSpec {
                    parent: vec!["mcpServers".into()],
                    key: "ratsa".into(),
                    value: mcp_json,
                }),
            },
        ],
        "copilot" => {
            let mut v = vec![
                Payload {
                    path: root.join(".github/copilot-instructions.md"),
                    content: render(COPILOT_MD, env_info),
                    merge: None,
                },
                Payload {
                    path: root.join(".github/skills/ratsa/SKILL.md"),
                    content: skill,
                    merge: None,
                },
                Payload {
                    path: root.join(".vscode/mcp.json"),
                    content: String::new(),
                    merge: Some(MergeSpec {
                        parent: vec!["servers".into()],
                        key: "ratsa".into(),
                        value: json!({
                            "type": "stdio",
                            "command": env_info.bin,
                            "args": ["mcp"],
                        }),
                    }),
                },
            ];
            // A global VS Code install targets the user prompt folder.
            if opt.global {
                if let Some(dir) = vscode_user_prompts() {
                    v.push(Payload {
                        path: dir.join("ratsa.instructions.md"),
                        content: render(COPILOT_MD, env_info),
                        merge: None,
                    });
                }
            }
            v
        }
        "agents" => vec![Payload {
            path: root.join("AGENTS.md"),
            content: render(AGENTS_MD, env_info),
            merge: None,
        }],
        "codex" => {
            let mut v = Vec::new();
            let codex_dir = if opt.dir.is_some() {
                root.join(".codex")
            } else {
                home().join(".codex")
            };
            v.push(Payload {
                path: codex_dir.join("config.toml"),
                content: String::new(),
                merge: Some(MergeSpec {
                    parent: vec![],
                    key: "mcp_servers".into(),
                    value: json!({ "ratsa": { "command": env_info.bin, "args": ["mcp"] } }),
                }),
            });
            v.push(Payload {
                path: codex_dir.join("RATSA.md"),
                content: render(AGENTS_MD, env_info),
                merge: None,
            });
            v
        }
        _ => Vec::new(),
    }
}

fn apply(p: &Payload, opt: &Options, report: &mut Report) {
    let shown = display_path(&p.path);

    // Merge-style targets (MCP / Codex): never clobber the rest of the file.
    if let Some(m) = p.merge.as_ref() {
        let current = fs::read_to_string(&p.path).ok();
        match merge_content(&p.path, m, current.as_deref(), opt.force) {
            None => report
                .lines
                .push(format!("  · 跳过 {shown}（已包含 ratsa 条目，--force 覆盖）")),
            Some((content, note)) => {
                if opt.dry_run {
                    report.lines.push(format!(
                        "  [dry-run] 合并 {shown} — {note}\n{}",
                        indent(&content, 8)
                    ));
                } else if let Err(e) = write(&p.path, &content, false) {
                    report.warnings.push(format!("{shown}：{e}"));
                } else {
                    report.lines.push(format!("  ✓ 合并 {shown} — {note}"));
                }
            }
        }
        return;
    }

    // Plain files (skill / rules / instructions).
    if p.path.exists() && !opt.force {
        if let Ok(existing) = fs::read_to_string(&p.path) {
            if existing.contains("RATSA") {
                report
                    .lines
                    .push(format!("  · 跳过 {shown}（已存在 RATSA 配置，--force 覆盖）"));
                return;
            }
        }
    }
    if opt.dry_run {
        report.lines.push(format!(
            "  [dry-run] 写入 {shown}（{} 字节）",
            p.content.len()
        ));
    } else if let Err(e) = write(&p.path, &p.content, true) {
        report.warnings.push(format!("{shown}：{e}"));
    } else {
        report.lines.push(format!("  ✓ 写入 {shown}"));
    }
}

fn write(path: &Path, content: &str, overwrite: bool) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if path.exists() && !overwrite {
        // Merge path: keep a one-shot backup before rewriting.
        let bak = path.with_extension(format!(
            "{}bak",
            path.extension().map(|e| format!("{}.", e.to_string_lossy())).unwrap_or_default()
        ));
        let _ = fs::copy(path, bak);
    }
    fs::write(path, content).map_err(|e| e.to_string())
}

/// Merge one JSON/Toml entry into an existing config file. Returns the new
/// content plus a human note, or `None` when nothing needs to change.
fn merge_content(
    path: &Path,
    m: &MergeSpec,
    current: Option<&str>,
    force: bool,
) -> Option<(String, String)> {
    let is_toml = path.extension().map(|e| e == "toml").unwrap_or(false);
    if is_toml {
        let existing = current.unwrap_or("");
        if existing.contains("[mcp_servers.ratsa]") && !force {
            return None;
        }
        let value = m.value.get("ratsa")?;
        let cmd = value.get("command")?.as_str()?;
        let args = value
            .get("args")
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| format!("\"{s}\""))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let block = format!(
            "\n[mcp_servers.ratsa]\ncommand = \"{cmd}\"\nargs = [{args}]\n\
             # RATSA-Harness：本地 Agent 侧的 RATSA.ai 接入（`{cmd} mcp`）\n"
        );
        let mut out = existing.to_string();
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&block);
        return Some((out, "追加 [mcp_servers.ratsa]".into()));
    }

    let mut doc: Value = current
        .and_then(|c| serde_json::from_str(c).ok())
        .unwrap_or_else(|| json!({}));
    if !doc.is_object() {
        doc = json!({});
    }
    let mut cursor = &mut doc;
    for part in &m.parent {
        let obj = cursor.as_object_mut()?;
        if !obj.contains_key(part) {
            obj.insert(part.clone(), json!({}));
        }
        cursor = obj.get_mut(part)?;
        if !cursor.is_object() {
            *cursor = json!({});
        }
    }
    let obj = cursor.as_object_mut()?;
    let note = if obj.contains_key(&m.key) {
        if !force && obj.get(&m.key) == Some(&m.value) {
            return None;
        }
        format!("更新 {} 条目", m.key)
    } else {
        format!("新增 {} 条目", m.key)
    };
    obj.insert(m.key.clone(), m.value.clone());
    let pretty = serde_json::to_string_pretty(&doc).ok()?;
    Some((format!("{pretty}\n"), note))
}

fn render(tpl: &str, env_info: &Env) -> String {
    let scope = if env_info.scopes.is_empty() {
        "（运行 `ratsa-harness whoami` 查看）".to_string()
    } else {
        env_info.scopes.join(", ")
    };
    let bound = if env_info.bound_slug.is_empty() {
        "（未绑定，key 以自身账号身份行动）".to_string()
    } else {
        format!("@{}（该 key 以该账号身份行动，发布/归属都算在该账号上）", env_info.bound_slug)
    };
    tpl.replace("{{BIN}}", &env_info.bin)
        .replace("{{BASE_URL}}", &env_info.base_url)
        .replace("{{SCOPES}}", &scope)
        .replace("{{BOUND}}", &bound)
        .replace("{{KEY_NAME}}", &env_info.key_name)
        .replace("{{VERSION}}", env!("CARGO_PKG_VERSION"))
}

fn home() -> PathBuf {
    env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

/// VS Code user-level prompt folder (where global `.instructions.md` live).
fn vscode_user_prompts() -> Option<PathBuf> {
    let home = home();
    let candidates = if cfg!(target_os = "macos") {
        vec![home.join("Library/Application Support/Code/User/prompts")]
    } else if cfg!(target_os = "windows") {
        env::var("APPDATA")
            .map(|a| vec![PathBuf::from(a).join("Code/User/prompts")])
            .unwrap_or_default()
    } else {
        vec![home.join(".config/Code/User/prompts")]
    };
    candidates
        .into_iter()
        .find(|p| p.exists())
        .or_else(|| {
            let fallback = home.join("Library/Application Support/Code/User/prompts");
            if cfg!(target_os = "macos") {
                Some(fallback)
            } else {
                None
            }
        })
}

fn current_bin() -> Result<String, String> {
    if let Ok(p) = env::var("RATSA_HARNESS_BIN") {
        return Ok(p);
    }
    env::current_exe()
        .map(|p| p.display().to_string())
        .map_err(|e| e.to_string())
}

fn display_path(p: &Path) -> String {
    let home = home();
    if let Ok(rest) = p.strip_prefix(&home) {
        return format!("~/{}", rest.display());
    }
    if let Ok(cwd) = env::current_dir() {
        if let Ok(rest) = p.strip_prefix(&cwd) {
            return rest.display().to_string();
        }
    }
    p.display().to_string()
}

fn indent(text: &str, n: usize) -> String {
    let pad = " ".repeat(n);
    text.lines()
        .map(|l| format!("{pad}{l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

// ------------------------------------------------------------------- assets

const SKILL_MD: &str = include_str!("../assets/SKILL.md");
const CURSOR_MDC: &str = include_str!("../assets/cursor.rules.mdc");
const COPILOT_MD: &str = include_str!("../assets/copilot-instructions.md");
const AGENTS_MD: &str = include_str!("../assets/AGENTS.md");
