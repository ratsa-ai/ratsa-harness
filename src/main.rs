//! RATSA-Harness — local, Agent-side entry point to the RATSA.ai platform.
//!
//! One binary, three jobs:
//!
//! 1. **接入** — `install` wires RATSA into whichever Agent you already run
//!    (Claude Code skill / MCP server / Cursor rules / VS Code Copilot /
//!    AGENTS.md / Codex MCP), so the Agent knows how to use the platform.
//! 2. **凭证** — `login` + `keys create --scopes … --bound-slug …` mint a
//!    *least-privilege* key: the permission table decides what it may do, and
//!    the bound handle decides whose account it acts as.
//! 3. **闭环** — `scopes` / `search` / `package` / `pull` / `report` /
//!    `feedback` run the discovery → pull → run-locally → report loop against
//!    the public HTTP API.
//!
//! Design rules: no daemon, no telemetry, no data plane — the CLI only pulls
//! artifacts and pushes structured results. Scanning happens locally.

mod api;
mod commands;
mod config;
mod install;
mod mcp;

use clap::{Args, Parser, Subcommand};

use config::Config;

#[derive(Parser, Debug)]
#[command(
    name = "ratsa",
    version,
    about = "ratsa —— RATSA-Harness CLI：把 RATSA.ai 接入你的 Agent（插件/Skill 安装 · 最小权限 key · Harness Package 与 SOF 文件 + 本地评测回传）",
    long_about = None,
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// 安装 / 更新各 Agent 的插件与 Skill（Claude Code、MCP、Cursor、Copilot、AGENTS.md、Codex）
    Install(InstallArgs),
    /// 列出支持的 Agent、各写哪些文件、是否注册 MCP
    Agents {
        /// 输出原始 JSON（供站点/脚本消费）
        #[arg(long)]
        json: bool,
    },
    /// 登录 RATSA 账号（换取会话，用于创建与管理 key）
    Login(LoginArgs),
    /// 退出登录（保留本地 API key）
    Logout,
    /// 查看当前身份、key 权限与归属绑定
    Whoami {
        /// 输出原始 JSON
        #[arg(long)]
        json: bool,
    },
    /// 打印 RATSA 权限表（可授予 key 的 scope 及含义）
    Scopes,
    /// 管理 API key / secret（权限表 + 归属绑定）
    Keys(KeysArgs),
    /// 本地凭证配置（base_url / key_id / secret）
    Config(ConfigArgs),
    /// 检索 SOF 目录
    Search {
        /// 关键字（名称 / 描述 / slug）
        query: Option<String>,
        /// 按类目过滤，如「核心零部件」
        #[arg(long)]
        category: Option<String>,
        /// 最多返回条数
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// 输出原始 JSON
        #[arg(long)]
        json: bool,
    },
    /// 读取一个 SOF 的完整信息
    Sof {
        /// SOF 的 slug（也可粘贴浏览器 URL）
        slug: String,
        /// 输出原始 JSON
        #[arg(long)]
        json: bool,
    },
    /// 读取设备 Harness manifest、就绪度与包清单
    Manifest {
        /// SOF 的 slug
        slug: String,
        /// 输出原始 JSON
        #[arg(long)]
        json: bool,
    },
    /// 只列出可拉取的 Harness Package
    Packages {
        /// SOF 的 slug
        slug: String,
        /// 输出原始 JSON
        #[arg(long)]
        json: bool,
    },
    /// 拉取 Harness Package（设备 Harness 包 / Eval Repo / 服务测试包）
    Pull {
        /// 设备或 Eval Repo / 服务的 slug（也可直接粘贴下载 URL）
        slug: String,
        /// 包类型
        #[arg(long, default_value = "device")]
        kind: String,
        /// 落盘目录（默认 ~/.ratsa/packages/<slug>/<kind>）
        #[arg(long)]
        out: Option<String>,
    },
    /// 让平台为该 SOF 生成设备 Harness 包并下载（= pull --kind device）
    Package {
        /// SOF 的 slug
        slug: String,
        /// 只输出描述（包内文件清单 / 就绪度 / 大小），不下载归档
        #[arg(long)]
        meta: bool,
        /// 落盘目录
        #[arg(long)]
        out: Option<String>,
    },
    /// 拉取 SOF 文件（规范化文档，可直接回灌 POST /api/sof）
    SofFile {
        /// SOF 的 slug
        slug: String,
        /// 落盘目录（默认 ~/.ratsa/sof）
        #[arg(long)]
        out: Option<String>,
    },
    /// 回传本地实测结果（Harness Eval，source=agent）
    Report {
        /// 被测设备的 SOF slug
        slug: String,
        /// 实测分数 0-100
        #[arg(long)]
        score: f64,
        /// 通过用例数
        #[arg(long)]
        passed: Option<i64>,
        /// 失败用例数
        #[arg(long)]
        failed: Option<i64>,
        /// 一句话结论
        #[arg(long, default_value = "")]
        summary: String,
        /// 补充说明
        #[arg(long, default_value = "")]
        notes: String,
    },
    /// 提交 Feedback 给厂商
    Feedback {
        /// SOF 的 slug
        slug: String,
        /// 标题
        #[arg(long)]
        title: String,
        /// 详细描述
        #[arg(long, default_value = "")]
        detail: String,
        /// bug | quality | feature | compatibility | support
        #[arg(long, default_value = "")]
        category: String,
        /// low | medium | high | critical
        #[arg(long, default_value = "")]
        severity: String,
    },
    /// 以 MCP stdio 服务器方式运行（供 Agent 作为工具调用）
    Mcp,
}

