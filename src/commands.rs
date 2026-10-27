//! Command implementations — the surface shared by the CLI and the MCP server.
//!
//! Keeping these in one module means the terminal experience and the Agent
//! experience can never drift apart: `ratsa-harness pull …` and the
//! `ratsa_pull_package` MCP tool run the exact same code.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::api::{self, Client, Headers};
use crate::config::{Config, UserInfo};

// ------------------------------------------------------------------ identity

pub fn cli_not_configured_hint() -> String {
    [
        "尚未配置 RATSA 凭证。",
        "",
        "方式一（推荐，可自选权限）：",
        "  ratsa-harness login --email you@example.com",
        "  ratsa-harness keys create --name agent --scopes sof:read,package:pull --bound-slug <你的账号handle> --use",
        "",
        "方式二（已有 key/secret）：",
        "  ratsa-harness config set --base-url https://ratsa.ai --key-id rtsk_xxx --secret rtsk_s_xxx",
        "",
        "方式三（环境变量，适合 CI / 容器）：",
        "  RATSA_KEY_ID=… RATSA_KEY_SECRET=… ratsa-harness whoami",
    ]
    .join("\n")
}

/// `whoami` — identity, permission table of the active key and the bound handle.
pub fn whoami(cfg: &mut Config, json_out: bool) -> Result<(), String> {
    if !cfg.has_key() && cfg.session_token.is_empty() {
        println!("{}", cli_not_configured_hint());
        return Ok(());
    }
    let client = Client::new(cfg);
    let path = if cfg.has_key() {
        "/api/v1/me"
    } else {
        "/api/me"
    };
    let resp = client.get(path)?;
    if resp.status >= 400 {
        return Err(resp.error_message());
    }
    let v = resp.json();
    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&v).unwrap_or_else(|_| resp.body.clone())
        );
        return Ok(());
    }

    if let Some(u) = v.get("user") {
        let name = u.get("name").and_then(|x| x.as_str()).unwrap_or("-");
        let slug = u.get("slug").and_then(|x| x.as_str()).unwrap_or("");
        let plan = u.get("plan").and_then(|x| x.as_str()).unwrap_or("");
        println!(
            "账号      : {name}{}",
            if slug.is_empty() {
                String::new()
            } else {
                format!("  @{slug}")
            }
        );
        if !plan.is_empty() {
            println!("套餐      : {plan}");
        }
    } else if let Some(u) = v.get("name") {
        println!("账号      : {}", u.as_str().unwrap_or("-"));
    }
    if let Some(k) = v.get("key") {
        let name = k.get("name").and_then(|x| x.as_str()).unwrap_or("");
        let kid = k.get("key_id").and_then(|x| x.as_str()).unwrap_or("");
        println!("key       : {name}  {kid}");
        let scopes: Vec<String> = k
            .get("scopes")
            .and_then(|s| s.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        println!(
            "权限      : {}",
            if scopes.is_empty() {
                "-".into()
            } else {
                scopes.join(", ")
            }
        );
        // Keep the local cache in sync so the installer can print it.
        cfg.scopes = scopes;
        cfg.key_name = name.to_string();
    }
    if let Some(a) = v.get("acting_as") {
        let slug = a.get("slug").and_then(|x| x.as_str()).unwrap_or("");
        let name = a.get("name").and_then(|x| x.as_str()).unwrap_or("");
        println!("归属绑定  : @{slug}（{name}）—— 以该账号身份行动");
        cfg.acting_as = slug.to_string();
    } else if !cfg.bound_slug.is_empty() {
        println!(
            "归属绑定  : @{}（未生效，请检查该 handle 是否存在）",
            cfg.bound_slug
        );
    }
    println!("服务端    : {}", cfg.base());
    let _ = cfg.save();
    Ok(())
}

/// `login` — exchange email/password for a JWT session (used to mint keys).
pub fn login(
    cfg: &mut Config,
    email: &str,
    password: &str,
    base_url: Option<&str>,
) -> Result<(), String> {
    if let Some(b) = base_url {
        cfg.base_url = b.trim_end_matches('/').to_string();
    }
    let client = Client::new(cfg);
    let resp = client.post(
        "/api/auth/login",
        &json!({"email": email, "password": password}),
    )?;
    if resp.status >= 400 {
        return Err(format!("登录失败：{}", resp.error_message()));
    }
    let v = resp.json();
    let token = v.get("token").and_then(|t| t.as_str()).unwrap_or("");
    if token.is_empty() {
        return Err("服务端未返回 token".into());
    }
    cfg.session_token = token.to_string();
    if let Some(u) = v.get("user") {
        cfg.user = Some(UserInfo {
            id: u.get("id").and_then(|x| x.as_u64()).unwrap_or(0),
            name: u.get("name").and_then(|x| x.as_str()).unwrap_or("").into(),
            slug: u.get("slug").and_then(|x| x.as_str()).unwrap_or("").into(),
            email: u.get("email").and_then(|x| x.as_str()).unwrap_or("").into(),
            plan: u.get("plan").and_then(|x| x.as_str()).unwrap_or("").into(),
        });
    }
    cfg.save()?;
    let who = cfg
        .user
        .as_ref()
        .map(|u| format!("{} @{}", u.name, u.slug))
        .unwrap_or_else(|| email.to_string());
    println!("已登录：{who}");
    println!("会话已保存到 {}", crate::config::config_path().display());
    if cfg.has_key() {
        println!("已有可用的 API key（{}），可直接使用。", cfg.key_name);
    } else {
        println!();
        println!("下一步：生成一个最小权限的 key 给 Agent 使用");
        println!("  ratsa-harness scopes                       # 查看权限表");
        println!("  ratsa-harness keys create --name agent --scopes sof:read,package:pull --use");
    }
    Ok(())
}

pub fn logout(cfg: &mut Config) -> Result<(), String> {
    cfg.session_token.clear();
    cfg.user = None;
    cfg.save()?;
    println!("已退出登录（本地 API key 保留，如需清除：ratsa-harness config clear）");
    Ok(())
}

// ------------------------------------------------------------------- scopes