#[derive(Args, Debug)]
struct InstallArgs {
    /// 目标 Agent：claude | mcp | cursor | copilot | agents | codex | all
    #[arg(long, short, default_value = "all")]
    agent: String,
    /// 安装到用户目录（默认安装到当前项目）
    #[arg(long, short)]
    global: bool,
    /// 指定安装根目录
    #[arg(long)]
    dir: Option<String>,
    /// 只展示将要写入的内容，不落盘
    #[arg(long)]
    dry_run: bool,
    /// 覆盖已存在的文件
    #[arg(long, short)]
    force: bool,
}

#[derive(Args, Debug)]
struct LoginArgs {
    /// 账号邮箱
    #[arg(long, short)]
    email: String,
    /// 密码（省略则交互式输入）
    #[arg(long, short)]
    password: Option<String>,
    /// 服务端地址（默认取本地配置或 https://ratsa.ai）
    #[arg(long)]
    base_url: Option<String>,
}

#[derive(Args, Debug)]
struct KeysArgs {
    #[command(subcommand)]
    action: KeysAction,
}

#[derive(Subcommand, Debug)]
enum KeysAction {
    /// 列出账号下的 key（含权限与归属绑定）
    List {
        #[arg(long)]
        json: bool,
    },
    /// 创建一个 key：自选权限表条目，可选绑定到某个账号 handle
    Create {
        /// key 名称，便于识别用途（如 laptop-agent、ci）
        #[arg(long, default_value = "agent")]
        name: String,
        /// 权限列表，逗号分隔，可重复：--scopes sof:read,package:pull
        #[arg(long = "scopes", short = 's', value_delimiter = ',')]
        scopes: Vec<String>,
        /// 绑定到某个账号 handle：该 key 之后以该账号身份行动（归属与可操作范围都限定为该账号）
        #[arg(long = "bound-slug", alias = "bind")]
        bound_slug: Option<String>,
        /// 创建后立即设为本地当前凭证
        #[arg(long = "use")]
        use_it: bool,
    },
    /// 吊销一个 key
    Revoke {
        /// key 的数字 id 或 key_id
        id: String,
    },
}

#[derive(Args, Debug)]
struct ConfigArgs {
    #[command(subcommand)]
    action: ConfigAction,
}

#[derive(Subcommand, Debug)]
enum ConfigAction {
    /// 写入 / 更新本地凭证
    Set {
        #[arg(long)]
        base_url: Option<String>,
        #[arg(long)]
        key_id: Option<String>,
        #[arg(long)]
        secret: Option<String>,
        /// 记录归属绑定（仅本地展示用，实际绑定在创建 key 时生效）
        #[arg(long)]
        bound_slug: Option<String>,
    },
    /// 显示当前配置（secret 默认脱敏）
    Show {
        /// 显示完整 secret
        #[arg(long)]
        reveal: bool,
    },
    /// 清除本地 API key（保留会话）
    Clear,
}

fn main() {
    let cli = Cli::parse();
    let mut cfg = Config::load();
    let result = match cli.command {
        Command::Install(args) => {
            let opt = install::Options {
                agent: args.agent,
                dry_run: args.dry_run,
                global: args.global,
                force: args.force,
                dir: args.dir.map(std::path::PathBuf::from),
            };
            install::run(&cfg, &opt)
        }
        Command::Agents { json } => install::print_agents(json),
        Command::Login(args) => {
            let password = match args.password {
                Some(p) => p,
                None => match rpassword_fallback() {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("错误：{e}");
                        std::process::exit(1);
                    }
                },
            };
            commands::login(&mut cfg, &args.email, &password, args.base_url.as_deref())
        }
        Command::Logout => commands::logout(&mut cfg),
        Command::Whoami { json } => commands::whoami(&mut cfg, json),
        Command::Scopes => commands::scopes(&cfg),
        Command::Keys(args) => match args.action {
            KeysAction::List { json } => commands::keys_list(&cfg, json),
            KeysAction::Create {
                name,
                scopes,
                bound_slug,
                use_it,
            } => commands::keys_create(
                &mut cfg,
                &name,
                &scopes,
                bound_slug.as_deref().unwrap_or(""),
                use_it,
            ),
            KeysAction::Revoke { id } => commands::keys_revoke(&cfg, &id),
        },
        Command::Config(args) => match args.action {
            ConfigAction::Set {
                base_url,
                key_id,
                secret,
                bound_slug,
            } => config_set(&mut cfg, base_url, key_id, secret, bound_slug),
            ConfigAction::Show { reveal } => config_show(&cfg, reveal),
            ConfigAction::Clear => config_clear(&mut cfg),
        },
        Command::Search {
            query,
            category,
            limit,
            json,
        } => run_search(&cfg, query, category, limit, json),
        Command::Sof { slug, json } => run_sof(&cfg, &slug, json),
        Command::Manifest { slug, json } => run_manifest(&cfg, &slug, json),
        Command::Packages { slug, json } => run_packages(&cfg, &slug, json),
        Command::Pull { slug, kind, out } => {
            let client = api::Client::new(&cfg);
            let slug = api::slug_or_url(&slug);
            let out = out.unwrap_or_default();
            commands::pull_report(&client, &slug, &kind, &out).map(|s| print!("{s}"))
        }
        Command::Package { slug, meta, out } => {
            let client = api::Client::new(&cfg);
            let slug = api::slug_or_url(&slug);
            let out = out.unwrap_or_default();
            commands::package_cmd(&client, &slug, meta, &out).map(|s| print!("{s}"))
        }
        Command::SofFile { slug, out } => {
            let client = api::Client::new(&cfg);
            let slug = api::slug_or_url(&slug);
            let out = out.unwrap_or_default();
            commands::sof_file_cmd(&client, &slug, &out).map(|s| print!("{s}"))
        }
        Command::Report {
            slug,
            score,
            passed,
            failed,
            summary,
            notes,
        } => {
            let client = api::Client::new(&cfg);
            let slug = api::slug_or_url(&slug);
            commands::report_run(
                &client, &slug, score, passed, failed, &summary, &notes, "agent",
            )
            .map(|s| print!("{s}"))
        }
        Command::Feedback {
            slug,
            title,
            detail,
            category,
            severity,
        } => {
            let client = api::Client::new(&cfg);
            let slug = api::slug_or_url(&slug);
            commands::submit_feedback(&client, &slug, &title, &detail, &category, &severity)
                .map(|s| print!("{s}"))
        }
        Command::Mcp => mcp::serve(),
    };

    if let Err(e) = result {
        eprintln!("错误：{e}");
        std::process::exit(1);
    }
}