/// Render the permission table (from `GET /api/meta` or `/api/v1/meta`).
pub fn format_scope_table(meta: &Value) -> String {
    let block = meta.get("api_key").cloned().unwrap_or(json!({}));
    let scopes = block
        .get("scopes")
        .and_then(|s| s.as_array())
        .cloned()
        .unwrap_or_default();
    let groups = block
        .get("groups")
        .and_then(|s| s.as_array())
        .cloned()
        .unwrap_or_default();
    let defaults: Vec<String> = block
        .get("defaults")
        .and_then(|s| s.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let mut out = String::new();
    out.push_str("RATSA 权限表（可授予 key 的 scope）\n\n");
    if scopes.is_empty() {
        out.push_str("（服务端未返回权限表，请确认 base_url）\n");
        return out;
    }
    // group -> rows, preserving the server's group order.
    let group_names: Vec<String> = if groups.is_empty() {
        let mut g: Vec<String> = scopes
            .iter()
            .filter_map(|s| s.get("group").and_then(|x| x.as_str()).map(String::from))
            .collect();
        g.dedup();
        g
    } else {
        groups
            .iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect()
    };
    for g in group_names {
        out.push_str(&format!("[{g}]\n"));
        for s in &scopes {
            if s.get("group").and_then(|x| x.as_str()) != Some(g.as_str()) {
                continue;
            }
            let id = s.get("id").and_then(|x| x.as_str()).unwrap_or("");
            let label = s.get("label").and_then(|x| x.as_str()).unwrap_or("");
            let desc = s.get("desc").and_then(|x| x.as_str()).unwrap_or("");
            let write = s.get("write").and_then(|x| x.as_bool()).unwrap_or(false);
            let admin = s
                .get("admin_only")
                .and_then(|x| x.as_bool())
                .unwrap_or(false);
            let mut flags = Vec::new();
            if write {
                flags.push("写");
            }
            if admin {
                flags.push("仅管理员");
            }
            let flag = if flags.is_empty() {
                String::new()
            } else {
                format!("（{}）", flags.join("/"))
            };
            out.push_str(&format!("  {id:<20} {label}{flag}\n"));
            if !desc.is_empty() {
                out.push_str(&format!("  {:<20} {desc}\n", ""));
            }
        }
        out.push('\n');
    }
    out.push_str(&format!("默认（不指定时）：{}\n", defaults.join(", ")));
    out.push_str("\n示例：ratsa-harness keys create --name agent --scopes sof:read,package:pull --bound-slug <handle>\n");
    out
}

pub fn scopes(cfg: &Config) -> Result<(), String> {
    let client = Client::new(cfg);
    let path = if cfg.has_key() {
        "/api/v1/meta"
    } else {
        "/api/meta"
    };
    let resp = client.get(path)?;
    if resp.status >= 400 {
        return Err(resp.error_message());
    }
    print!("{}", format_scope_table(&resp.json()));
    Ok(())
}

// --------------------------------------------------------------------- keys

pub fn keys_list(cfg: &Config, json_out: bool) -> Result<(), String> {
    let client = Client::new(cfg);
    let resp = client.get("/api/keys")?;
    if resp.status >= 400 {
        return Err(format!(
            "{}（需要先 `ratsa-harness login --email …`）",
            resp.error_message()
        ));
    }
    let v = resp.json();
    if json_out {
        println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
        return Ok(());
    }
    let items = v
        .get("items")
        .and_then(|i| i.as_array())
        .cloned()
        .unwrap_or_default();
    if items.is_empty() {
        println!("还没有 key。创建一个：ratsa-harness keys create --name agent --scopes sof:read,package:pull");
        return Ok(());
    }
    println!(
        "{:<18} {:<16} {:<26} {:<12} {}",
        "KEY_ID", "名称", "权限", "归属绑定", "状态"
    );
    for k in items {
        let kid = k.get("key_id").and_then(|x| x.as_str()).unwrap_or("");
        let name = k.get("name").and_then(|x| x.as_str()).unwrap_or("");
        let scopes: Vec<String> = k
            .get("scopes")
            .and_then(|s| s.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let bound = k
            .get("bound_user")
            .and_then(|b| b.get("slug"))
            .and_then(|x| x.as_str())
            .map(|s| format!("@{s}"))
            .or_else(|| {
                k.get("bound_slug")
                    .and_then(|x| x.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| format!("@{s}"))
            })
            .unwrap_or_else(|| "-".into());
        let active = k.get("active").and_then(|x| x.as_bool()).unwrap_or(false);
        println!(
            "{:<18} {:<16} {:<26} {:<12} {}",
            short(kid),
            name,
            scopes.join(","),
            bound,
            if active { "启用" } else { "停用" }
        );
    }
    Ok(())
}

/// `keys create` — the enforcement point of the permission table: pick scopes,
/// optionally bind the key to an account handle.
pub fn keys_create(
    cfg: &mut Config,
    name: &str,
    scopes_raw: &[String],
    bound_slug: &str,
    use_it: bool,
) -> Result<(), String> {
    let picked = api::parse_scopes(scopes_raw);
    let bound = api::normalize_slug(bound_slug);
    let mut body = json!({"name": name});
    if !picked.is_empty() {
        body["scopes"] = json!(picked);
    }
    if !bound.is_empty() {
        body["bound_slug"] = json!(bound);
    }
    let client = Client::new(cfg);
    let resp = client.post("/api/keys", &body)?;
    if resp.status >= 400 {
        let hint = match resp.json().get("code").and_then(|c| c.as_str()) {
            Some("scope_forbidden") => "（admin 权限只能由管理员授予）",
            Some("bad_bound_slug") => "（绑定的账号 handle 不存在）",
            Some("bound_slug_not_allowed") => {
                "（只能绑定自己的 handle；绑定他人账号需要对方/管理员授权）"
            }
            Some("unauthorized") | Some("login_required") => "（需要先 `ratsa-harness login`）",
            _ => "",
        };
        return Err(format!("创建失败：{}{hint}", resp.error_message()));
    }
    let v = resp.json();
    let key = v.get("key").cloned().unwrap_or(json!({}));
    let key_id = key.get("key_id").and_then(|x| x.as_str()).unwrap_or("");
    let secret = v.get("secret").and_then(|x| x.as_str()).unwrap_or("");
    let eff: Vec<String> = key
        .get("scopes")
        .and_then(|s| s.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    println!("已创建 key：{name}");
    println!("  key_id : {key_id}");
    println!("  权限   : {}", eff.join(", "));
    if !bound.is_empty() {
        println!("  归属   : @{bound}（该 key 以该账号身份行动）");
    }
    println!();
    println!("secret（仅显示一次，请立即保存）：");
    println!("  {secret}");

    if use_it {
        cfg.key_id = key_id.to_string();
        cfg.secret = secret.to_string();
        cfg.scopes = eff.clone();
        cfg.key_name = name.to_string();
        cfg.bound_slug = bound.clone();
        cfg.acting_as = bound.clone();
        cfg.save()?;
        println!();
        println!(
            "已设为当前凭证（保存在 {}）",
            crate::config::config_path().display()
        );
    } else {
        println!();
        println!("应用为当前凭证：ratsa-harness config set --key-id {key_id} --secret <secret>");
    }
    Ok(())
}

pub fn keys_revoke(cfg: &Config, id: &str) -> Result<(), String> {
    let client = Client::new(cfg);
    let resp = client.delete(&format!("/api/keys/{id}"))?;
    if resp.status >= 400 {
        return Err(resp.error_message());
    }
    println!("已吊销 key {id}");
    Ok(())
}

// ------------------------------------------------------------------ catalog

/// Search SOFs across vendors.
///
/// Deliberately uses the **public** catalog (`/api/sof`): `/api/v1/sof` is the
/// acting account's *own* catalog (publisher view), while discovery of other
/// vendors' devices is exactly what an Agent needs. The public catalog also
/// carries the owning `user`, which is how the vendor column is filled in.
pub fn filter_sofs(
    client: &Client,
    query: &str,
    category: &str,
    limit: usize,
) -> Result<Vec<Value>, String> {
    let resp = client.get("/api/sof")?;
    if resp.status >= 400 {
        return Err(resp.error_message());
    }
    let v = resp.json();
    let items = v
        .get("items")
        .and_then(|i| i.as_array())
        .cloned()
        .unwrap_or_default();
    let q = query.to_lowercase();
    let mut rows: Vec<Value> = items
        .into_iter()
        .filter(|it| {
            if !q.is_empty() {
                let hay = format!(
                    "{} {} {}",
                    it.get("name").and_then(|x| x.as_str()).unwrap_or(""),
                    it.get("description").and_then(|x| x.as_str()).unwrap_or(""),
                    it.get("slug").and_then(|x| x.as_str()).unwrap_or("")
                )
                .to_lowercase();
                if !hay.contains(&q) {
                    return false;
                }
            }
            if !category.is_empty() {
                let cat = it.get("category").and_then(|x| x.as_str()).unwrap_or("");
                if cat != category {
                    return false;
                }
            }
            true
        })
        .collect();
    rows.truncate(if limit == 0 { 20 } else { limit });
    Ok(rows)
}

/// Table view used by the CLI and the MCP text result.
pub fn format_sofs(rows: &[Value]) -> String {
    let mut out = String::new();
    if rows.is_empty() {
        out.push_str("没有匹配的 SOF。\n");
        return out;
    }
    out.push_str(&format!(
        "{:<14} {:<24} {:<12} {:<10} {:<10} {}\n",
        "SLUG", "名称", "类目", "版本", "就绪度", "厂商"
    ));
    for it in rows {
        let slug = it.get("slug").and_then(|x| x.as_str()).unwrap_or("");
        let name = it.get("name").and_then(|x| x.as_str()).unwrap_or("");
        let cat = it.get("category").and_then(|x| x.as_str()).unwrap_or("");
        let ver = it.get("version").and_then(|x| x.as_str()).unwrap_or("-");
        let ready = it
            .get("agentic_ready")
            .and_then(|x| x.as_bool())
            .map(|b| if b { "agentic" } else { "scaffold" })
            .unwrap_or("-");
        let vendor = it
            .get("user")
            .and_then(|u| u.get("slug"))
            .and_then(|x| x.as_str())
            .map(|s| format!("@{s}"))
            .unwrap_or_else(|| "-".into());
        out.push_str(&format!(
            "{:<14} {:<24} {:<12} {:<10} {:<10} {}\n",
            slug,
            truncate_cells(name, 22),
            truncate_cells(cat, 10),
            truncate_cells(ver, 8),
            ready,
            vendor
        ));
    }
    out
}

pub fn list_sofs(
    client: &Client,
    query: &str,
    category: &str,
    limit: usize,
) -> Result<String, String> {
    let rows = filter_sofs(client, query, category, limit)?;
    Ok(format_sofs(&rows))
}

/// Render the unified `packages[]` of a device manifest.
pub fn format_packages(manifest: &Value) -> Result<String, String> {
    let pkgs = manifest
        .get("packages")
        .and_then(|p| p.as_array())
        .cloned()
        .unwrap_or_default();
    let slug = manifest
        .get("slug")
        .or_else(|| manifest.get("sof").and_then(|s| s.get("slug")))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let sof_ver = manifest
        .pointer("/sof/version")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    if pkgs.is_empty() {
        return Ok(format!(
            "设备 {slug} 目前没有可拉取的 Harness Package。\n（厂商尚未发布 Harness / Eval Repo / 服务测试包，或可见性未对你开放）"
        ));
    }
    let mut out = format!(
        "设备 {slug}{} 的 Harness Package（{} 个）\n\n",
        if sof_ver.is_empty() {
            String::new()
        } else {
            format!(" @ {sof_ver}")
        },
        pkgs.len()
    );
    for p in pkgs {
        let kind = p.get("package_kind").and_then(|x| x.as_str()).unwrap_or("");
        let name = p.get("name").and_then(|x| x.as_str()).unwrap_or("");
        let id = p.get("id").and_then(|x| x.as_str()).unwrap_or("");
        let ver = p.get("version").and_then(|x| x.as_str()).unwrap_or("");
        let pkg_slug = p.get("slug").and_then(|x| x.as_str()).unwrap_or("");
        let sof_ver = p.get("sof_version").and_then(|x| x.as_str()).unwrap_or("");
        let url = p.get("download_url").and_then(|x| x.as_str()).unwrap_or("");
        let by = p
            .get("provider")
            .and_then(|x| x.as_str())
            .or_else(|| p.get("by").and_then(|x| x.as_str()))
            .unwrap_or("");
        // The pull flag follows the family kind: the same device can carry a
        // platform-derived harness, a third-party eval repo and a service pack.
        let flag = match kind {
            "eval-repo" => "--kind eval-repo",
            "eval-service-harness" => "--kind service",
            _ => "--kind device",
        };
        let pull_slug = if url.is_empty() { id } else { url };
        out.push_str(&format!("• [{kind}] {name}\n"));
        out.push_str(&format!("    id  : {id}\n"));
        if !ver.is_empty() {
            out.push_str(&format!("    版本: {ver}\n"));
        }
        if !by.is_empty() {
            out.push_str(&format!("    厂商: {by}\n"));
        }
        if !pull_slug.is_empty() {
            let version_flag = if kind == "device-harness" && !sof_ver.is_empty() {
                format!(" --version {sof_ver}")
            } else {
                String::new()
            };
            let target = if !pkg_slug.is_empty() {
                pkg_slug
            } else {
                pull_slug
            };
            out.push_str(&format!(
                "    拉取: ratsa-harness pull {target} {flag}{version_flag}\n"
            ));
        }
    }
    Ok(out)
}

/// Append an optional `?version=` pin to a platform path.
pub fn with_version(path: &str, version: &str) -> String {
    let v = version.trim();
    if v.is_empty() {
        return path.to_string();
    }
    let sep = if path.contains('?') { '&' } else { '?' };
    format!("{path}{sep}version={}", urlencode(v))
}

/// Render a SOF's version history.
pub fn format_versions(v: &Value) -> String {
    let current = v.get("current").and_then(|x| x.as_str()).unwrap_or("");
    let items = v
        .get("items")
        .and_then(|i| i.as_array())
        .cloned()
        .unwrap_or_default();
    let mut out = String::new();
    if items.is_empty() {
        out.push_str("该 SOF 还没有版本记录。\n");
        return out;
    }
    out.push_str(&format!("版本历史（当前 {current}）\n\n"));
    out.push_str(&format!("{:<12} {:<12} {}\n", "VERSION", "时间", "说明"));
    for it in items {
        let ver = it.get("version").and_then(|x| x.as_str()).unwrap_or("-");
        let at = it
            .get("created_at")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .chars()
            .take(10)
            .collect::<String>();
        let note = it.get("changelog").and_then(|x| x.as_str()).unwrap_or("");
        let mark = if ver == current { "*" } else { " " };
        out.push_str(&format!(
            "{mark}{:<11} {:<12} {}\n",
            ver,
            at,
            if note.is_empty() { "-" } else { note }
        ));
    }
    out
}

// --------------------------------------------------------------------- pull

/// Download a Harness Package into `~/.ratsa/packages/<slug>/` (or `out_dir`).
pub fn pull_report(
    client: &Client,
    slug: &str,
    kind: &str,
    version: &str,
    out_dir: &str,
) -> Result<String, String> {
    let kind = match kind {
        "" => "device",
        k @ ("device" | "eval-repo" | "service") => k,
        other => {
            return Err(format!(
                "未知 kind「{other}」，可选 device | eval-repo | service"
            ))
        }
    };
    let path = match kind {
        "device" => with_version(&format!("/api/harness/sof/{slug}/package"), version),
        "eval-repo" => format!("/api/eval-repo/{slug}/download"),
        _ => format!("/api/eval-service/{slug}/harness/download"),
    };
    let dir = if out_dir.is_empty() {
        let mut d = crate::config::home_dir().join("packages").join(slug);
        d.push(kind);
        d
    } else {
        PathBuf::from(out_dir)
    };
    let default_name = format!("{slug}-{kind}.zip");
    let (file, size, headers) = client.download_to(&path, &dir, &default_name)?;
    verify_checksum(&std::fs::read(&file).map_err(|e| e.to_string())?, &headers)?;

    let mut out = String::new();
    out.push_str(&format!("已拉取（{kind}）\n"));
    out.push_str(&format!("  来源  : {}{path}\n", client.base));
    out.push_str(&format!("  落盘  : {}\n", file.display()));
    out.push_str(&format!("  大小  : {size} 字节\n"));
    append_artifact_meta(&mut out, &headers);
    out.push_str(&format!(
        "  解包  : unzip -o {} -d {}\n",
        file.display(),
        dir.display()
    ));
    Ok(out)
}

fn append_artifact_meta(out: &mut String, headers: &Headers) {
    if !headers.checksum.is_empty() {
        out.push_str(&format!(
            "  校验  : sha256 {}（已本地复算通过）\n",
            headers.checksum
        ));
    }
    if !headers.level.is_empty() {
        let level = match headers.level.as_str() {
            "ready" => "ready（manifest 可派生且校验通过）",
            "warn" => "warn（可派生但有校验问题）",
            _ => "scaffold（SOF 还不是 agentic，包内是待填骨架）",
        };
        out.push_str(&format!("  级别  : {level}\n"));
    }
}

/// `ratsa package` —— 平台官方为该 SOF **生成** 设备 Harness 包并下载。
///
/// `--meta` 先只看描述（包内容清单 / 就绪度 / 文件数，不下载归档），
/// 再把包落到本地；这就是 Harness Package 家族里的 `device-harness`。
pub fn package_cmd(
    client: &Client,
    slug: &str,
    version: &str,
    meta_only: bool,
    out_dir: &str,
) -> Result<String, String> {
    let path = with_version(&format!("/api/harness/sof/{slug}/package"), version);
    if !meta_only {
        let out = pull_report(client, slug, "device", version, out_dir)?;
        return Ok(format!(
            "{out}\n提示：SOF 文件（可回灌的规范文档）用 `ratsa sof-file {slug}` 单独拉取。\n"
        ));
    }
    let meta_path = format!("{path}{}meta=1", if path.contains('?') { '&' } else { '?' });
    let resp = client.get(&meta_path)?;
    if resp.status >= 400 {
        return Err(resp.error_message());
    }
    let v = resp.json();
    let pkg = v.get("package").cloned().unwrap_or(json!({}));
    let rd = v.get("readiness").cloned().unwrap_or(json!({}));
    let mut out = String::new();
    out.push_str(&format!(
        "Harness Package（{}）· 设备 {slug}\n",
        v.get("package_kind")
            .and_then(|x| x.as_str())
            .unwrap_or("device-harness")
    ));
    let id = pkg.get("id").and_then(|x| x.as_str()).unwrap_or("");
    let ver = pkg.get("version").and_then(|x| x.as_str()).unwrap_or("");
    if !id.is_empty() {
        out.push_str(&format!("  id/version : {id} @ {ver}\n"));
    }
    if let Some(sof_ver) = pkg.pointer("/sof/version").and_then(|x| x.as_str()) {
        if !sof_ver.is_empty() {
            out.push_str(&format!("  SOF 版本   : {sof_ver}\n"));
        }
    }
    let score = rd
        .get("score")
        .map(|s| s.to_string())
        .unwrap_or_else(|| "-".into());
    let grade = rd.get("grade").and_then(|x| x.as_str()).unwrap_or("-");
    out.push_str(&format!("  就绪度     : {score} / {grade}\n"));
    if let Some(n) = v.get("file_count") {
        out.push_str(&format!("  文件数     : {n}\n"));
    }
    if let Some(b) = v.get("bytes") {
        out.push_str(&format!("  归档大小   : {b} 字节\n"));
    }
    if let Some(files) = pkg.get("files").and_then(|x| x.as_array()) {
        out.push_str("  包内文件   :\n");
        for f in files {
            let p = f
                .get("path")
                .or_else(|| f.get("name"))
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let note = f.get("note").and_then(|x| x.as_str()).unwrap_or("");
            if note.is_empty() {
                out.push_str(&format!("    - {p}\n"));
            } else {
                out.push_str(&format!("    - {p}  {note}\n"));
            }
        }
    }
    if let Some(todo) = pkg.get("todo").and_then(|t| t.as_array()) {
        if !todo.is_empty() {
            out.push_str("  待填（scaffold）：\n");
            for t in todo {
                if let Some(s) = t.as_str() {
                    out.push_str(&format!("    ! {s}\n"));
                }
            }
        }
    }
    out.push_str(&format!("\n下载：ratsa package {slug}\n"));
    Ok(out)
}

/// `ratsa sof-file` —— 拉取 SOF 文件（规范化、可回灌的文档）。
pub fn sof_file_cmd(
    client: &Client,
    slug: &str,
    version: &str,
    out_dir: &str,
) -> Result<String, String> {
    let dir = if out_dir.is_empty() {
        crate::config::home_dir().join("sof")
    } else {
        PathBuf::from(out_dir)
    };
    let (file, size, _h) = client.download_to(
        &with_version(&format!("/api/sof/{slug}/file"), version),
        &dir,
        &format!("{slug}.sof.json"),
    )?;
    let mut out = format!("已拉取 SOF 文件（{slug}）\n");
    out.push_str(&format!("  落盘 : {}\n", file.display()));
    out.push_str(&format!("  大小 : {size} 字节\n"));
    out.push_str("  内容 : kind=ratsa.sof.json，`component` 与 `POST/PUT /api/sof` 同构\n");
    out.push_str(&format!(
        "  回灌 : ratsa sof-push {slug}.sof.json   （改动后重传；需 sof:write 权限）\n"
    ));
    Ok(out)
}

// ---------------------------------------------------------------- SOF push

/// 把一份 SOF 文件回灌到平台（新建或更新）。
///
/// 输入就是 `sof-file` 拉下来的那份 —— `kind=ratsa.sof.json`，`component` 块
/// 与 `POST/PUT /api/sof` 同构。不传 `slug` 走 POST（新建），传了走 PUT（更新该 slug）。
///
/// ⚠️ **四个字段要序列化成字符串**：`spec` / `certification` / `skills` /
/// `agentic_spec` 在库里是 TEXT 列，而 SOF 文件里它们是**展开的 JSON**
/// （为了可读）。不转的话服务端按 JSON 对象收，落库就成 `map[...]` 而不是原文。
/// 这与包内 `tools/import-sof.py` 的处理是同一件事。
///
/// 端点是 `/api/v1/sof`（key 路径），需要该 key 带 `sof:write`。
pub fn sof_push_cmd(
    client: &Client,
    file: &str,
    slug: Option<&str>,
    dry_run: bool,
) -> Result<String, String> {
    let path = PathBuf::from(file);
    let raw =
        std::fs::read_to_string(&path).map_err(|e| format!("读不了 {}：{e}", path.display()))?;
    let doc: Value =
        serde_json::from_str(&raw).map_err(|e| format!("{} 不是合法 JSON：{e}", path.display()))?;

    let comp = doc
        .get("component")
        .ok_or("文件里没有 `component` 块（应为 kind=ratsa.sof.json）")?;
    if !comp.is_object() {
        return Err("`component` 不是对象".into());
    }

    // 展开的 JSON 字段 -> 字符串；已经是字符串的原样保留
    let mut body = comp.clone();
    // 版本是平台生成的历史快照标识；回灌会由平台自动递增一个新版本。
    if let Some(obj) = body.as_object_mut() {
        obj.remove("version");
    }
    let mut coerced: Vec<&str> = Vec::new();
    for f in ["spec", "certification", "skills", "agentic_spec"] {
        let Some(v) = comp.get(f) else { continue };
        if v.is_null() {
            continue;
        }
        if v.is_string() {
            continue;
        }
        let s = serde_json::to_string(v).map_err(|e| format!("{f} 序列化失败：{e}"))?;
        body[f] = Value::String(s);
        coerced.push(f);
    }

    let name = comp
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("（未填）");
    let category = comp
        .get("category")
        .and_then(|v| v.as_str())
        .unwrap_or("（未填）");

    if dry_run {
        let mut out = format!("（dry-run，未提交）\n 文件 : {}\n", path.display());
        out.push_str(&format!(" 名称 : {name}\n 类别 : {category}\n"));
        out.push_str(&format!(
            " 动作 : {}\n",
            match slug {
                Some(s) => format!("更新 PUT /api/v1/sof/{s}"),
                None => "新建 POST /api/v1/sof".into(),
            }
        ));
        if !coerced.is_empty() {
            out.push_str(&format!(" 转字符串 : {}\n", coerced.join(", ")));
        }
        out.push_str(&format!(" 体积 : {} 字节\n", body.to_string().len()));
        return Ok(out);
    }

    let resp = match slug {
        Some(s) => client.put(&format!("/api/v1/sof/{s}"), &body)?,
        None => client.post("/api/v1/sof", &body)?,
    };
    if resp.status >= 400 {
        return Err(format!(
            "{}失败（HTTP {}）：{}",
            if slug.is_some() { "更新" } else { "创建" },
            resp.status,
            resp.error_message()
        ));
    }

    let v: Value = serde_json::from_str(&resp.body).unwrap_or(json!({}));
    let item = v.get("item").cloned().unwrap_or(json!({}));
    let got = item.get("slug").and_then(|x| x.as_str()).unwrap_or("");
    let got_name = item.get("name").and_then(|x| x.as_str()).unwrap_or(name);

    let mut out = format!("{}成功\n", if slug.is_some() { "更新" } else { "创建" });
    out.push_str(&format!(" 名称 : {got_name}\n"));
    if !got.is_empty() {
        out.push_str(&format!(" slug : {got}\n"));
        out.push_str(&format!(" 链接 : https://ratsa.ai/sof/{got}\n"));
    }
    if !coerced.is_empty() {
        out.push_str(&format!(
            " 转换 : {} 已序列化为字符串\n",
            coerced.join(", ")
        ));
    }
    if slug.is_none() && !got.is_empty() {
        out.push_str(" 提示 : 记住这个 slug —— 重传要用它（`sof-push <file> --slug ");
        out.push_str(got);
        out.push_str("`）\n");
    }
    Ok(out)
}

// ---------------------------------------------------------------- SOF upload

/// 平台未声明上限时的兜底值。**正常情况下应以上限为准从 `/api/meta` 读**，
/// 免得改上限要重发客户端。
const DEFAULT_MAX_UPLOAD_BYTES: i64 = 50 << 20;

/// `assets/` 子目录 -> 资源类型。与 `sof/README.md` 的分层表一致。
///
/// `images/` **刻意不在表里**：产品图走 `POST /api/sof/{slug}/images`，
/// 不进 resources（它们是封面图，不是可下载的附件）。
fn resource_type_for_dir(dir: &str) -> Option<&'static str> {
    match dir {
        "docs" => Some("document"),
        "drawings" => Some("drawing"),
        "tools" => Some("devtool"),
        "firmware" => Some("firmware"),
        _ => None,
    }
}

/// 带类型的待上传文件。
struct Pending {
    path: std::path::PathBuf,
    name: String,
    size: i64,
    r#type: String,
}

/// 收文件。`from_assets` 时按 `assets/<子目录>/` 判类型，否则一律用 `fallback_type`。
fn collect_files(
    paths: &[String],
    from_assets: bool,
    fallback_type: &str,
) -> Result<Vec<Pending>, String> {
    let mut out = Vec::new();
    for p in paths {
        let base = std::path::PathBuf::from(p);
        if base.is_file() {
            let size = std::fs::metadata(&base)
                .map(|m| m.len() as i64)
                .unwrap_or(0);
            out.push(Pending {
                name: base
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
                path: base,
                size,
                r#type: fallback_type.to_string(),
            });
            continue;
        }
        if !base.is_dir() {
            return Err(format!("路径不存在：{p}"));
        }
        // 目录：--from-assets 时只收 assets/<已知子目录>/，其余一律不进
        // （避免把 sop.json / README.md / _incoming 里的大件也传上去）。
        let root = if from_assets {
            base.join("assets")
        } else {
            base.clone()
        };
        if from_assets && !root.is_dir() {
            return Err(format!("{} 下没有 assets/ 目录", base.display()));
        }
        let mut stack = vec![root.clone()];
        while let Some(d) = stack.pop() {
            let entries =
                std::fs::read_dir(&d).map_err(|e| format!("读不了 {}：{e}", d.display()))?;
            for e in entries.flatten() {
                let path = e.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let rel_dir = path
                    .parent()
                    .and_then(|x| x.strip_prefix(&root).ok())
                    .and_then(|x| x.components().next())
                    .map(|c| c.as_os_str().to_string_lossy().to_string())
                    .unwrap_or_default();
                let ty = if from_assets {
                    match resource_type_for_dir(&rel_dir) {
                        Some(t) => t.to_string(),
                        None => continue, // images/ 等不进 resources
                    }
                } else {
                    fallback_type.to_string()
                };
                let size = std::fs::metadata(&path)
                    .map(|m| m.len() as i64)
                    .unwrap_or(0);
                out.push(Pending {
                    name: path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string(),
                    path,
                    size,
                    r#type: ty,
                });
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// 从 `/api/meta` 问出本部署的上传上限；拿不到则回退内置默认并**如实说明**。
fn resolve_max_bytes(client: &Client) -> (i64, bool) {
    let v = client
        .get("/api/meta")
        .map(|r| r.json())
        .unwrap_or(Value::Null);
    match v.pointer("/upload/max_bytes").and_then(|x| x.as_i64()) {
        Some(n) if n > 0 => (n, false),
        _ => (DEFAULT_MAX_UPLOAD_BYTES, true),
    }
}

pub fn sof_upload_cmd(
    client: &Client,
    slug: &str,
    paths: &[String],
    from_assets: bool,
    type_opt: Option<&str>,
    dry_run: bool,
    json_out: bool,
) -> Result<String, String> {
    if paths.is_empty() {
        return Err("请给出要上传的文件或目录".into());
    }
    // **先在本地判断大小**，在读 meta 之前 —— 上限本身要问服务端，但过滤不能等。
    let (max_bytes, used_default) = resolve_max_bytes(client);

    let fallback = type_opt.unwrap_or("other");
    let all = collect_files(paths, from_assets, fallback)?;
    if all.is_empty() {
        return Err(
            "没有找到可上传的文件（--from-assets 只收 assets/{docs,drawings,tools,firmware}/）"
                .into(),
        );
    }

    // ★ 本地过滤：超限文件**一个字节都不发**，直接计入跳过。
    let mut sendable: Vec<Pending> = Vec::new();
    let mut skipped: Vec<(String, i64)> = Vec::new();
    for f in all {
        if f.size > max_bytes {
            skipped.push((f.name, f.size));
        } else {
            sendable.push(f);
        }
    }

    if dry_run {
        let mut s = String::from("（dry-run，未请求票据、未上传）\n");
        s.push_str(&format!(
            " 上限 : {} 字节（{:.1} MiB）",
            max_bytes,
            max_bytes as f64 / 1048576.0
        ));
        s.push_str(if used_default {
            "  ← 服务端未声明，用的是本地默认值\n"
        } else {
            "\n"
        });
        s.push_str(&format!(" 将上传 {} 个：\n", sendable.len()));
        for f in &sendable {
            s.push_str(&format!(
                "   ✓ {:>8} B  [{}] {}\n",
                f.size, f.r#type, f.name
            ));
        }
        if !skipped.is_empty() {
            s.push_str(&format!(" 跳过 {} 个（超限）：\n", skipped.len()));
            for (n, sz) in &skipped {
                s.push_str(&format!("   ✗ {n}（{sz} > {max_bytes}）\n"));
            }
            s.push_str(" 超限的文件请自行上传到别处，再用资源外链提交。\n");
        }
        return Ok(s);
    }
    if sendable.is_empty() {
        return Err(format!(
            "{} 个文件全部超过上限（{} 字节），没有可上传的",
            skipped.len(),
            max_bytes
        ));
    }

    // —— 取票据（一次覆盖整批）——
    let files: Vec<Value> = sendable
        .iter()
        .map(|f| json!({"name": f.name, "size": f.size, "type": f.r#type}))
        .collect();
    let resp = client.post(
        &format!("/api/v1/sof/{slug}/upload-ticket"),
        &json!({"files": files}),
    )?;
    if resp.status >= 400 {
        return Err(format!("取上传票据失败：{}", resp.error_message()));
    }
    let body = resp.json();
    let t = &body["ticket"];
    let ticket = crate::obs::Ticket {
        endpoint: t["endpoint"].as_str().unwrap_or("").to_string(),
        region: t["region"].as_str().unwrap_or("").to_string(),
        bucket: t["bucket"].as_str().unwrap_or("").to_string(),
        path_style: t["path_style"].as_bool().unwrap_or(false),
        access_key: t["access_key"].as_str().unwrap_or("").to_string(),
        secret_key: t["secret_key"].as_str().unwrap_or("").to_string(),
        security_token: t["security_token"].as_str().unwrap_or("").to_string(),
    };
    if ticket.access_key.is_empty() || ticket.security_token.is_empty() {
        return Err("票据里没有凭证（服务端未开启直传？）".into());
    }

    // 服务端为每个文件定了 key 与 url —— **按名字对齐**，
    // 客户端不自己拼 URL，也不自己定 key。
    let objects: Vec<crate::obs::Object> = body["objects"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|o| crate::obs::Object {
                    name: o["name"].as_str().unwrap_or("").to_string(),
                    key: o["key"].as_str().unwrap_or("").to_string(),
                    url: o["url"].as_str().unwrap_or("").to_string(),
                    content_type: o["content_type"]
                        .as_str()
                        .unwrap_or("application/octet-stream")
                        .to_string(),
                })
                .collect()
        })
        .unwrap_or_default();

    // —— 逐个 PUT 并回填 ——
    let mut uploaded: Vec<Value> = Vec::new();
    let mut failed: Vec<(String, String)> = Vec::new();
    for f in &sendable {
        let Some(obj) = objects.iter().find(|o| o.name == f.name) else {
            failed.push((f.name.clone(), "服务端没给这个文件的落点".into()));
            continue;
        };
        let bytes = match std::fs::read(&f.path) {
            Ok(b) => b,
            Err(e) => {
                failed.push((f.name.clone(), format!("读不了：{e}")));
                continue;
            }
        };
        match crate::obs::put_object(&ticket, &obj.key, &bytes, &obj.content_type) {
            Ok(_) => uploaded.push(json!({
                // url **原样用服务端给的**，不自己拼 —— 两边拼法不一致是这类功能的经典 bug 源
                "type": f.r#type, "name": f.name, "url": obj.url,
                "size_bytes": f.size, "checksum": sha256_hex(&bytes),
            })),
            Err(e) => failed.push((f.name.clone(), e)),
        }
    }

    // —— 回填 resources ——
    // 走独立的 resources 端点，**不要走 PUT /api/sof/:id**：那条是整体更新，
    // 其中 Description 是无条件覆盖的，会把描述清空。
    let mut wrote = false;
    if !uploaded.is_empty() {
        let r = client.post(
            &format!("/api/v1/sof/{slug}/resources"),
            &json!({"resources": uploaded}),
        )?;
        if r.status >= 400 {
            return Err(format!(
                "文件已上传成功（{} 个），但回填资源失败：{} —— \
                 请用 `ratsa sof-file {slug}` 拿到当前资源后手工补 url",
                uploaded.len(),
                r.error_message()
            ));
        }
        wrote = true;
    }

    if json_out {
        return Ok(serde_json::to_string_pretty(&json!({
            "uploaded": uploaded, "failed": failed.iter().map(|(n, e)| json!({"name": n, "error": e})).collect::<Vec<_>>(),
            "skipped": skipped.iter().map(|(n, s)| json!({"name": n, "size": s})).collect::<Vec<_>>(),
            "resources_written": wrote,
        }))
        .unwrap_or_default());
    }

    let mut s = String::new();
    for u in &uploaded {
        s.push_str(&format!(
            " ✓ {} → {}\n",
            u["name"].as_str().unwrap_or(""),
            u["url"].as_str().unwrap_or("")
        ));
    }
    for (n, e) in &failed {
        s.push_str(&format!(" ✗ {n}：{e}\n"));
    }
    for (n, sz) in &skipped {
        s.push_str(&format!(" ⤫ {n}（{sz} > {max_bytes}）\n"));
    }
    s.push_str(&format!(
        "\n上传 {} / 失败 {} / 跳过（超限）{}",
        uploaded.len(),
        failed.len(),
        skipped.len()
    ));
    s.push_str(if wrote {
        "\n资源已回填。\n"
    } else {
        "\n（没有需要回填的资源）\n"
    });
    if !skipped.is_empty() {
        s.push_str("超限的文件请自行上传到别处，再用资源外链提交。\n");
    }
    Ok(s)
}

// ------------------------------------------------------------ knowledge base

/// kb_path builds `/api/v1/kb` with the optional `sof` / `type` filters.
///
/// `sof` 有值 = 设备视图（该设备的 `sof` 文档 + 账号级 `common` / `scope`，同 key
/// 设备级覆盖）；无值 = 账号级视图。`type` ∈ common | scope | sof。
pub fn kb_path(sof: &str, type_filter: &str) -> String {
    let mut q: Vec<String> = Vec::new();
    if !sof.is_empty() {
        q.push(format!("sof={sof}"));
    }
    if !type_filter.is_empty() {
        q.push(format!("type={type_filter}"));
    }
    if q.is_empty() {
        "/api/v1/kb".to_string()
    } else {
        format!("/api/v1/kb?{}", q.join("&"))
    }
}

/// `ratsa kb` —— SOF 知识库索引（按类型：common 公共 / scope 用户 Scope / sof 设备）。
///
/// 知识库**不对外开放**：服务端只放行「该厂商账号名下的 key」或「bound_slug
/// 绑定到该账号的 key」，且 key 需带 `kb:read`；只有 `published` 文档会返回。
pub fn kb_index(
    client: &Client,
    sof: &str,
    type_filter: &str,
    json_out: bool,
) -> Result<String, String> {
    let path = kb_path(sof, type_filter);
    let resp = client.get(&path)?;
    if resp.status >= 400 {
        return Err(resp.error_message());
    }
    let v = resp.json();
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }

    let owner = v
        .get("owner")
        .and_then(|o| o.get("slug"))
        .and_then(|s| s.as_str())
        .unwrap_or("");
    let sof_name = v
        .get("sof")
        .and_then(|s| s.get("name"))
        .and_then(|s| s.as_str())
        .unwrap_or("");
    let docs = v
        .get("docs")
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default();

    let mut out = String::new();
    out.push_str(&format!(
        "SOF 知识库 · 归属 @{owner}{}{} · 共 {} 篇（仅含已发布）\n",
        if sof_name.is_empty() {
            ""
        } else {
            " · 设备 "
        },
        sof_name,
        docs.len()
    ));
    out.push_str(&format!(
        "spec {} · 范围：{}\n",
        v.get("spec").and_then(|s| s.as_str()).unwrap_or("-"),
        if sof.is_empty() {
            "账号级（common 公共 + scope 用户 Scope）"
        } else {
            "设备视图（该设备的 sof 文档 + 账号级；同 key 时设备级覆盖）"
        }
    ));
    out.push_str(&format!(
        "拉取原文：ratsa kb{} --doc <key>\n",
        if sof.is_empty() {
            String::new()
        } else {
            format!(" --sof {sof}")
        }
    ));
    if docs.is_empty() {
        out.push_str("\n（这个知识库还没有已发布的文档）\n");
        return Ok(out);
    }
    out.push_str("\n");
    let mut last_cat: Option<String> = None;
    for d in &docs {
        let cat = d.get("category").and_then(|c| c.as_str()).unwrap_or("");
        if last_cat.as_deref() != Some(cat) {
            out.push_str(&format!(
                "[{}]\n",
                if cat.is_empty() { "未分组" } else { cat }
            ));
            last_cat = Some(cat.to_string());
        }
        let key = d.get("key").and_then(|k| k.as_str()).unwrap_or("");
        let title = d.get("title").and_then(|k| k.as_str()).unwrap_or("");
        let tp = d.get("type").and_then(|k| k.as_str()).unwrap_or("");
        let bytes = d.get("bytes").and_then(|b| b.as_u64()).unwrap_or(0);
        let sum = d
            .get("checksum")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .trim_start_matches("sha256:");
        out.push_str(&format!(
            "  {:<22} {:<7} {:>6} 字节  {}  {}\n",
            truncate_cells(key, 22),
            tp,
            bytes,
            truncate_cells(title, 40),
            truncate_cells(sum, 12)
        ));
    }
    out.push_str("\n提示：原文为 Markdown；checksum 是 sha256(正文)，可在本地校验。\n");
    Ok(out)
}

/// `ratsa kb --doc <key|slug>` —— 拉取一篇 Markdown 原文（默认打印到终端）。
///
/// `doc` 既可以是文档键（如 `quickstart`），也可以是文档 slug；键会先在索引里
/// 解析成 slug，所以客户不需要记住随机编码。
pub fn kb_read(
    client: &Client,
    sof: &str,
    type_filter: &str,
    doc: &str,
    out_dir: &str,
) -> Result<String, String> {
    let index_path = kb_path(sof, type_filter);
    let idx = client.get(&index_path)?;
    if idx.status >= 400 {
        return Err(idx.error_message());
    }
    let idx = idx.json();
    let docs = idx
        .get("docs")
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default();
    let found = docs.iter().find(|d| {
        d.get("key").and_then(|k| k.as_str()) == Some(doc)
            || d.get("slug").and_then(|k| k.as_str()) == Some(doc)
    });
    let found = match found {
        Some(f) => f.clone(),
        None => {
            let keys: Vec<String> = docs
                .iter()
                .filter_map(|d| d.get("key").and_then(|k| k.as_str()).map(|s| s.to_string()))
                .collect();
            return Err(format!(
                "知识库里没有文档「{doc}」{}。可用：{}",
                if sof.is_empty() {
                    String::new()
                } else {
                    format!("（设备 {sof}）")
                },
                if keys.is_empty() {
                    "（暂无已发布文档）".to_string()
                } else {
                    keys.join(", ")
                }
            ));
        }
    };
    let slug = found
        .get("slug")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let key = found
        .get("key")
        .and_then(|s| s.as_str())
        .unwrap_or("doc")
        .to_string();
    let title = found
        .get("title")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let tp = found
        .get("type")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let want = found
        .get("checksum")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();

    let (body, headers) = client.download(&format!("/api/v1/kb/{slug}"))?;
    let got = headers.checksum.clone();
    if !want.is_empty() && !got.is_empty() && want != got {
        return Err(format!("checksum 不一致：期望 {want}，服务端返回 {got}"));
    }

    if out_dir.is_empty() {
        let mut out = String::new();
        out.push_str(&format!("# {title}\n"));
        out.push_str(&format!(
            "<!-- key={key} type={tp} checksum={} -->\n\n",
            if got.is_empty() {
                want.as_str()
            } else {
                got.as_str()
            }
        ));
        out.push_str(&String::from_utf8_lossy(&body));
        if !out.ends_with('\n') {
            out.push('\n');
        }
        return Ok(out);
    }

    let dir = PathBuf::from(out_dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建 {}: {e}", dir.display()))?;
    let file = dir.join(format!("{key}.md"));
    std::fs::write(&file, &body).map_err(|e| format!("写入 {} 失败: {e}", file.display()))?;
    Ok(format!(
        "已保存知识库文档\n  标题 : {title}\n  类型 : {tp}\n  落盘 : {}\n  大小 : {} 字节\n  校验 : {}\n",
        file.display(),
        body.len(),
        if got.is_empty() { want } else { got }
    ))
}

// ------------------------------------------------------------ report / feedback

#[allow(clippy::too_many_arguments)]
pub fn report_run(
    client: &Client,
    slug: &str,
    score: f64,
    passed: Option<i64>,
    failed: Option<i64>,
    summary: &str,
    notes: &str,
    source: &str,
) -> Result<String, String> {
    let mut checks = Vec::new();
    if let Some(p) = passed {
        checks.push(json!({"name": "passed", "status": "pass", "value": p}));
    }
    if let Some(f) = failed {
        checks.push(
            json!({"name": "failed", "status": if f == 0 { "pass" } else { "fail" }, "value": f}),
        );
    }
    let body = json!({
        "source": source,
        "score": score,
        "summary": summary,
        "notes": notes,
        "agent": "ratsa-harness",
        "checks": checks,
    });
    let resp = client.post(&format!("/api/v1/harness/eval/sof/{slug}"), &body)?;
    if resp.status >= 400 {
        return Err(resp.error_message());
    }
    let v = resp.json();
    // The platform answers with the stored execution view (`execution`), the
    // device it belongs to, and deep links into the readiness board.
    let exec = v.get("execution").cloned().unwrap_or(json!({}));
    let mut out = format!("已回传实测结果（{slug}）\n");
    if let Some(s) = exec.get("score") {
        out.push_str(&format!("  分数: {s}\n"));
    }
    if let Some(g) = exec.get("grade").and_then(|x| x.as_str()) {
        out.push_str(&format!("  评级: {g}\n"));
    }
    if let Some(src) = exec.get("source").and_then(|x| x.as_str()) {
        out.push_str(&format!("  来源: {src}\n"));
    }
    if let Some(name) = v
        .get("sof")
        .and_then(|s| s.get("name"))
        .and_then(|x| x.as_str())
    {
        out.push_str(&format!("  设备: {name}\n"));
    }
    if let Some(board) = v
        .get("links")
        .and_then(|l| l.get("board"))
        .and_then(|x| x.as_str())
    {
        out.push_str(&format!("  看板: {board}\n"));
    }
    Ok(out)
}

pub fn submit_feedback(
    client: &Client,
    slug: &str,
    title: &str,
    detail: &str,
    category: &str,
    severity: &str,
) -> Result<String, String> {
    let mut body = json!({
        "sof_slug": slug,
        "title": title,
        "detail": detail,
        "type": "agent_drive",
    });
    if !category.is_empty() {
        body["category"] = json!(category);
    }
    if !severity.is_empty() {
        body["severity"] = json!(severity);
    }
    let resp = client.post("/api/v1/feedback", &body)?;
    if resp.status >= 400 {
        return Err(resp.error_message());
    }
    let v = resp.json();
    let mut out = format!("已提交 Feedback：{title}\n");
    if let Some(cat) = v.get("category").and_then(|x| x.as_str()) {
        out.push_str(&format!("  归类: {cat}\n"));
    }
    if let Some(sev) = v.get("severity").and_then(|x| x.as_str()) {
        out.push_str(&format!("  严重度: {sev}\n"));
    }
    Ok(out)
}

// ------------------------------------------------------------------ RATSA Lab

fn lab_ok(resp: api::Response) -> Result<Value, String> {
    if resp.status >= 400 {
        return Err(resp.error_message());
    }
    Ok(resp.json())
}

fn read_json_file(path: &str) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("读取 {path} 失败：{e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("{path} 不是合法 JSON：{e}"))
}

fn labs_of(v: &Value) -> Vec<Value> {
    v.get("items")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default()
}

fn lab_field<'a>(lab: &'a Value, key: &str) -> &'a str {
    lab.get(key).and_then(|x| x.as_str()).unwrap_or("")
}

fn status_text(status: &str) -> &str {
    match status {
        "draft" => "草稿",
        "pending" => "审核中",
        "approved" => "已通过",
        "rejected" => "未通过",
        _ => status,
    }
}

/// List the acting account's labs. There is intentionally no public lab list:
/// Bio/Chem/Env/Materials labs are private to their owner (or bound account).
pub fn lab_list(client: &Client, json_out: bool) -> Result<String, String> {
    let v = lab_ok(client.get("/api/v1/labs")?)?;
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let items = labs_of(&v);
    if items.is_empty() {
        return Ok(
            "没有可管理的 Lab：任何登录账号都可自助创建。\n\
             用 `ratsa lab apply <profile.json>` 建档，再 `ratsa lab upload <slug> <file> --kind license` 上传资料、`--submit` 提交审核；\n\
             也可以直接在站点 /lab 创建。\n"
                .into(),
        );
    }
    let mut out = String::from("slug        领域       状态      Agent   名称\n");
    for it in items {
        out.push_str(&format!(
            "{:<11} {:<9} {:<8} {:<8} {}\n",
            lab_field(&it, "slug"),
            lab_field(&it, "vertical"),
            status_text(lab_field(&it, "status")),
            if it
                .get("agent_ready")
                .and_then(|x| x.as_bool())
                .unwrap_or(false)
            {
                "yes"
            } else {
                "no"
            },
            lab_field(&it, "name"),
        ));
    }
    Ok(out)
}

/// Read one lab (profile + services + records + materials + applications).
pub fn lab_get(client: &Client, slug: &str, json_out: bool) -> Result<String, String> {
    let v = lab_ok(client.get(&format!("/api/v1/labs/{slug}"))?)?;
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let lab = v.get("lab").cloned().unwrap_or(Value::Null);
    let n = |key: &str| {
        lab.get(key)
            .and_then(|x| x.as_array())
            .map(|a| a.len())
            .unwrap_or(0)
    };
    let mut out = format!(
        "{}  [{}]\nslug      : {}\n状态      : {}\nAgent     : {}\n领域      : {}\n",
        lab_field(&lab, "name"),
        lab_field(&lab, "vertical"),
        lab_field(&lab, "slug"),
        status_text(lab_field(&lab, "status")),
        if lab
            .get("agent_ready")
            .and_then(|x| x.as_bool())
            .unwrap_or(false)
        {
            "可接入"
        } else {
            "未声明"
        },
        lab_field(&lab, "vertical"),
    );
    if !lab_field(&lab, "description").is_empty() {
        out.push_str(&format!("简介      : {}\n", lab_field(&lab, "description")));
    }
    if !lab_field(&lab, "review_note").is_empty() {
        out.push_str(&format!("审核意见  : {}\n", lab_field(&lab, "review_note")));
    }
    out.push_str(&format!(
        "资料      : {}\n服务      : {}\n过往服务  : {}\n",
        n("documents"),
        n("services"),
        n("records")
    ));
    out.push_str(&format!(
        "\n下一步：ratsa lab spec {slug}   /   ratsa lab services {slug}\n"
    ));
    Ok(out)
}

/// Create / update the lab profile from a JSON file; `submit` also files the
/// application (materials must be uploaded first).
pub fn lab_apply(
    client: &Client,
    file: &str,
    submit: bool,
    note: &str,
    json_out: bool,
) -> Result<String, String> {
    let body = read_json_file(file)?;
    if !body.is_object() {
        return Err("Lab 档案 JSON 必须是对象（字段同 POST /api/lab）".into());
    }
    let saved = lab_ok(client.post("/api/v1/labs", &body)?)?;
    let mut result = saved.clone();
    if submit {
        result = lab_ok(client.post("/api/v1/labs/apply", &json!({ "note": note }))?)?;
    }
    if json_out {
        return Ok(serde_json::to_string_pretty(&result).unwrap_or_default());
    }
    let lab = result.get("lab").cloned().unwrap_or(result);
    let mut out = format!(
        "{}已保存：{}（{}）\n",
        if submit {
            "档案已提交审核："
        } else {
            "档案"
        },
        lab_field(&lab, "name"),
        lab_field(&lab, "slug"),
    );
    out.push_str(&format!(
        "状态：{}\n",
        status_text(lab_field(&lab, "status"))
    ));
    if submit && lab_field(&lab, "status") == "pending" {
        out.push_str("提示：平台运营审核资料后才能维护服务与过往服务记录。\n");
    }
    Ok(out)
}

/// Show the application / review status.
pub fn lab_status(client: &Client, slug: &str, json_out: bool) -> Result<String, String> {
    let v = lab_ok(client.get(&format!("/api/v1/labs/{slug}"))?)?;
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let lab = v.get("lab").cloned().unwrap_or(Value::Null);
    let mut out = format!(
        "{}（{}）\n状态：{}\n",
        lab_field(&lab, "name"),
        lab_field(&lab, "slug"),
        status_text(lab_field(&lab, "status"))
    );
    for key in ["submitted_at", "reviewed_at"] {
        if !lab_field(&lab, key).is_empty() {
            out.push_str(&format!("{key}：{}\n", lab_field(&lab, key)));
        }
    }
    if !lab_field(&lab, "review_note").is_empty() {
        out.push_str(&format!("审核意见：{}\n", lab_field(&lab, "review_note")));
    }
    Ok(out)
}

/// Delete the caller's lab (materials, services and records go with it).
pub fn lab_delete(client: &Client, slug: &str) -> Result<String, String> {
    lab_ok(client.delete(&format!("/api/v1/labs/{slug}"))?)?;
    Ok(format!("已删除 Lab {slug}（含资料、服务与过往服务记录）\n"))
}

fn content_type_for(path: &str) -> &'static str {
    match std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "txt" => "text/plain",
        "md" => "text/markdown",
        "csv" => "text/csv",
        "json" => "application/json",
        "zip" => "application/zip",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        _ => "application/octet-stream",
    }
}

/// Upload one credential / equipment / SOP file to the caller's lab.
pub fn lab_upload(
    client: &Client,
    slug: &str,
    file: &str,
    kind: &str,
    json_out: bool,
) -> Result<String, String> {
    let data = std::fs::read(file).map_err(|e| format!("读取 {file} 失败：{e}"))?;
    let name = std::path::Path::new(file)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("material.bin");
    let resp = client.post_multipart(
        &format!("/api/v1/labs/{slug}/documents"),
        "file",
        name,
        content_type_for(file),
        &[("kind", kind)],
        &data,
    )?;
    let v = lab_ok(resp)?;
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let doc = v.get("document").cloned().unwrap_or(Value::Null);
    Ok(format!(
        "已上传 {}（{}，{} 字节）\n",
        lab_field(&doc, "name"),
        lab_field(&doc, "kind"),
        data.len()
    ))
}

/// Pull `ratsa.lab.json` (the Agent/cloud contract for this lab).
pub fn lab_spec(client: &Client, slug: &str, out_dir: &str) -> Result<String, String> {
    let (bytes, headers) = client.download(&format!("/api/v1/labs/{slug}/spec"))?;
    verify_checksum(&bytes, &headers)?;
    let dir = if out_dir.is_empty() {
        default_lab_dir()
    } else {
        PathBuf::from(out_dir)
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建 {}: {e}", dir.display()))?;
    let file = dir.join(headers.filename("ratsa.lab.json"));
    std::fs::write(&file, &bytes).map_err(|e| format!("写入 {} 失败：{e}", file.display()))?;
    let mut out = format!("已保存 {}", file.display());
    if !headers.checksum.is_empty() {
        out.push_str(&format!("\nsha256 校验通过：{}", headers.checksum));
    }
    out.push('\n');
    Ok(out)
}

fn default_lab_dir() -> PathBuf {
    match std::env::var("HOME") {
        Ok(h) if !h.is_empty() => PathBuf::from(h).join(".ratsa").join("lab"),
        _ => PathBuf::from(".ratsa/lab"),
    }
}

pub fn lab_services(client: &Client, slug: &str, json_out: bool) -> Result<String, String> {
    let v = lab_ok(client.get(&format!("/api/v1/labs/{slug}/services"))?)?;
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let items = v
        .get("items")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    if items.is_empty() {
        return Ok("该 Lab 还没有发布服务（Lab 通过审核后可新增）。\n".into());
    }
    let mut out = String::from("id  类别        周期  Agent  名称\n");
    for s in items {
        out.push_str(&format!(
            "{:<3} {:<11} {:<5} {:<6} {}\n",
            s.get("id").and_then(|x| x.as_u64()).unwrap_or(0),
            lab_field(&s, "category"),
            s.get("turnaround_days")
                .and_then(|x| x.as_u64())
                .unwrap_or(0),
            if s.get("agent_callable")
                .and_then(|x| x.as_bool())
                .unwrap_or(false)
            {
                "yes"
            } else {
                "no"
            },
            lab_field(&s, "name"),
        ));
    }
    Ok(out)
}

pub fn lab_service_add(
    client: &Client,
    slug: &str,
    file: &str,
    json_out: bool,
) -> Result<String, String> {
    let body = read_json_file(file)?;
    let v = lab_ok(client.post(&format!("/api/v1/labs/{slug}/services"), &body)?)?;
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let s = v.get("service").cloned().unwrap_or(Value::Null);
    Ok(format!(
        "已新增服务：{}（id={}）\n",
        lab_field(&s, "name"),
        s.get("id").and_then(|x| x.as_u64()).unwrap_or(0)
    ))
}

pub fn lab_service_delete(client: &Client, slug: &str, id: u64) -> Result<String, String> {
    lab_ok(client.delete(&format!("/api/v1/labs/{slug}/services/{id}"))?)?;
    Ok(format!("已删除服务 {id}\n"))
}

pub fn lab_records(client: &Client, slug: &str, json_out: bool) -> Result<String, String> {
    let v = lab_ok(client.get(&format!("/api/v1/labs/{slug}/records"))?)?;
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let items = v
        .get("items")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    if items.is_empty() {
        return Ok("该 Lab 还没有过往服务记录。\n".into());
    }
    let mut out = String::from("id  完成时间    客户              标题\n");
    for r in items {
        out.push_str(&format!(
            "{:<3} {:<11} {:<16} {}\n",
            r.get("id").and_then(|x| x.as_u64()).unwrap_or(0),
            lab_field(&r, "completed_at").get(..10).unwrap_or("-"),
            truncate_cells(lab_field(&r, "client"), 16),
            lab_field(&r, "title"),
        ));
    }
    Ok(out)
}

pub fn lab_record_add(
    client: &Client,
    slug: &str,
    file: &str,
    json_out: bool,
) -> Result<String, String> {
    let body = read_json_file(file)?;
    let v = lab_ok(client.post(&format!("/api/v1/labs/{slug}/records"), &body)?)?;
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let r = v.get("record").cloned().unwrap_or(Value::Null);
    Ok(format!(
        "已新增服务记录：{}（id={}）\n",
        lab_field(&r, "title"),
        r.get("id").and_then(|x| x.as_u64()).unwrap_or(0)
    ))
}

pub fn lab_record_delete(client: &Client, slug: &str, id: u64) -> Result<String, String> {
    lab_ok(client.delete(&format!("/api/v1/labs/{slug}/records/{id}"))?)?;
    Ok(format!("已删除服务记录 {id}\n"))
}

fn device_status_text(status: &str) -> &str {
    match status {
        "active" => "在用",
        "idle" => "空闲",
        "maintenance" => "维护中",
        "offline" => "停用",
        _ => status,
    }
}

/// List the RATSA devices bound to a lab, with their lab-side status.
pub fn lab_devices(client: &Client, slug: &str, json_out: bool) -> Result<String, String> {
    let v = lab_ok(client.get(&format!("/api/v1/labs/{slug}/devices"))?)?;
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let items = v
        .get("items")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    if items.is_empty() {
        return Ok(
            "该 Lab 还没有绑定设备。用 `ratsa lab device add <lab> <sof-slug>` 添加。\n".into(),
        );
    }
    let mut out = format!(
        "id  {} {} {} {}\n",
        pad_cells("状态", 8),
        pad_cells("设备", 28),
        pad_cells("厂商", 16),
        "最近确认"
    );
    for d in items {
        out.push_str(&format!(
            "{} {} {} {} {}\n",
            pad_cells(
                &d.get("id")
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0)
                    .to_string(),
                3
            ),
            pad_cells(device_status_text(lab_field(&d, "status")), 8),
            pad_cells(&truncate_cells(lab_field(&d, "name"), 28), 28),
            pad_cells(&truncate_cells(lab_field(&d, "vendor_name"), 16), 16),
            lab_field(&d, "last_checked_at").get(..10).unwrap_or("-"),
        ));
    }
    Ok(out)
}

/// List device vendors / devices from the RATSA.ai catalog (public + own).
pub fn lab_vendors(client: &Client, query: &str, json_out: bool) -> Result<String, String> {
    let path = if query.is_empty() {
        "/api/v1/lab/catalog?limit=20".to_string()
    } else {
        format!("/api/v1/lab/catalog?limit=20&q={}", urlencode(query))
    };
    let v = lab_ok(client.get(&path)?)?;
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let vendors = v
        .get("vendors")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    let mut out = format!(
        "{} {} {} {}\n",
        pad_cells("厂商 slug", 12),
        pad_cells("设备数", 8),
        pad_cells("认证", 6),
        "名称"
    );
    for x in &vendors {
        out.push_str(&format!(
            "{} {} {} {}\n",
            pad_cells(lab_field(x, "slug"), 12),
            pad_cells(
                &x.get("devices")
                    .and_then(|d| d.as_u64())
                    .unwrap_or(0)
                    .to_string(),
                8
            ),
            pad_cells(
                if x.get("verified").and_then(|b| b.as_bool()).unwrap_or(false) {
                    "yes"
                } else {
                    "no"
                },
                6
            ),
            lab_field(x, "name"),
        ));
    }
    let items = v
        .get("items")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    if !items.is_empty() {
        out.push('\n');
        out.push_str(&format!(
            "{} {} {} {}\n",
            pad_cells("设备 slug", 12),
            pad_cells("类目", 12),
            pad_cells("厂商", 16),
            "名称"
        ));
        for it in items {
            let vendor = it.get("vendor").cloned().unwrap_or(Value::Null);
            out.push_str(&format!(
                "{} {} {} {}\n",
                pad_cells(lab_field(&it, "slug"), 12),
                pad_cells(&truncate_cells(lab_field(&it, "category"), 12), 12),
                pad_cells(&truncate_cells(lab_field(&vendor, "name"), 16), 16),
                lab_field(&it, "name"),
            ));
        }
    }
    out.push_str("\n下一步：ratsa lab device add <lab-slug> <sof-slug>\n");
    Ok(out)
}

/// Bind a RATSA device to the lab (the lab maintains status afterwards).
#[allow(clippy::too_many_arguments)]
pub fn lab_device_add(
    client: &Client,
    slug: &str,
    sof: &str,
    model: &str,
    note: &str,
    status: &str,
    json_out: bool,
) -> Result<String, String> {
    let body = json!({
        "sof_slug": sof,
        "model": model,
        "note": note,
        "status": if status.is_empty() { "active" } else { status },
    });
    let v = lab_ok(client.post(&format!("/api/v1/labs/{slug}/devices"), &body)?)?;
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let d = v.get("device").cloned().unwrap_or(Value::Null);
    Ok(format!(
        "已绑定设备：{}（id={}，厂商 {}）\n",
        lab_field(&d, "name"),
        d.get("id").and_then(|x| x.as_u64()).unwrap_or(0),
        lab_field(&d, "vendor_name"),
    ))
}

/// Update the lab-side status / note of a bound device.
pub fn lab_device_status(
    client: &Client,
    slug: &str,
    id: u64,
    status: &str,
    note: Option<&str>,
) -> Result<String, String> {
    let mut body = json!({ "status": status });
    if let Some(n) = note {
        body["note"] = json!(n);
    }
    let v = lab_ok(client.put(&format!("/api/v1/labs/{slug}/devices/{id}"), &body)?)?;
    let d = v.get("device").cloned().unwrap_or(Value::Null);
    Ok(format!(
        "已更新设备 {}：{}（{}）\n",
        d.get("id").and_then(|x| x.as_u64()).unwrap_or(id),
        device_status_text(lab_field(&d, "status")),
        lab_field(&d, "name"),
    ))
}

pub fn lab_device_remove(client: &Client, slug: &str, id: u64) -> Result<String, String> {
    lab_ok(client.delete(&format!("/api/v1/labs/{slug}/devices/{id}"))?)?;
    Ok(format!("已从 Lab 解绑设备 {id}\n"))
}

fn feedback_status_text(status: &str) -> &str {
    match status {
        "recorded" => "已登记",
        "verified" => "已核验",
        "disputed" => "有争议",
        _ => status,
    }
}

/// List the lab's customer transaction records plus the qualification summary.
pub fn lab_feedback(client: &Client, slug: &str, json_out: bool) -> Result<String, String> {
    let v = lab_ok(client.get(&format!("/api/v1/labs/{slug}/feedback"))?)?;
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let items = v
        .get("items")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    let mut out = String::new();
    if let Some(sm) = v.get("summary") {
        let num = |k: &str| sm.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0);
        out.push_str(&format!(
            "交易 {}, 已核验 {}, 平均评分 {}, 可展示 {}, 资质: {}\n\n",
            num("total") as i64,
            num("verified") as i64,
            num("avg_rating"),
            num("consented") as i64,
            if sm
                .get("qualification_ready")
                .and_then(|b| b.as_bool())
                .unwrap_or(false)
            {
                "达到参考门槛"
            } else {
                "未达参考门槛"
            },
        ));
    }
    if items.is_empty() {
        out.push_str("还没有登记服务交易。用 `ratsa lab feedback-add` 登记。\n");
        return Ok(out);
    }
    out.push_str(&format!(
        "id  {} {} {} {} {} {}\n",
        pad_cells("日期", 12),
        pad_cells("客户", 20),
        pad_cells("服务", 18),
        pad_cells("评分", 5),
        pad_cells("状态", 8),
        "公开"
    ));
    for f in items {
        let customer = if !lab_field(&f, "customer_company").is_empty() {
            lab_field(&f, "customer_company")
        } else {
            lab_field(&f, "customer_name")
        };
        let date = lab_field(&f, "occurred_at");
        let date = date.get(..10).unwrap_or("-");
        out.push_str(&format!(
            "{} {} {} {} {} {} {}\n",
            pad_cells(
                &f.get("id")
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0)
                    .to_string(),
                3
            ),
            pad_cells(date, 12),
            pad_cells(&truncate_cells(customer, 20), 20),
            pad_cells(&truncate_cells(lab_field(&f, "service_name"), 18), 18),
            pad_cells(
                &f.get("rating")
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0)
                    .to_string(),
                5
            ),
            pad_cells(feedback_status_text(lab_field(&f, "status")), 8),
            if f.get("consent").and_then(|b| b.as_bool()).unwrap_or(false) {
                "yes"
            } else {
                "no"
            },
        ));
    }
    Ok(out)
}