fn run_search(
    cfg: &Config,
    query: Option<String>,
    category: Option<String>,
    limit: usize,
    json: bool,
) -> Result<(), String> {
    let client = api::Client::new(cfg);
    let q = query.unwrap_or_default();
    let cat = category.unwrap_or_default();
    let rows = commands::filter_sofs(&client, &q, &cat, limit)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::Value::Array(rows)).unwrap_or_default()
        );
        return Ok(());
    }
    print!("{}", commands::format_sofs(&rows));
    Ok(())
}

fn run_sof(cfg: &Config, slug: &str, json: bool) -> Result<(), String> {
    let client = api::Client::new(cfg);
    let slug = api::slug_or_url(slug);
    let resp = client.get(&format!("/api/sof/{slug}"))?;
    if resp.status >= 400 {
        return Err(resp.error_message());
    }
    let v = resp.json();
    if json {
        println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
        return Ok(());
    }
    let item = v.get("item").cloned().unwrap_or(v.clone());
    let name = item.get("name").and_then(|x| x.as_str()).unwrap_or("-");
    let cat = item.get("category").and_then(|x| x.as_str()).unwrap_or("-");
    let desc = item.get("description").and_then(|x| x.as_str()).unwrap_or("");
    println!("{name}  [{cat}]");
    println!("slug      : {slug}");
    if !desc.is_empty() {
        println!("描述      : {desc}");
    }
    if let Some(ready) = item.get("agentic_ready").and_then(|x| x.as_bool()) {
        println!("agentic   : {}", if ready { "是" } else { "否（Harness 包为骨架）" });
    }
    if let Some(spec) = item.get("spec").and_then(|x| x.as_str()) {
        if !spec.is_empty() {
            println!("规格      : {}", api::truncate(spec, 400));
        }
    }
    println!();
    println!("下一步：ratsa-harness manifest {slug}   /   ratsa-harness pull {slug}");
    Ok(())
}

fn run_manifest(cfg: &Config, slug: &str, json: bool) -> Result<(), String> {
    let client = api::Client::new(cfg);
    let slug = api::slug_or_url(slug);
    let resp = client.get(&format!("/api/harness/sof/{slug}"))?;
    if resp.status >= 400 {
        return Err(resp.error_message());
    }
    let v = resp.json();
    if json {
        println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
        return Ok(());
    }
    if let Some(rd) = v.get("readiness") {
        let score = rd.get("score").map(|s| s.to_string()).unwrap_or_else(|| "-".into());
        let grade = rd.get("grade").and_then(|x| x.as_str()).unwrap_or("-");
        let valid = rd.get("valid").and_then(|x| x.as_bool()).unwrap_or(false);
        println!("就绪度    : {score} / {grade}{}", if valid { "" } else { "（存在校验问题）" });
        if let Some(issues) = rd.get("issues").and_then(|i| i.as_array()) {
            for it in issues {
                let msg = it
                    .get("message")
                    .or_else(|| it.get("code"))
                    .and_then(|x| x.as_str())
                    .unwrap_or("");
                if !msg.is_empty() {
                    println!("  ! {msg}");
                }
            }
        }
    }
    if let Some(exec) = v.get("execution") {
        let executed = exec.get("executed").and_then(|x| x.as_bool()).unwrap_or(false);
        if executed {
            let score = exec.get("score").map(|s| s.to_string()).unwrap_or_else(|| "-".into());
            let src = exec.get("source").and_then(|x| x.as_str()).unwrap_or("");
            println!("实测      : {score}{}", if src.is_empty() { String::new() } else { format!("（来源 {src}）") });
        } else {
            println!("实测      : 尚未有实测结果（可用 ratsa-harness report 回传）");
        }
    }
    println!();
    print!("{}", commands::format_packages(&v)?);
    Ok(())
}

fn run_packages(cfg: &Config, slug: &str, json: bool) -> Result<(), String> {
    let client = api::Client::new(cfg);
    let slug = api::slug_or_url(slug);
    let resp = client.get(&format!("/api/harness/sof/{slug}"))?;
    if resp.status >= 400 {
        return Err(resp.error_message());
    }
    let v = resp.json();
    if json {
        let pkgs = v.get("packages").cloned().unwrap_or(serde_json::json!([]));
        println!("{}", serde_json::to_string_pretty(&pkgs).unwrap_or_default());
        return Ok(());
    }
    print!("{}", commands::format_packages(&v)?);
    Ok(())
}

fn config_set(
    cfg: &mut Config,
    base_url: Option<String>,
    key_id: Option<String>,
    secret: Option<String>,
    bound_slug: Option<String>,
) -> Result<(), String> {
    if let Some(b) = base_url {
        cfg.base_url = b.trim_end_matches('/').to_string();
    }
    if let Some(k) = key_id {
        cfg.key_id = k.trim().to_string();
    }
    if let Some(s) = secret {
        cfg.secret = s.trim().to_string();
    }
    if let Some(bs) = bound_slug {
        let bs = api::normalize_slug(&bs);
        cfg.bound_slug = bs.clone();
        cfg.acting_as = bs;
    }
    if cfg.key_id.is_empty() != cfg.secret.is_empty() {
        return Err("key_id 与 secret 必须同时提供".into());
    }
    cfg.save()?;
    println!("已保存到 {}", config::config_path().display());
    println!("验证：ratsa-harness whoami");
    Ok(())
}

fn config_show(cfg: &Config, reveal: bool) -> Result<(), String> {
    let path = config::config_path();
    println!("配置文件 : {}", path.display());
    println!("base_url : {}", cfg.base());
    println!("key_id   : {}", if cfg.key_id.is_empty() { "-" } else { &cfg.key_id });
    println!(
        "secret   : {}",
        if cfg.secret.is_empty() {
            "-".to_string()
        } else if reveal {
            cfg.secret.clone()
        } else {
            mask(&cfg.secret)
        }
    );
    println!(
        "权限     : {}",
        if cfg.scopes.is_empty() { "-".to_string() } else { cfg.scopes.join(", ") }
    );
    let bound = if !cfg.acting_as.is_empty() {
        cfg.acting_as.clone()
    } else {
        cfg.bound_slug.clone()
    };
    println!("归属绑定 : {}", if bound.is_empty() { "-".to_string() } else { format!("@{bound}") });
    println!(
        "会话     : {}",
        if cfg.session_token.is_empty() {
            "未登录（无法创建 key：ratsa-harness login --email …）"
        } else {
            "已登录"
        }
    );
    if let Some(u) = cfg.user.as_ref() {
        println!("账号     : {} @{} {}", u.name, u.slug, u.plan);
    }
    Ok(())
}

fn config_clear(cfg: &mut Config) -> Result<(), String> {
    cfg.key_id.clear();
    cfg.secret.clear();
    cfg.scopes.clear();
    cfg.bound_slug.clear();
    cfg.acting_as.clear();
    cfg.key_name.clear();
    cfg.save()?;
    println!("已清除本地 API key 与权限缓存（会话保留）");
    Ok(())
}

fn mask(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= 8 {
        return "****".into();
    }
    let head: String = chars.iter().take(6).collect();
    let tail: String = chars.iter().skip(chars.len() - 4).collect();
    format!("{head}…{tail}")
}

/// Password prompt without an extra dependency: read from the TTY with echo on,
/// and prefer `RATSA_PASSWORD` for non-interactive use.
fn rpassword_fallback() -> Result<String, String> {
    if let Ok(p) = std::env::var("RATSA_PASSWORD") {
        if !p.is_empty() {
            return Ok(p);
        }
    }
    eprint!("密码（或设置 RATSA_PASSWORD 环境变量）：");
    use std::io::BufRead;
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    let p = line.trim_end_matches(['\n', '\r']).to_string();
    if p.is_empty() {
        return Err("密码为空".into());
    }
    Ok(p)
}