/// Register one customer transaction (the lab's key writes it).
#[allow(clippy::too_many_arguments)]
pub fn lab_feedback_add(
    client: &Client,
    slug: &str,
    customer: &str,
    company: &str,
    contact: &str,
    service_id: Option<u64>,
    order_ref: &str,
    amount: f64,
    currency: &str,
    rating: u8,
    category: &str,
    title: &str,
    detail: &str,
    date: &str,
    consent: bool,
    json_out: bool,
) -> Result<String, String> {
    let mut body = json!({
        "customer_name": customer,
        "customer_company": company,
        "customer_contact": contact,
        "order_ref": order_ref,
        "amount": amount,
        "currency": if currency.is_empty() { "CNY" } else { currency },
        "rating": rating,
        "category": if category.is_empty() { "other" } else { category },
        "title": title,
        "detail": detail,
        "consent": consent,
    });
    if let Some(id) = service_id {
        body["service_id"] = json!(id);
    }
    if !date.is_empty() {
        body["occurred_at"] = json!(date);
    }
    let v = lab_ok(client.post(&format!("/api/v1/labs/{slug}/feedback"), &body)?)?;
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let f = v.get("feedback").cloned().unwrap_or(Value::Null);
    Ok(format!(
        "已登记交易 #{}：{} / {}（评分 {}）\n",
        f.get("id").and_then(|x| x.as_u64()).unwrap_or(0),
        lab_field(&f, "customer_company"),
        lab_field(&f, "service_name"),
        f.get("rating").and_then(|x| x.as_u64()).unwrap_or(0),
    ))
}

/// Qualification / marketing roll-up for the lab.
pub fn lab_feedback_summary(client: &Client, slug: &str, json_out: bool) -> Result<String, String> {
    let v = lab_ok(client.get(&format!("/api/v1/labs/{slug}/feedback/summary"))?)?;
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let sm = v.get("summary").cloned().unwrap_or(Value::Null);
    let num = |k: &str| sm.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0);
    let mut out = format!(
        "交易数      : {}\n已核验      : {}\n有争议      : {}\n平均评分    : {}\n可展示案例  : {}\n资质参考    : {}\n",
        num("total") as i64,
        num("verified") as i64,
        num("disputed") as i64,
        num("avg_rating"),
        num("consented") as i64,
        if sm.get("qualification_ready").and_then(|b| b.as_bool()).unwrap_or(false) {
            "达到参考门槛"
        } else {
            "未达参考门槛"
        },
    );
    let testimonials = v
        .get("testimonials")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    if !testimonials.is_empty() {
        out.push_str("\n可展示案例：\n");
        for t in testimonials {
            out.push_str(&format!(
                "  - {}（{}）{} 分 {}\n",
                lab_field(&t, "customer_company"),
                lab_field(&t, "service_name"),
                t.get("rating").and_then(|x| x.as_u64()).unwrap_or(0),
                lab_field(&t, "title"),
            ));
        }
    }
    Ok(out)
}

pub fn lab_feedback_delete(client: &Client, slug: &str, id: u64) -> Result<String, String> {
    lab_ok(client.delete(&format!("/api/v1/labs/{slug}/feedback/{id}"))?)?;
    Ok(format!("已删除交易记录 {id}\n"))
}

/// Minimal percent-encoding for query values (space, &, #, non-ASCII).
fn urlencode(raw: &str) -> String {
    let mut out = String::new();
    for b in raw.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ------------------------------------------------------------------ helpers

fn short(s: &str) -> String {
    if s.chars().count() <= 16 {
        return s.to_string();
    }
    s.chars().take(16).collect()
}

/// Truncate by display cells (CJK counts as 2) so tables stay aligned.
fn truncate_cells(s: &str, max_cells: usize) -> String {
    let mut cells = 0;
    let mut out = String::new();
    for ch in s.chars() {
        let w = if (ch as u32) > 0x2E80 { 2 } else { 1 };
        if cells + w > max_cells {
            out.push('…');
            return out;
        }
        cells += w;
        out.push(ch);
    }
    out
}

/// Pad to a display-cell width (CJK counts as 2) so mixed-language tables line up.
fn pad_cells(s: &str, width: usize) -> String {
    let mut cells = 0;
    for ch in s.chars() {
        cells += if (ch as u32) > 0x2E80 { 2 } else { 1 };
    }
    let mut out = s.to_string();
    while cells < width {
        out.push(' ');
        cells += 1;
    }
    out
}

/// Verify the checksum reported by the server, when present. A Harness Package
/// is going to be executed locally, so a mismatch is treated as fatal.
pub fn verify_checksum(bytes: &[u8], headers: &Headers) -> Result<(), String> {
    if headers.checksum.is_empty() {
        return Ok(());
    }
    let mine = sha256_hex(bytes);
    if !mine.eq_ignore_ascii_case(&headers.checksum) {
        return Err(format!(
            "校验失败：服务端 {}，本地 {}（不要执行这个包）",
            headers.checksum, mine
        ));
    }
    Ok(())
}

pub(crate) fn sha256_hex(data: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, data);
    let mut out = String::with_capacity(digest.as_ref().len() * 2);
    for b in digest.as_ref() {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

// --------------------------------------------------------------------- RTH

/// `ratsa rth spec` — the machine-readable RTH (Ratsa Runtime Harness) contract.
pub fn rth_spec(client: &Client, json_out: bool) -> Result<String, String> {
    let resp = client.get("/api/rth/spec")?;
    if resp.status >= 400 {
        return Err(resp.error_message());
    }
    let v = resp.json();
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let mut out = String::new();
    out.push_str(&format!(
        "RTH · {}\n",
        v.get("manifest_spec")
            .and_then(|x| x.as_str())
            .unwrap_or("")
    ));
    if let Some(d) = v.get("definition").and_then(|x| x.as_str()) {
        out.push_str(&format!("{d}\n\n"));
    }
    out.push_str("Planes\n");
    for p in v
        .get("planes")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default()
    {
        out.push_str(&format!(
            "  • {} — {}\n",
            p.get("name").and_then(|x| x.as_str()).unwrap_or(""),
            p.get("role").and_then(|x| x.as_str()).unwrap_or("")
        ));
    }
    if let Some(caps) = v.pointer("/rth/capabilities").and_then(|x| x.as_array()) {
        out.push_str("\nRTHLoop capabilities\n");
        for c in caps {
            out.push_str(&format!("  • {}\n", c.as_str().unwrap_or("")));
        }
    }
    out.push_str("\nUse cases\n");
    for u in v
        .get("use_cases")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default()
    {
        out.push_str(&format!(
            "  • {}\n",
            u.get("text").and_then(|x| x.as_str()).unwrap_or("")
        ));
    }
    Ok(out)
}

pub fn rth_projects(client: &Client, query: &str, json_out: bool) -> Result<String, String> {
    let path = if query.trim().is_empty() {
        "/api/rth/projects".to_string()
    } else {
        format!("/api/rth/projects?q={}", urlencode(query))
    };
    let resp = client.get(&path)?;
    if resp.status >= 400 {
        return Err(resp.error_message());
    }
    let v = resp.json();
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let items = v
        .get("items")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    if items.is_empty() {
        return Ok("没有公开的 RTH 项目。\n".into());
    }
    let mut out =
        String::from("SLUG          NAME                          CORE      STATUS    AGENTIC\n");
    for it in items {
        out.push_str(&format!(
            "{:<13} {:<29} {:<9} {:<9} {}\n",
            it.get("slug").and_then(|x| x.as_str()).unwrap_or(""),
            truncate_cells(it.get("name").and_then(|x| x.as_str()).unwrap_or(""), 27),
            it.get("rth_loop_version")
                .and_then(|x| x.as_str())
                .unwrap_or("-"),
            it.get("status").and_then(|x| x.as_str()).unwrap_or("-"),
            it.get("agentic_status")
                .and_then(|x| x.as_str())
                .unwrap_or("-"),
        ));
    }
    Ok(out)
}

pub fn rth_get(client: &Client, slug: &str, json_out: bool) -> Result<String, String> {
    let resp = client.get(&format!("/api/rth/projects/{slug}"))?;
    if resp.status >= 400 {
        return Err(resp.error_message());
    }
    let v = resp.json();
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let item = v.get("item").cloned().unwrap_or(json!({}));
    let mut out = String::new();
    out.push_str(&format!(
        "{}  [{}]\n",
        item.get("name").and_then(|x| x.as_str()).unwrap_or("-"),
        item.get("status").and_then(|x| x.as_str()).unwrap_or("-")
    ));
    out.push_str(&format!("slug        : {slug}\n"));
    out.push_str(&format!(
        "RTHLoop     : {}\n",
        item.get("rth_loop_version")
            .and_then(|x| x.as_str())
            .unwrap_or("-")
    ));
    out.push_str(&format!(
        "Agentic     : {} {}\n",
        item.get("agentic_status")
            .and_then(|x| x.as_str())
            .unwrap_or("-"),
        item.get("agentic_score")
            .map(|s| s.to_string())
            .unwrap_or_default()
    ));
    if let Some(key) = item.get("signing_public_key").and_then(|x| x.as_str()) {
        out.push_str(&format!("signing key : {key}\n"));
    }
    if let Some(pkgs) = item.get("packages").and_then(|x| x.as_array()) {
        out.push_str(&format!("\npackages ({})\n", pkgs.len()));
        for p in pkgs {
            out.push_str(&format!(
                "  • [{}] {} v{} — {}\n",
                p.get("kind").and_then(|x| x.as_str()).unwrap_or(""),
                p.get("name").and_then(|x| x.as_str()).unwrap_or(""),
                p.get("version").and_then(|x| x.as_str()).unwrap_or(""),
                p.get("status").and_then(|x| x.as_str()).unwrap_or("")
            ));
        }
    }
    if let Some(ch) = v
        .get("links")
        .and_then(|l| l.get("channel"))
        .and_then(|x| x.as_str())
    {
        out.push_str(&format!("\nchannel: {ch}\n"));
    }
    Ok(out)
}

pub fn rth_channel(client: &Client, slug: &str, json_out: bool) -> Result<String, String> {
    let resp = client.get(&format!("/api/rth/projects/{slug}/channel"))?;
    if resp.status >= 400 {
        return Err(resp.error_message());
    }
    let v = resp.json();
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let mut out = String::new();
    out.push_str(&format!(
        "RTH channel · {} → {} → device\n",
        v.pointer("/service/provider")
            .and_then(|x| x.as_str())
            .unwrap_or("RATSA AI"),
        v.pointer("/system/harness")
            .and_then(|x| x.as_str())
            .unwrap_or("RTH")
    ));
    out.push_str(&format!(
        "ready       : {}\n",
        v.get("ready").and_then(|x| x.as_bool()).unwrap_or(false)
    ));
    if let Some(missing) = v.get("missing").and_then(|x| x.as_array()) {
        if !missing.is_empty() {
            out.push_str("missing     : ");
            out.push_str(
                &missing
                    .iter()
                    .filter_map(|x| x.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            out.push('\n');
        }
    }
    if let Some(pkgs) = v.get("packages").and_then(|x| x.as_array()) {
        out.push_str(&format!("packages    : {}\n", pkgs.len()));
        for p in pkgs {
            out.push_str(&format!(
                "  • [{}] {} v{}\n",
                p.get("kind").and_then(|x| x.as_str()).unwrap_or(""),
                p.get("name").and_then(|x| x.as_str()).unwrap_or(""),
                p.get("version").and_then(|x| x.as_str()).unwrap_or("")
            ));
        }
    }
    Ok(out)
}

pub fn rth_verify(
    client: &Client,
    slug: &str,
    package_id: u64,
    json_out: bool,
) -> Result<String, String> {
    let resp = client.get(&format!(
        "/api/rth/projects/{slug}/packages/{package_id}/verify"
    ))?;
    if resp.status >= 400 {
        return Err(resp.error_message());
    }
    let v = resp.json();
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let mut out = String::new();
    out.push_str(&format!(
        "RTH Signature · project {slug} · package {package_id}\nvalid    : {}\nadmitted : {}\n",
        v.get("valid").and_then(|x| x.as_bool()).unwrap_or(false),
        v.get("admitted").and_then(|x| x.as_bool()).unwrap_or(false)
    ));
    for c in v
        .get("checks")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default()
    {
        out.push_str(&format!(
            "  {} {}\n",
            if c.get("ok").and_then(|x| x.as_bool()).unwrap_or(false) {
                "✓"
            } else {
                "✗"
            },
            c.get("id").and_then(|x| x.as_str()).unwrap_or("")
        ));
    }
    Ok(out)
}

// ---------------------------------------------------------------- RTH platform writes

pub fn rth_create(
    client: &Client,
    name: &str,
    summary: &str,
    description: &str,
    loop_version: &str,
    repo: &str,
    docs: &str,
    sof: &str,
    lab: &str,
    visibility: &str,
    json_out: bool,
) -> Result<String, String> {
    let body = json!({
        "name": name,
        "summary": summary,
        "description": description,
        "rth_loop_version": loop_version,
        "repository_url": repo,
        "docs_url": docs,
        "sof_slug": sof,
        "lab_slug": lab,
        "visibility": visibility,
    });
    let resp = client.post("/api/rth/projects", &body)?;
    if resp.status >= 400 {
        return Err(resp.error_message());
    }
    let v = resp.json();
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let item = v.get("item").cloned().unwrap_or(json!({}));
    let mut out = String::from("RTH 项目已创建\n");
    out.push_str(&format!(
        "  name  : {}\n  slug  : {}\n  key   : {}\n",
        item.get("name").and_then(|x| x.as_str()).unwrap_or(name),
        item.get("slug").and_then(|x| x.as_str()).unwrap_or("-"),
        item.get("signing_public_key")
            .and_then(|x| x.as_str())
            .unwrap_or("-"),
    ));
    if let Some(web) = v.pointer("/links/web").and_then(|x| x.as_str()) {
        out.push_str(&format!("  web   : {web}\n"));
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
pub fn rth_package(
    client: &Client,
    slug: &str,
    name: &str,
    version: &str,
    kind: &str,
    file: Option<&str>,
    checksum: &str,
    metadata: &str,
    description: &str,
    json_out: bool,
) -> Result<String, String> {
    let path = format!("/api/rth/projects/{slug}/packages");
    let resp = if let Some(file) = file {
        let bytes = fs::read(file).map_err(|e| format!("读不了 {file}：{e}"))?;
        let filename = Path::new(file)
            .file_name()
            .and_then(|x| x.to_str())
            .unwrap_or("package.bin");
        let extra = [
            ("name", name),
            ("version", version),
            ("kind", kind),
            ("description", description),
            ("metadata", metadata),
        ];
        client.post_multipart(
            &path,
            "file",
            filename,
            "application/octet-stream",
            &extra,
            &bytes,
        )?
    } else {
        let body = json!({
            "name": name, "version": version, "kind": kind,
            "description": description, "metadata": metadata, "checksum": checksum,
        });
        client.post(&path, &body)?
    };
    if resp.status >= 400 {
        return Err(resp.error_message());
    }
    let v = resp.json();
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let item = v.get("item").cloned().unwrap_or(json!({}));
    Ok(format!(
        "Package 已登记\n  id      : {}\n  name    : {}\n  version : {}\n  kind    : {}\n  status  : {}\n  checksum: {}\n下一步：ratsa rth sign {slug} {}\n",
        item.get("id").map(|x| x.to_string()).unwrap_or_else(|| "-".into()),
        item.get("name").and_then(|x| x.as_str()).unwrap_or(name),
        item.get("version").and_then(|x| x.as_str()).unwrap_or(version),
        item.get("kind").and_then(|x| x.as_str()).unwrap_or(kind),
        item.get("status").and_then(|x| x.as_str()).unwrap_or("-"),
        item.get("checksum").and_then(|x| x.as_str()).unwrap_or("-"),
        item.get("id").map(|x| x.to_string()).unwrap_or_else(|| "<id>".into()),
    ))
}

pub fn rth_sign(
    client: &Client,
    slug: &str,
    package_id: u64,
    json_out: bool,
) -> Result<String, String> {
    let resp = client.post(
        &format!("/api/rth/projects/{slug}/packages/{package_id}/sign"),
        &json!({}),
    )?;
    if resp.status >= 400 {
        return Err(resp.error_message());
    }
    let v = resp.json();
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    Ok(format!(
        "RTH Signature\n  valid   : {}\n  status  : {}\n  algo    : {}\n  key id  : {}\n",
        v.get("valid").and_then(|x| x.as_bool()).unwrap_or(false),
        v.pointer("/item/status")
            .and_then(|x| x.as_str())
            .unwrap_or("-"),
        v.pointer("/item/signature_algo")
            .and_then(|x| x.as_str())
            .unwrap_or("-"),
        v.pointer("/item/signature_key_id")
            .and_then(|x| x.as_str())
            .unwrap_or("-"),
    ))
}

pub fn rth_validate(client: &Client, slug: &str, json_out: bool) -> Result<String, String> {
    let resp = client.post(
        &format!("/api/rth/projects/{slug}/validate-agentic"),
        &json!({}),
    )?;
    if resp.status >= 400 {
        return Err(resp.error_message());
    }
    let v = resp.json();
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    Ok(format!(
        "Agentic validation\n  status : {}\n  score  : {}\n  grade  : {}\n",
        v.pointer("/item/agentic_status")
            .and_then(|x| x.as_str())
            .unwrap_or("-"),
        v.pointer("/item/agentic_score")
            .map(|x| x.to_string())
            .unwrap_or_else(|| "-".into()),
        v.pointer("/result/readiness/grade")
            .and_then(|x| x.as_str())
            .unwrap_or("-"),
    ))
}

pub fn rth_admit(
    client: &Client,
    slug: &str,
    package_id: u64,
    note: &str,
    reject: bool,
) -> Result<String, String> {
    let action = if reject { "reject" } else { "admit" };
    let resp = client.post(
        &format!("/api/rth/projects/{slug}/packages/{package_id}/{action}"),
        &json!({ "note": note }),
    )?;
    if resp.status >= 400 {
        return Err(resp.error_message());
    }
    let v = resp.json();
    Ok(format!(
        "Package {package_id} {}\n  status : {}\n",
        if reject { "已拒绝" } else { "已准入" },
        v.pointer("/item/status")
            .and_then(|x| x.as_str())
            .unwrap_or("-")
    ))
}

// ---------------------------------------------------------------- RTH local runtime

const RTH_MANIFEST: &str = "ratsa.rth.json";
const RTH_STATE: &str = ".rth";

fn valid_rth_kind(kind: &str) -> bool {
    matches!(
        kind,
        "rth-loop"
            | "rth-core"
            | "rth-plugin"
            | "bsp"
            | "suite"
            | "sdk"
            | "hal"
            | "os"
            | "kernel"
            | "layer"
            | "perception"
            | "ontology"
            | "mem"
            | "service"
            | "other"
    )
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn rth_root(dir: Option<&str>) -> Result<PathBuf, String> {
    if let Some(d) = dir {
        return Ok(PathBuf::from(d));
    }
    let mut p = std::env::current_dir().map_err(|e| e.to_string())?;
    loop {
        if p.join(RTH_MANIFEST).exists() || p.join(RTH_STATE).exists() {
            return Ok(p);
        }
        if !p.pop() {
            break;
        }
    }
    Err("未找到 RTH 项目：请先运行 `ratsa rth init`（或在 --dir 指定目录）".into())
}

fn rth_manifest_path(root: &Path) -> PathBuf {
    root.join(RTH_MANIFEST)
}

fn load_rth_manifest(root: &Path) -> Result<Value, String> {
    let path = rth_manifest_path(root);
    let raw = fs::read_to_string(&path).map_err(|e| format!("读不了 {}：{e}", path.display()))?;
    serde_json::from_str(&raw).map_err(|e| format!("{} 不是合法 JSON：{e}", path.display()))
}

fn state_file(root: &Path, name: &str) -> PathBuf {
    root.join(RTH_STATE).join(name)
}

fn load_state(root: &Path, name: &str, default: Value) -> Value {
    let path = state_file(root, name);
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(default)
}

fn save_state(root: &Path, name: &str, value: &Value) -> Result<(), String> {
    let dir = root.join(RTH_STATE);
    fs::create_dir_all(&dir).map_err(|e| format!("无法创建 {}：{e}", dir.display()))?;
    let path = dir.join(name);
    let raw = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    fs::write(&path, raw + "\n").map_err(|e| format!("无法写入 {}：{e}", path.display()))
}

pub fn rth_init(
    dir: Option<&str>,
    name: Option<&str>,
    loop_version: &str,
) -> Result<String, String> {
    let root = dir
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir().map_err(|e| e.to_string())?);
    fs::create_dir_all(&root).map_err(|e| format!("无法创建 {}：{e}", root.display()))?;
    let manifest_path = rth_manifest_path(&root);
    if manifest_path.exists() {
        return Err(format!("已存在 {}，不会覆盖", manifest_path.display()));
    }
    let project = name
        .map(|x| x.to_string())
        .or_else(|| {
            root.file_name()
                .and_then(|x| x.to_str())
                .map(|x| x.to_string())
        })
        .unwrap_or_else(|| "rth-project".into());
    let manifest = json!({
        "kind": "ratsa.rth.json",
        "spec_version": "rth/1.0",
        "name": project,
        "rth_loop_version": loop_version,
        "build_graph": {
            "nodes": ["rth-loop", "plugin", "perception", "ontology", "mem"],
            "edges": []
        },
        "plugins": [],
        "robot_mem": {
            "stores": ["episodic", "semantic", "working"],
            "engine": "local"
        },
        "perception": [],
        "ontology": {
            "namespace": project,
            "version": "1.0.0",
            "entities": ["device", "sensor", "capability", "action", "state"],
            "actions": []
        }
    });
    let raw = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    fs::write(&manifest_path, raw + "\n")
        .map_err(|e| format!("无法写入 {}：{e}", manifest_path.display()))?;
    save_state(&root, "plugins.json", &json!([]))?;
    save_state(
        &root,
        "mem.json",
        &json!({"episodic": {}, "semantic": {}, "working": {}}),
    )?;
    save_state(
        &root,
        "runtime.json",
        &json!({
            "kind": "ratsa.rth.runtime",
            "spec_version": "rth/1.0",
            "rth_loop_version": loop_version,
            "created_at": now_unix()
        }),
    )?;
    Ok(format!(
        "RTH 项目已初始化：{}\n  manifest : {}\n  state    : {}/\n下一步：ratsa rth status / ratsa rth plugin add <id> / ratsa rth mem set <key> <value>\n",
        project,
        manifest_path.display(),
        root.join(RTH_STATE).display()
    ))
}

pub fn rth_status(dir: Option<&str>, json_out: bool) -> Result<String, String> {
    let root = rth_root(dir)?;
    let manifest = load_rth_manifest(&root)?;
    let plugins = load_state(&root, "plugins.json", json!([]));
    let mem = load_state(&root, "mem.json", json!({}));
    let ontology_ok = manifest
        .pointer("/ontology/namespace")
        .and_then(|x| x.as_str())
        .map(|x| !x.is_empty())
        .unwrap_or(false);
    let summary = json!({
        "root": root,
        "name": manifest.get("name"),
        "spec_version": manifest.get("spec_version"),
        "rth_loop_version": manifest.get("rth_loop_version").or_else(|| manifest.get("rth_core_version")),
        "plugins": plugins.as_array().map(|x| x.len()).unwrap_or(0),
        "perception": manifest.get("perception").and_then(|x| x.as_array()).map(|x| x.len()).unwrap_or(0),
        "build_nodes": manifest.pointer("/build_graph/nodes").and_then(|x| x.as_array()).map(|x| x.len()).unwrap_or(0),
        "mem": mem.as_object().map(|o| o.iter().map(|(k, v)| {
            (k.clone(), json!(v.as_object().map(|m| m.len()).unwrap_or(0)))
        }).collect::<serde_json::Map<_, _>>()),
        "ontology_ok": ontology_ok,
    });
    if json_out {
        return Ok(serde_json::to_string_pretty(&summary).unwrap_or_default());
    }
    Ok(format!(
        "RTH runtime · {}\n  root        : {}\n  spec        : {}\n  RTHLoop     : {}\n  plugins     : {}\n  perception  : {}\n  build nodes : {}\n  ontology    : {}\n",
        manifest.get("name").and_then(|x| x.as_str()).unwrap_or("-"),
        root.display(),
        manifest.get("spec_version").and_then(|x| x.as_str()).unwrap_or("-"),
        manifest
            .get("rth_loop_version")
            .or_else(|| manifest.get("rth_core_version"))
            .and_then(|x| x.as_str())
            .unwrap_or("-"),
        summary.get("plugins").map(|x| x.to_string()).unwrap_or_else(|| "0".into()),
        summary.get("perception").map(|x| x.to_string()).unwrap_or_else(|| "0".into()),
        summary.get("build_nodes").map(|x| x.to_string()).unwrap_or_else(|| "0".into()),
        if ontology_ok { "ok" } else { "missing namespace" },
    ))
}

pub fn rth_plugin_list(dir: Option<&str>, json_out: bool) -> Result<String, String> {
    let root = rth_root(dir)?;
    let plugins = load_state(&root, "plugins.json", json!([]));
    if json_out {
        return Ok(serde_json::to_string_pretty(&plugins).unwrap_or_default());
    }
    let items = plugins.as_array().cloned().unwrap_or_default();
    if items.is_empty() {
        return Ok("没有已安装的 RTH 插件。\n".into());
    }
    let mut out = String::from("ID                  VERSION   KIND           CHANNEL   SOURCE\n");
    for p in items {
        out.push_str(&format!(
            "{:<19} {:<9} {:<14} {:<9} {}\n",
            truncate_cells(p.get("id").and_then(|x| x.as_str()).unwrap_or(""), 17),
            truncate_cells(p.get("version").and_then(|x| x.as_str()).unwrap_or("-"), 8),
            truncate_cells(p.get("kind").and_then(|x| x.as_str()).unwrap_or("-"), 12),
            truncate_cells(p.get("channel").and_then(|x| x.as_str()).unwrap_or("-"), 8),
            p.get("source").and_then(|x| x.as_str()).unwrap_or(""),
        ));
    }
    Ok(out)
}

pub fn rth_plugin_add(
    dir: Option<&str>,
    id: &str,
    version: &str,
    kind: &str,
    source: &str,
    channel: &str,
) -> Result<String, String> {
    if !valid_rth_kind(kind) {
        return Err(format!("未知插件类型「{kind}」"));
    }
    let root = rth_root(dir)?;
    let existing = load_state(&root, "plugins.json", json!([]));
    let mut items = existing.as_array().cloned().unwrap_or_default();
    let now = now_unix();
    if let Some(item) = items
        .iter_mut()
        .find(|p| p.get("id").and_then(|x| x.as_str()) == Some(id))
    {
        item["version"] = json!(version);
        item["kind"] = json!(kind);
        item["source"] = json!(source);
        item["channel"] = json!(channel);
        item["updated_at"] = json!(now);
    } else {
        items.push(json!({
            "id": id, "version": version, "kind": kind, "source": source,
            "channel": channel, "installed_at": now, "updated_at": now
        }));
    }
    save_state(&root, "plugins.json", &Value::Array(items))?;
    Ok(format!(
        "插件 {id}@{version} 已登记（{kind}/{channel}）。\n"
    ))
}

pub fn rth_plugin_update(dir: Option<&str>, id: &str, version: &str) -> Result<String, String> {
    let root = rth_root(dir)?;
    let existing = load_state(&root, "plugins.json", json!([]));
    let mut items = existing.as_array().cloned().unwrap_or_default();
    let now = now_unix();
    let Some(item) = items
        .iter_mut()
        .find(|p| p.get("id").and_then(|x| x.as_str()) == Some(id))
    else {
        return Err(format!("插件 {id} 未安装"));
    };
    item["version"] = json!(version);
    item["updated_at"] = json!(now);
    save_state(&root, "plugins.json", &Value::Array(items))?;
    Ok(format!("插件 {id} 已更新到 {version}。\n"))
}

pub fn rth_plugin_remove(dir: Option<&str>, id: &str) -> Result<String, String> {
    let root = rth_root(dir)?;
    let existing = load_state(&root, "plugins.json", json!([]));
    let mut items = existing.as_array().cloned().unwrap_or_default();
    let before = items.len();
    items.retain(|p| p.get("id").and_then(|x| x.as_str()) != Some(id));
    if items.len() == before {
        return Err(format!("插件 {id} 未安装"));
    }
    save_state(&root, "plugins.json", &Value::Array(items))?;
    Ok(format!("插件 {id} 已移除。\n"))
}

fn valid_mem_store(store: &str) -> Result<(), String> {
    match store {
        "episodic" | "semantic" | "working" => Ok(()),
        _ => Err(format!(
            "未知 Mem store「{store}」，可选 episodic | semantic | working"
        )),
    }
}

fn mem_object(root: &Path) -> Value {
    let mut mem = load_state(root, "mem.json", json!({}));
    if !mem.is_object() {
        mem = json!({});
    }
    mem
}

pub fn rth_mem_list(dir: Option<&str>, store: &str, json_out: bool) -> Result<String, String> {
    valid_mem_store(store)?;
    let root = rth_root(dir)?;
    let mem = mem_object(&root);
    let items = mem
        .get(store)
        .and_then(|x| x.as_object())
        .cloned()
        .unwrap_or_default();
    if json_out {
        return Ok(serde_json::to_string_pretty(&Value::Object(items)).unwrap_or_default());
    }
    if items.is_empty() {
        return Ok(format!("Mem[{store}] 为空。\n"));
    }
    let mut out = format!("Mem[{store}]\n");
    for (k, v) in items {
        out.push_str(&format!("  {k} = {}\n", api::truncate(&v.to_string(), 160)));
    }
    Ok(out)
}

pub fn rth_mem_get(
    dir: Option<&str>,
    store: &str,
    key: &str,
    json_out: bool,
) -> Result<String, String> {
    valid_mem_store(store)?;
    let root = rth_root(dir)?;
    let mem = mem_object(&root);
    let Some(v) = mem.pointer(&format!("/{store}/{key}")) else {
        return Err(format!("Mem[{store}] 没有键 {key}"));
    };
    if json_out {
        Ok(serde_json::to_string_pretty(v).unwrap_or_default())
    } else {
        Ok(format!("{}\n", v))
    }
}

pub fn rth_mem_set(
    dir: Option<&str>,
    store: &str,
    key: &str,
    value: &str,
) -> Result<String, String> {
    valid_mem_store(store)?;
    let root = rth_root(dir)?;
    let mut mem = mem_object(&root);
    if !mem.get(store).map(|x| x.is_object()).unwrap_or(false) {
        mem[store] = json!({});
    }
    let parsed: Value = serde_json::from_str(value).unwrap_or_else(|_| json!(value));
    mem[store][key] = parsed;
    save_state(&root, "mem.json", &mem)?;
    Ok(format!("Mem[{store}].{key} 已写入。\n"))
}

pub fn rth_mem_delete(dir: Option<&str>, store: &str, key: &str) -> Result<String, String> {
    valid_mem_store(store)?;
    let root = rth_root(dir)?;
    let mut mem = mem_object(&root);
    let removed = mem
        .get_mut(store)
        .and_then(|x| x.as_object_mut())
        .and_then(|o| o.remove(key));
    if removed.is_none() {
        return Err(format!("Mem[{store}] 没有键 {key}"));
    }
    save_state(&root, "mem.json", &mem)?;
    Ok(format!("Mem[{store}].{key} 已删除。\n"))
}

pub fn rth_perception(dir: Option<&str>, json_out: bool) -> Result<String, String> {
    let root = rth_root(dir)?;
    let manifest = load_rth_manifest(&root)?;
    let items = manifest
        .get("perception")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    if json_out {
        return Ok(serde_json::to_string_pretty(&Value::Array(items)).unwrap_or_default());
    }
    if items.is_empty() {
        return Ok("清单中没有感知部件。\n".into());
    }
    let mut out = String::from("ID                  MODALITY    INTERFACE        ONTOLOGY\n");
    for p in items {
        out.push_str(&format!(
            "{:<19} {:<11} {:<16} {}\n",
            truncate_cells(p.get("id").and_then(|x| x.as_str()).unwrap_or(""), 17),
            truncate_cells(
                p.get("modality").and_then(|x| x.as_str()).unwrap_or("-"),
                10
            ),
            truncate_cells(
                p.get("interface").and_then(|x| x.as_str()).unwrap_or("-"),
                15
            ),
            p.get("ontology_ref").and_then(|x| x.as_str()).unwrap_or(""),
        ));
    }
    Ok(out)
}

pub fn rth_ontology(dir: Option<&str>, json_out: bool) -> Result<String, String> {
    let root = rth_root(dir)?;
    let manifest = load_rth_manifest(&root)?;
    let mut errors: Vec<String> = Vec::new();
    let Some(onto) = manifest.get("ontology") else {
        return Err("清单缺少 ontology 区块".into());
    };
    let namespace = onto.get("namespace").and_then(|x| x.as_str()).unwrap_or("");
    if namespace.is_empty() {
        errors.push("ontology.namespace 不能为空".into());
    }
    if onto
        .get("version")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .is_empty()
    {
        errors.push("ontology.version 不能为空".into());
    }
    match onto.get("entities") {
        Some(Value::Array(a)) if !a.is_empty() => {}
        Some(Value::Array(_)) => errors.push("ontology.entities 不能为空".into()),
        _ => errors.push("ontology.entities 必须是数组".into()),
    }
    if let Some(actions) = onto.get("actions") {
        if !actions.is_array() {
            errors.push("ontology.actions 必须是数组".into());
        }
    }
    let report = json!({
        "root": root,
        "namespace": namespace,
        "valid": errors.is_empty(),
        "errors": errors,
    });
    if json_out {
        return Ok(serde_json::to_string_pretty(&report).unwrap_or_default());
    }
    if errors.is_empty() {
        Ok(format!("Ontology 校验通过：namespace={namespace}\n"))
    } else {
        let mut out = String::from("Ontology 校验失败\n");
        for e in errors {
            out.push_str(&format!("  ✗ {e}\n"));
        }
        Err(out)
    }
}

pub fn rth_build_graph(dir: Option<&str>, json_out: bool) -> Result<String, String> {
    let root = rth_root(dir)?;
    let manifest = load_rth_manifest(&root)?;
    let graph = manifest.get("build_graph").cloned().unwrap_or(json!({}));
    if json_out {
        return Ok(serde_json::to_string_pretty(&graph).unwrap_or_default());
    }
    let nodes = graph
        .get("nodes")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    let edges = graph
        .get("edges")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    let mut out = String::from("Build graph\n  nodes:\n");
    for n in nodes {
        out.push_str(&format!("    • {}\n", n.as_str().unwrap_or(&n.to_string())));
    }
    out.push_str("  edges:\n");
    if edges.is_empty() {
        out.push_str("    (none)\n");
    } else {
        for e in edges {
            out.push_str(&format!(
                "    {} -> {} ({})\n",
                e.get("from").and_then(|x| x.as_str()).unwrap_or("?"),
                e.get("to").and_then(|x| x.as_str()).unwrap_or("?"),
                e.get("type")
                    .and_then(|x| x.as_str())
                    .unwrap_or("depends_on"),
            ));
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- RTH Platform

pub fn rth_platform(client: &Client, json_out: bool) -> Result<String, String> {
    let resp = client.get("/api/rth/platform")?;
    if resp.status >= 400 {
        return Err(resp.error_message());
    }
    let v = resp.json();
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let mut out = String::new();
    out.push_str(&format!(
        "RTH Platform · {}\n",
        v.get("rth_loop_version")
            .and_then(|x| x.as_str())
            .unwrap_or("")
    ));
    if let Some(d) = v.get("definition").and_then(|x| x.as_str()) {
        out.push_str(&format!("{d}\n\n"));
    }
    out.push_str("Runtime modes\n");
    for m in v
        .get("modes")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default()
    {
        out.push_str(&format!(
            "  • {} ({}) — {}\n",
            m.get("name").and_then(|x| x.as_str()).unwrap_or(""),
            m.get("id").and_then(|x| x.as_str()).unwrap_or(""),
            m.get("summary").and_then(|x| x.as_str()).unwrap_or("")
        ));
    }
    out.push_str("\nPackage families\n");
    for p in v
        .get("package_families")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default()
    {
        out.push_str(&format!(
            "  • {} [{}] — {}\n",
            p.get("name").and_then(|x| x.as_str()).unwrap_or(""),
            p.get("kind").and_then(|x| x.as_str()).unwrap_or(""),
            p.get("description").and_then(|x| x.as_str()).unwrap_or("")
        ));
    }
    if let Some(u) = v.pointer("/release/cli").and_then(|x| x.as_str()) {
        out.push_str(&format!("\nRelease (CLI): {u}\n"));
    }
    Ok(out)
}

pub fn rth_releases(client: &Client, slug: &str, json_out: bool) -> Result<String, String> {
    let resp = client.get(&format!("/api/rth/releases/{slug}"))?;
    if resp.status >= 400 {
        return Err(resp.error_message());
    }
    let v = resp.json();
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let items = v
        .get("items")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    if items.is_empty() {
        return Ok("没有 Release。\n".into());
    }
    let mut out = String::from("VERSION   CHANNEL   MODE          STATUS      PUBLISHED\n");
    for r in items {
        out.push_str(&format!(
            "{:<9} {:<9} {:<13} {:<11} {}\n",
            r.get("version").and_then(|x| x.as_str()).unwrap_or("-"),
            r.get("channel").and_then(|x| x.as_str()).unwrap_or("-"),
            r.get("mode").and_then(|x| x.as_str()).unwrap_or("-"),
            r.get("status").and_then(|x| x.as_str()).unwrap_or("-"),
            r.get("published_at").and_then(|x| x.as_str()).unwrap_or(""),
        ));
    }
    Ok(out)
}

pub fn rth_release(
    client: &Client,
    slug: &str,
    version: &str,
    json_out: bool,
) -> Result<String, String> {
    let resp = client.get(&format!("/api/rth/releases/{slug}/{version}"))?;
    if resp.status >= 400 {
        return Err(resp.error_message());
    }
    let v = resp.json();
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let item = v.get("item").cloned().unwrap_or(json!({}));
    let mut out = format!(
        "{slug} · v{version}\n  status  : {}\n  channel : {}\n  mode    : {}\n",
        item.get("status").and_then(|x| x.as_str()).unwrap_or("-"),
        item.get("channel").and_then(|x| x.as_str()).unwrap_or("-"),
        item.get("mode").and_then(|x| x.as_str()).unwrap_or("-"),
    );
    if let Some(n) = item.get("notes").and_then(|x| x.as_str()) {
        if !n.is_empty() {
            out.push_str(&format!("  notes   : {n}\n"));
        }
    }
    if let Some(pkgs) = item.get("packages").and_then(|x| x.as_array()) {
        out.push_str(&format!("\npackages ({})\n", pkgs.len()));
        for p in pkgs {
            out.push_str(&format!(
                "  • [{}] {} v{} — {}\n",
                p.get("kind").and_then(|x| x.as_str()).unwrap_or(""),
                p.get("name").and_then(|x| x.as_str()).unwrap_or(""),
                p.get("version").and_then(|x| x.as_str()).unwrap_or(""),
                p.get("checksum").and_then(|x| x.as_str()).unwrap_or(""),
            ));
        }
    }
    if let Some(u) = v.pointer("/links/cli").and_then(|x| x.as_str()) {
        out.push_str(&format!("\nrefetch: {u}\n"));
    }
    Ok(out)
}

pub fn rth_release_create(
    client: &Client,
    slug: &str,
    version: &str,
    channel: &str,
    mode: &str,
    notes: &str,
    publish: bool,
) -> Result<String, String> {
    let body = json!({
        "version": version, "channel": channel, "mode": mode, "notes": notes, "publish": publish
    });
    let resp = client.post(&format!("/api/rth/projects/{slug}/releases"), &body)?;
    if resp.status >= 400 {
        return Err(resp.error_message());
    }
    let v = resp.json();
    Ok(format!(
        "Release v{} 已创建\n  status : {}\n  mode   : {}\n下一步：ratsa rth release {slug} {version}\n",
        v.pointer("/item/version").and_then(|x| x.as_str()).unwrap_or(version),
        v.pointer("/item/status").and_then(|x| x.as_str()).unwrap_or("-"),
        v.pointer("/item/mode").and_then(|x| x.as_str()).unwrap_or(mode),
    ))
}

pub fn rth_publish(client: &Client, slug: &str, version: &str) -> Result<String, String> {
    let resp = client.post(
        &format!("/api/rth/projects/{slug}/releases/{version}/publish"),
        &json!({}),
    )?;
    if resp.status >= 400 {
        return Err(resp.error_message());
    }
    let v = resp.json();
    Ok(format!(
        "Release v{version} 已发布（status={})\n",
        v.pointer("/item/status")
            .and_then(|x| x.as_str())
            .unwrap_or("-")
    ))
}

// --------------------------------------------------- RTH user assets (profile)

fn asset_error(resp: &api::Response) -> Option<String> {
    if resp.status >= 400 {
        Some(resp.error_message())
    } else {
        None
    }
}

fn read_value_arg(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if let Some(path) = raw.strip_prefix('@') {
        return fs::read_to_string(path).map_err(|e| format!("无法读取 {path}: {e}"));
    }
    Ok(raw.to_string())
}

fn upload_content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "tar" => "application/x-tar",
        "wasm" => "application/wasm",
        "json" => "application/json",
        "whl" => "application/zip",
        _ => "application/octet-stream",
    }
}

fn format_asset_checks(checks: &[Value]) -> String {
    let mut out = String::new();
    for ch in checks {
        let mark = if ch.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
            "✓"
        } else {
            "✗"
        };
        out.push_str(&format!(
            "  {mark} {}{}\n",
            ch.get("label").and_then(|v| v.as_str()).unwrap_or(""),
            ch.get("detail")
                .and_then(|v| v.as_str())
                .filter(|d| !d.is_empty())
                .map(|d| format!(" — {d}"))
                .unwrap_or_default()
        ));
    }
    out
}

fn format_asset_detail(v: &Value) -> String {
    let item = v.get("item").cloned().unwrap_or(json!({}));
    let mut out = String::new();
    out.push_str(&format!(
        "#{} {} v{} [{}] — {}\n",
        item.get("id").and_then(|x| x.as_u64()).unwrap_or(0),
        item.get("name").and_then(|x| x.as_str()).unwrap_or("-"),
        item.get("version").and_then(|x| x.as_str()).unwrap_or("-"),
        item.get("kind").and_then(|x| x.as_str()).unwrap_or("-"),
        item.get("status").and_then(|x| x.as_str()).unwrap_or("-")
    ));
    if let Some(d) = item
        .get("description")
        .and_then(|x| x.as_str())
        .filter(|d| !d.is_empty())
    {
        out.push_str(&format!("{d}\n"));
    }
    out.push_str(&format!(
        "checksum : {}\nvisibility: {}\n",
        item.get("checksum").and_then(|x| x.as_str()).unwrap_or("-"),
        item.get("visibility")
            .and_then(|x| x.as_str())
            .unwrap_or("-")
    ));
    if let Some(r) = item
        .get("repository_url")
        .and_then(|x| x.as_str())
        .filter(|r| !r.is_empty())
    {
        out.push_str(&format!("repo     : {r}\n"));
    }
    if let Some(sig) = item
        .get("signature")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
    {
        out.push_str(&format!("signature: {}\n", api::truncate(sig, 32)));
    }
    if let Some(checks) = v.get("checks").and_then(|x| x.as_array()) {
        out.push_str("\nchecks\n");
        out.push_str(&format_asset_checks(checks));
    }
    out
}

/// `ratsa rth assets`
pub fn rth_assets(
    client: &Client,
    status: Option<&str>,
    kind: Option<&str>,
    json_out: bool,
) -> Result<String, String> {
    let mut path = String::from("/api/rth/profile/assets");
    let mut sep = '?';
    if let Some(s) = status.filter(|s| !s.trim().is_empty()) {
        path.push_str(&format!("{sep}status={}", urlencode(s)));
        sep = '&';
    }
    if let Some(k) = kind.filter(|k| !k.trim().is_empty()) {
        path.push_str(&format!("{sep}kind={}", urlencode(k)));
    }
    let resp = client.get(&path)?;
    if let Some(e) = asset_error(&resp) {
        return Err(e);
    }
    let v = resp.json();
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let items = v
        .get("items")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    if items.is_empty() {
        return Ok(
            "还没有 RTH 资产。用 `ratsa rth asset-upload --name … --file …` 上传。\n".into(),
        );
    }
    let mut out =
        String::from("ID    NAME                          VERSION   KIND          STATUS\n");
    for it in items {
        out.push_str(&format!(
            "{:<5} {:<29} {:<9} {:<13} {}\n",
            it.get("id").and_then(|x| x.as_u64()).unwrap_or(0),
            truncate_cells(it.get("name").and_then(|x| x.as_str()).unwrap_or(""), 27),
            it.get("version").and_then(|x| x.as_str()).unwrap_or("-"),
            it.get("kind").and_then(|x| x.as_str()).unwrap_or("-"),
            it.get("status").and_then(|x| x.as_str()).unwrap_or("-"),
        ));
    }
    Ok(out)
}

/// `ratsa rth asset-upload`
#[allow(clippy::too_many_arguments)]
pub fn rth_asset_upload(
    client: &Client,
    name: &str,
    version: &str,
    kind: &str,
    file: Option<&str>,
    description: &str,
    metadata: &str,
    manifest: Option<&str>,
    checksum: &str,
    repo: &str,
    license: &str,
    signature: &str,
    signature_key: &str,
    visibility: &str,
    json_out: bool,
) -> Result<String, String> {
    let metadata = read_value_arg(metadata)?;
    if !metadata.is_empty() && serde_json::from_str::<Value>(&metadata).is_err() {
        return Err("--metadata 不是合法 JSON".into());
    }
    let manifest_text = match manifest {
        Some(path) if !path.trim().is_empty() => {
            let text =
                fs::read_to_string(path).map_err(|e| format!("无法读取 manifest {path}: {e}"))?;
            if serde_json::from_str::<Value>(&text).is_err() {
                return Err("--manifest 文件不是合法 JSON".into());
            }
            text
        }
        _ => String::new(),
    };
    let signature = read_value_arg(signature)?;
    let signature_key = read_value_arg(signature_key)?;

    let resp = if let Some(file) = file.filter(|f| !f.trim().is_empty()) {
        let path = Path::new(file);
        let data = fs::read(path).map_err(|e| format!("无法读取 {file}: {e}"))?;
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("asset.bin");
        let extra_owned: Vec<(&str, String)> = vec![
            ("name", name.to_string()),
            ("version", version.to_string()),
            ("kind", kind.to_string()),
            ("description", description.to_string()),
            ("metadata", metadata.clone()),
            ("manifest", manifest_text.clone()),
            ("checksum", checksum.to_string()),
            ("repository_url", repo.to_string()),
            ("license", license.to_string()),
            ("signature", signature.clone()),
            ("signature_public_key", signature_key.clone()),
            ("visibility", visibility.to_string()),
        ];
        let extra: Vec<(&str, &str)> = extra_owned.iter().map(|(k, v)| (*k, v.as_str())).collect();
        client.post_multipart(
            "/api/rth/profile/assets",
            "file",
            filename,
            upload_content_type(path),
            &extra,
            &data,
        )?
    } else {
        let body = json!({
            "name": name,
            "version": version,
            "kind": kind,
            "description": description,
            "metadata": metadata,
            "manifest": manifest_text,
            "checksum": checksum,
            "repository_url": repo,
            "license": license,
            "signature": signature,
            "signature_public_key": signature_key,
            "visibility": visibility,
        });
        client.post("/api/rth/profile/assets", &body)?
    };
    if let Some(e) = asset_error(&resp) {
        return Err(e);
    }
    let v = resp.json();
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let item = v.get("item").cloned().unwrap_or(json!({}));
    let certified = v
        .get("certified")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    let mut out = String::new();
    out.push_str(&format!(
        "{} #{} {} v{} ({})\n",
        if certified {
            "✓ 已认证"
        } else {
            "… 等待认证"
        },
        item.get("id").and_then(|x| x.as_u64()).unwrap_or(0),
        item.get("name").and_then(|x| x.as_str()).unwrap_or("-"),
        item.get("version").and_then(|x| x.as_str()).unwrap_or("-"),
        item.get("kind").and_then(|x| x.as_str()).unwrap_or("-"),
    ));
    out.push_str(&format!(
        "checksum: {}\n",
        item.get("checksum").and_then(|x| x.as_str()).unwrap_or("-")
    ));
    if let Some(checks) = v.get("checks").and_then(|x| x.as_array()) {
        out.push_str("\nchecks\n");
        out.push_str(&format_asset_checks(checks));
    }
    if !certified {
        out.push_str("\n账号通过认证后可执行 `ratsa rth asset-certify <id>` 重新认证。\n");
    }
    Ok(out)
}

/// `ratsa rth asset <id>`
pub fn rth_asset(client: &Client, id: u64, json_out: bool) -> Result<String, String> {
    let resp = client.get(&format!("/api/rth/profile/assets/{id}"))?;
    if let Some(e) = asset_error(&resp) {
        return Err(e);
    }
    let v = resp.json();
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    Ok(format_asset_detail(&v))
}

/// `ratsa rth asset-certify <id>`
pub fn rth_asset_certify(client: &Client, id: u64, json_out: bool) -> Result<String, String> {
    let resp = client.post(&format!("/api/rth/profile/assets/{id}/certify"), &json!({}))?;
    if let Some(e) = asset_error(&resp) {
        return Err(e);
    }
    let v = resp.json();
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let certified = v
        .get("certified")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    let mut out = format!(
        "{}\n",
        if certified {
            "✓ 已认证"
        } else {
            "✗ 仍需修复"
        }
    );
    if let Some(checks) = v.get("checks").and_then(|x| x.as_array()) {
        out.push_str(&format_asset_checks(checks));
    }
    Ok(out)
}

/// `ratsa rth asset-delete <id>`
pub fn rth_asset_delete(client: &Client, id: u64) -> Result<String, String> {
    let resp = client.delete(&format!("/api/rth/profile/assets/{id}"))?;
    if let Some(e) = asset_error(&resp) {
        return Err(e);
    }
    Ok(format!("✓ 已删除资产 #{id}\n"))
}

/// `ratsa rth asset-download <id>`
pub fn rth_asset_download(client: &Client, id: u64, out: &str) -> Result<String, String> {
    let meta = client.get(&format!("/api/rth/profile/assets/{id}"))?;
    if let Some(e) = asset_error(&meta) {
        return Err(e);
    }
    let item = meta.json().get("item").cloned().unwrap_or(json!({}));
    let expected = item
        .get("checksum")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim_start_matches("sha256:")
        .to_string();
    let file_name = item
        .get("file_name")
        .and_then(|x| x.as_str())
        .filter(|n| !n.is_empty())
        .map(|n| n.to_string())
        .unwrap_or_else(|| format!("rth-asset-{id}.bin"));
    let (bytes, _headers) = client.download(&format!("/api/rth/assets/{id}/download"))?;
    let actual = sha256_hex(&bytes);
    if !expected.is_empty() && expected != actual {
        return Err(format!(
            "校验失败：服务端 {expected}，本地 {actual}（不要使用这个文件）"
        ));
    }
    let out_path = PathBuf::from(out);
    let target = if out_path.is_dir() || out.ends_with('/') {
        out_path.join(file_name)
    } else {
        out_path
    };
    if let Some(parent) = target.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| format!("无法创建 {}: {e}", parent.display()))?;
    }
    fs::write(&target, &bytes).map_err(|e| format!("无法写入 {}: {e}", target.display()))?;
    Ok(format!(
        "✓ 已下载 {}（{} 字节，sha256 校验通过）\n",
        target.display(),
        bytes.len()
    ))
}

/// `ratsa rth asset-revoke <id>`
pub fn rth_asset_revoke(client: &Client, id: u64, note: &str) -> Result<String, String> {
    let resp = client.post(
        &format!("/api/admin/rth/assets/{id}/revoke"),
        &json!({ "note": note }),
    )?;
    if let Some(e) = asset_error(&resp) {
        return Err(e);
    }
    Ok(format!("✓ 已撤销资产 #{id}\n"))
}

// ------------------------------------------------------------ rth-host (直连)

struct HostReply {
    status: u16,
    body: String,
}

impl HostReply {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|_| json!({ "raw": self.body }))
    }
}

fn resolve_host_target(platform: &Client, target: &str) -> Result<(String, String), String> {
    let target = target.trim();
    if target.is_empty() {
        return Err("请用 --host <id|name|url> 指定 Host".into());
    }
    if target.starts_with("http://") || target.starts_with("https://") {
        return Ok((target.trim_end_matches('/').to_string(), String::new()));
    }
    let item = if let Ok(id) = target.parse::<u64>() {
        let resp = platform.get(&format!("/api/rth/hosts/{id}"))?;
        if let Some(e) = asset_error(&resp) {
            return Err(format!("{e}（用 `ratsa host list` 查看已登记的 Host）"));
        }
        resp.json().get("item").cloned().unwrap_or(json!({}))
    } else {
        let resp = platform.get("/api/rth/hosts")?;
        if let Some(e) = asset_error(&resp) {
            return Err(e);
        }
        resp.json()
            .get("items")
            .and_then(|x| x.as_array())
            .and_then(|items| {
                items
                    .iter()
                    .find(|it| it.get("name").and_then(|n| n.as_str()) == Some(target))
                    .cloned()
            })
            .ok_or_else(|| format!("Host {target} 不存在（用 `ratsa host list` 查看）"))?
    };
    let url = item
        .get("url")
        .and_then(|x| x.as_str())
        .filter(|u| !u.is_empty())
        .ok_or_else(|| "平台返回的 Host 缺少 URL".to_string())?;
    Ok((
        url.trim_end_matches('/').to_string(),
        item.get("name")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
    ))
}

fn host_call(
    base: &str,
    token: Option<&str>,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> Result<HostReply, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout_read(std::time::Duration::from_secs(60))
        .user_agent(concat!("ratsa-harness/", env!("CARGO_PKG_VERSION")))
        .build();
    let url = format!("{base}{path}");
    let mut req = match method {
        "GET" => agent.get(&url),
        "POST" => agent.post(&url),
        "PUT" => agent.put(&url),
        "DELETE" => agent.delete(&url),
        other => return Err(format!("不支持的 Host 方法 {other}")),
    };
    if let Some(t) = token.filter(|t| !t.trim().is_empty()) {
        req = req.set("Authorization", &format!("Bearer {t}"));
    }
    let result = match body {
        Some(v) => req.send_json(v),
        None => req.call(),
    };
    let resp = match result {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(ureq::Error::Transport(t)) => {
            return Err(format!("无法连接 Host {base}：{t}"));
        }
    };
    let status = resp.status();
    let body = resp
        .into_string()
        .map_err(|e| format!("读取 Host 响应失败：{e}"))?;
    Ok(HostReply { status, body })
}

fn host_json(
    platform: &Client,
    target: &str,
    token: Option<&str>,
    path: &str,
    json_out: bool,
) -> Result<(String, Value), String> {
    let (base, _name) = resolve_host_target(platform, target)?;
    let reply = host_call(&base, token, "GET", path, None)?;
    if reply.status == 401 || reply.status == 403 {
        return Err(format!(
            "Host 拒绝访问（HTTP {}）。用 --token 或 RTH_HOST_TOKEN 提供 Host token。",
            reply.status
        ));
    }
    if reply.status >= 400 {
        return Err(format!(
            "Host 返回 HTTP {}：{}",
            reply.status,
            api::truncate(&reply.body, 300)
        ));
    }
    let v = reply.json();
    if json_out {
        return Ok((serde_json::to_string_pretty(&v).unwrap_or_default(), v));
    }
    Ok((String::new(), v))
}

fn finish_host(label: &str, v: &Value, json_out: bool, human: impl Fn(&Value) -> String) -> String {
    if json_out {
        serde_json::to_string_pretty(v).unwrap_or_default()
    } else {
        format!("{label}\n{}", human(v))
    }
}

/// `ratsa host add`
pub fn host_add(
    client: &Client,
    name: &str,
    url: &str,
    description: &str,
    capabilities: &str,
    json_out: bool,
) -> Result<String, String> {
    let caps = api::parse_scopes(&[capabilities.to_string()]);
    let resp = client.post(
        "/api/rth/hosts",
        &json!({ "name": name, "url": url, "description": description, "capabilities": caps }),
    )?;
    if let Some(e) = asset_error(&resp) {
        return Err(e);
    }
    let v = resp.json();
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let item = v.get("item").cloned().unwrap_or(json!({}));
    let caps = item
        .get("capabilities")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|c| c.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    Ok(format!(
        "✓ Host #{} {} → {}\n  capabilities: {}\n",
        item.get("id").and_then(|x| x.as_u64()).unwrap_or(0),
        item.get("name").and_then(|x| x.as_str()).unwrap_or("-"),
        item.get("url").and_then(|x| x.as_str()).unwrap_or("-"),
        caps
    ))
}

/// `ratsa host list`
pub fn host_list(client: &Client, json_out: bool) -> Result<String, String> {
    let resp = client.get("/api/rth/hosts")?;
    if let Some(e) = asset_error(&resp) {
        return Err(e);
    }
    let v = resp.json();
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let items = v
        .get("items")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    if items.is_empty() {
        return Ok(
            "还没有登记 Host。用 `ratsa host add --name bench --url https://…` 登记。\n".into(),
        );
    }
    let mut out = String::from(
        "ID    NAME                 URL                                      CAPABILITIES\n",
    );
    for it in items {
        let caps = it
            .get("capabilities")
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|c| c.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();
        out.push_str(&format!(
            "{:<5} {:<20} {:<40} {}\n",
            it.get("id").and_then(|x| x.as_u64()).unwrap_or(0),
            truncate_cells(it.get("name").and_then(|x| x.as_str()).unwrap_or(""), 18),
            truncate_cells(it.get("url").and_then(|x| x.as_str()).unwrap_or(""), 38),
            caps
        ));
    }
    Ok(out)
}

/// `ratsa host get`
pub fn host_get(client: &Client, id: u64, json_out: bool) -> Result<String, String> {
    let resp = client.get(&format!("/api/rth/hosts/{id}"))?;
    if let Some(e) = asset_error(&resp) {
        return Err(e);
    }
    let v = resp.json();
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let item = v.get("item").cloned().unwrap_or(json!({}));
    let caps = item
        .get("capabilities")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|c| c.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    Ok(format!(
        "#{} {}\nurl         : {}\ncapabilities: {}\n",
        item.get("id").and_then(|x| x.as_u64()).unwrap_or(0),
        item.get("name").and_then(|x| x.as_str()).unwrap_or("-"),
        item.get("url").and_then(|x| x.as_str()).unwrap_or("-"),
        caps
    ))
}

/// `ratsa host remove`
pub fn host_remove(client: &Client, id: u64) -> Result<String, String> {
    let resp = client.delete(&format!("/api/rth/hosts/{id}"))?;
    if let Some(e) = asset_error(&resp) {
        return Err(e);
    }
    Ok(format!("✓ 已删除 Host #{id}（Host Server 本身未受影响）\n"))
}

/// `ratsa host status`
pub fn host_status(
    platform: &Client,
    target: &str,
    token: Option<&str>,
    json_out: bool,
) -> Result<String, String> {
    let (base, name) = resolve_host_target(platform, target)?;
    let reply = host_call(&base, token, "GET", "/api/v1/host/status", None)?;
    if reply.status == 401 || reply.status == 403 {
        return Err(format!(
            "Host 拒绝访问（HTTP {}）。用 --token 或 RTH_HOST_TOKEN 提供 Host token。",
            reply.status
        ));
    }
    if reply.status >= 400 {
        return Err(format!(
            "Host 返回 HTTP {}：{}",
            reply.status,
            api::truncate(&reply.body, 300)
        ));
    }
    let v = reply.json();
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let caps = v
        .get("capabilities")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|c| c.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let label = if name.is_empty() {
        base.clone()
    } else {
        format!("{name} ({base})")
    };
    Ok(format!(
        "{label}\n  version     : {}\n  profile     : {}\n  rthd / rtm  : {} / {}\n  capabilities: {}\n  programs    : {}\n",
        v.get("version").and_then(|x| x.as_str()).unwrap_or("-"),
        v.get("profile").and_then(|x| x.as_str()).unwrap_or("-"),
        if v.get("rthd_reachable").and_then(|x| x.as_bool()).unwrap_or(false) { "✓" } else { "✗" },
        if v.get("rtm_reachable").and_then(|x| x.as_bool()).unwrap_or(false) { "✓" } else { "✗" },
        caps,
        v.get("programs").and_then(|x| x.as_array()).map(|a| a.len()).unwrap_or(0),
    ))
}

/// `ratsa host programs`
pub fn host_programs(
    platform: &Client,
    target: &str,
    token: Option<&str>,
    json_out: bool,
) -> Result<String, String> {
    let (base, name) = resolve_host_target(platform, target)?;
    let reply = host_call(&base, token, "GET", "/api/v1/programs", None)?;
    if reply.status >= 400 {
        return Err(format!(
            "Host 返回 HTTP {}：{}",
            reply.status,
            api::truncate(&reply.body, 300)
        ));
    }
    let v = reply.json();
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let items = v.as_array().cloned().unwrap_or_default();
    let label = if name.is_empty() {
        base
    } else {
        format!("{name} ({base})")
    };
    let mut out = format!("{label}\n");
    if items.is_empty() {
        out.push_str("  （没有程序）\n");
        return Ok(out);
    }
    out.push_str("  NAME                      STATE       HEALTH      PID      UPTIME\n");
    for it in items {
        out.push_str(&format!(
            "  {:<25} {:<11} {:<11} {:<8} {}\n",
            truncate_cells(it.get("name").and_then(|x| x.as_str()).unwrap_or(""), 23),
            it.get("state").and_then(|x| x.as_str()).unwrap_or("-"),
            it.get("health").and_then(|x| x.as_str()).unwrap_or("-"),
            it.get("pid")
                .and_then(|x| x.as_u64())
                .map(|p| p.to_string())
                .unwrap_or_else(|| "-".into()),
            it.get("uptime_secs").and_then(|x| x.as_u64()).unwrap_or(0),
        ));
    }
    Ok(out)
}

/// `ratsa host program`
pub fn host_program(
    platform: &Client,
    target: &str,
    program: &str,
    action: &str,
    token: Option<&str>,
    json_out: bool,
) -> Result<String, String> {
    let action = action.trim().to_ascii_lowercase();
    if !matches!(action.as_str(), "start" | "stop" | "restart") {
        return Err("--action 只支持 start | stop | restart".into());
    }
    let (base, name) = resolve_host_target(platform, target)?;
    let path = format!("/api/v1/programs/{}/{}", urlencode(program), action);
    let reply = host_call(&base, token, "POST", &path, None)?;
    if reply.status >= 400 {
        return Err(format!(
            "Host 返回 HTTP {}：{}",
            reply.status,
            api::truncate(&reply.body, 300)
        ));
    }
    if json_out {
        return Ok(serde_json::to_string_pretty(&reply.json()).unwrap_or_default());
    }
    let label = if name.is_empty() {
        base
    } else {
        format!("{name} ({base})")
    };
    Ok(format!("✓ {label} · {program} → {action}\n"))
}

/// `ratsa host events`
pub fn host_events(
    platform: &Client,
    target: &str,
    limit: usize,
    token: Option<&str>,
    json_out: bool,
) -> Result<String, String> {
    let path = format!("/api/v1/events?limit={limit}");
    let (human, v) = host_json(platform, target, token, &path, json_out)?;
    if json_out {
        return Ok(human);
    }
    Ok(finish_host("events", &v, false, |v| {
        serde_json::to_string_pretty(v).unwrap_or_default()
    }))
}

/// `ratsa host signals`
pub fn host_signals(
    platform: &Client,
    target: &str,
    limit: usize,
    token: Option<&str>,
    json_out: bool,
) -> Result<String, String> {
    let path = format!("/api/v1/signals?limit={limit}");
    let (human, v) = host_json(platform, target, token, &path, json_out)?;
    if json_out {
        return Ok(human);
    }
    Ok(finish_host("signals", &v, false, |v| {
        serde_json::to_string_pretty(v).unwrap_or_default()
    }))
}

/// `ratsa host signal`
pub fn host_signal(
    platform: &Client,
    target: &str,
    payload: &str,
    token: Option<&str>,
    json_out: bool,
) -> Result<String, String> {
    let raw = read_value_arg(payload)?;
    let body: Value =
        serde_json::from_str(&raw).map_err(|e| format!("--payload 不是合法 JSON：{e}"))?;
    let (base, name) = resolve_host_target(platform, target)?;
    let reply = host_call(&base, token, "POST", "/api/v1/signals", Some(body))?;
    let status = reply.status;
    // A signal rejected by device policy is a 409 with the decision record.
    if status >= 400 && status != 409 {
        return Err(format!(
            "Host 返回 HTTP {status}：{}",
            api::truncate(&reply.body, 300)
        ));
    }
    let v = reply.json();
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let label = if name.is_empty() {
        base
    } else {
        format!("{name} ({base})")
    };
    let accepted = v
        .get("accepted")
        .and_then(|x| x.as_bool())
        .unwrap_or(status < 400);
    Ok(format!(
        "{} {label} · signal → {}\n{}\n",
        if accepted {
            "✓ 已接受"
        } else {
            "✗ 被拒绝"
        },
        if accepted { "accepted" } else { "rejected" },
        serde_json::to_string_pretty(&v).unwrap_or_default()
    ))
}

/// `ratsa host harness`
pub fn host_harness(
    platform: &Client,
    target: &str,
    token: Option<&str>,
    json_out: bool,
) -> Result<String, String> {
    let (human, v) = host_json(platform, target, token, "/api/v1/harness", json_out)?;
    if json_out {
        return Ok(human);
    }
    Ok(finish_host("harness", &v, false, |v| {
        serde_json::to_string_pretty(v).unwrap_or_default()
    }))
}

/// `ratsa host agent`
pub fn host_agent(
    platform: &Client,
    target: &str,
    text: &str,
    source: &str,
    intent: Option<&str>,
    token: Option<&str>,
    json_out: bool,
) -> Result<String, String> {
    if text.trim().is_empty() {
        return Err("--text 不能为空".into());
    }
    let (base, name) = resolve_host_target(platform, target)?;
    let mut body = json!({ "text": text, "source": source });
    if let Some(intent) = intent.filter(|i| !i.trim().is_empty()) {
        body["intent"] = json!(intent);
    }
    let reply = host_call(
        &base,
        token,
        "POST",
        "/api/v1/harness/instruction",
        Some(body),
    )?;
    if reply.status >= 400 {
        return Err(format!(
            "Host 返回 HTTP {}：{}",
            reply.status,
            api::truncate(&reply.body, 300)
        ));
    }
    let v = reply.json();
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let label = if name.is_empty() {
        base
    } else {
        format!("{name} ({base})")
    };
    Ok(format!(
        "✓ {label} · agent instruction accepted\n{}\n",
        serde_json::to_string_pretty(&v).unwrap_or_default()
    ))
}

/// `ratsa host event`
pub fn host_event(
    platform: &Client,
    target: &str,
    event: &str,
    reason: Option<&str>,
    token: Option<&str>,
    json_out: bool,
) -> Result<String, String> {
    if event.trim().is_empty() {
        return Err("--event 不能为空".into());
    }
    let (base, name) = resolve_host_target(platform, target)?;
    let mut body = json!({ "event": event });
    if let Some(reason) = reason.filter(|r| !r.trim().is_empty()) {
        body["reason"] = json!(reason);
    }
    let reply = host_call(&base, token, "POST", "/api/v1/harness/event", Some(body))?;
    if reply.status >= 400 {
        return Err(format!(
            "Host 返回 HTTP {}：{}",
            reply.status,
            api::truncate(&reply.body, 300)
        ));
    }
    if json_out {
        return Ok(serde_json::to_string_pretty(&reply.json()).unwrap_or_default());
    }
    let label = if name.is_empty() {
        base
    } else {
        format!("{name} ({base})")
    };
    Ok(format!("✓ {label} · event {event}\n"))
}

/// `ratsa host tick`
pub fn host_tick(
    platform: &Client,
    target: &str,
    token: Option<&str>,
    json_out: bool,
) -> Result<String, String> {
    let (base, name) = resolve_host_target(platform, target)?;
    let reply = host_call(&base, token, "POST", "/api/v1/harness/tick", None)?;
    if reply.status >= 400 {
        return Err(format!(
            "Host 返回 HTTP {}：{}",
            reply.status,
            api::truncate(&reply.body, 300)
        ));
    }
    let v = reply.json();
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let label = if name.is_empty() {
        base
    } else {
        format!("{name} ({base})")
    };
    Ok(format!(
        "✓ {label} · tick\n{}\n",
        serde_json::to_string_pretty(&v).unwrap_or_default()
    ))
}

/// `ratsa rth asset-update <id>`
pub fn rth_asset_update(
    client: &Client,
    id: u64,
    description: Option<&str>,
    repo: Option<&str>,
    license: Option<&str>,
    visibility: Option<&str>,
    json_out: bool,
) -> Result<String, String> {
    let mut body = serde_json::Map::new();
    if let Some(v) = description {
        body.insert("description".into(), json!(v));
    }
    if let Some(v) = repo {
        body.insert("repository_url".into(), json!(v));
    }
    if let Some(v) = license {
        body.insert("license".into(), json!(v));
    }
    if let Some(v) = visibility {
        body.insert("visibility".into(), json!(v));
    }
    if body.is_empty() {
        return Err("至少提供一个字段：--description / --repo / --license / --visibility".into());
    }
    let resp = client.patch(
        &format!("/api/rth/profile/assets/{id}"),
        &Value::Object(body),
    )?;
    if let Some(e) = asset_error(&resp) {
        return Err(e);
    }
    let v = resp.json();
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    Ok(format_asset_detail(&v))
}

/// `ratsa host update <id>`
pub fn host_update(
    client: &Client,
    id: u64,
    name: Option<&str>,
    url: Option<&str>,
    description: Option<&str>,
    capabilities: Option<&str>,
    json_out: bool,
) -> Result<String, String> {
    let mut body = serde_json::Map::new();
    if let Some(v) = name {
        body.insert("name".into(), json!(v));
    }
    if let Some(v) = url {
        body.insert("url".into(), json!(v));
    }
    if let Some(v) = description {
        body.insert("description".into(), json!(v));
    }
    if let Some(v) = capabilities {
        body.insert(
            "capabilities".into(),
            json!(api::parse_scopes(&[v.to_string()])),
        );
    }
    if body.is_empty() {
        return Err("至少提供一个字段：--name / --url / --description / --capabilities".into());
    }
    let resp = client.put(&format!("/api/rth/hosts/{id}"), &Value::Object(body))?;
    if let Some(e) = asset_error(&resp) {
        return Err(e);
    }
    let v = resp.json();
    if json_out {
        return Ok(serde_json::to_string_pretty(&v).unwrap_or_default());
    }
    let item = v.get("item").cloned().unwrap_or(json!({}));
    let caps = item
        .get("capabilities")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|c| c.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    Ok(format!(
        "✓ Host #{} {} → {}\n  capabilities: {}\n",
        item.get("id").and_then(|x| x.as_u64()).unwrap_or(0),
        item.get("name").and_then(|x| x.as_str()).unwrap_or("-"),
        item.get("url").and_then(|x| x.as_str()).unwrap_or("-"),
        caps
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_content_type_covers_archives_and_sdks() {
        assert_eq!(
            upload_content_type(Path::new("plugin.zip")),
            "application/zip"
        );
        assert_eq!(
            upload_content_type(Path::new("sdk.tar.gz")),
            "application/gzip"
        );
        assert_eq!(upload_content_type(Path::new("sdk.whl")), "application/zip");
        assert_eq!(
            upload_content_type(Path::new("blob.bin")),
            "application/octet-stream"
        );
    }

    #[test]
    fn read_value_arg_supports_at_file() {
        let path = std::env::temp_dir().join(format!("ratsa-arg-{}.json", std::process::id()));
        fs::write(&path, "{\"a\":1}").unwrap();
        let got = read_value_arg(&format!("@{}", path.display())).unwrap();
        assert_eq!(got, "{\"a\":1}");
        let _ = fs::remove_file(path);
        assert_eq!(read_value_arg("plain").unwrap(), "plain");
    }

    #[test]
    fn asset_checks_mark_failures() {
        let checks = vec![
            json!({"id":"checksum","label":"SHA-256","ok":true}),
            json!({"id":"account","label":"账号认证","ok":false,"detail":"未认证"}),
        ];
        let out = format_asset_checks(&checks);
        assert!(out.contains("✓ SHA-256"));
        assert!(out.contains("✗ 账号认证 — 未认证"));
    }
}
