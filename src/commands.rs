//! Command implementations — the surface shared by the CLI and the MCP server.
//!
//! Keeping these in one module means the terminal experience and the Agent
//! experience can never drift apart: `ratsa-harness pull …` and the
//! `ratsa_pull_package` MCP tool run the exact same code.

use std::path::PathBuf;

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
    let path = if cfg.has_key() { "/api/v1/me" } else { "/api/me" };
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
        println!("账号      : {name}{}", if slug.is_empty() { String::new() } else { format!("  @{slug}") });
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
            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default();
        println!("权限      : {}", if scopes.is_empty() { "-".into() } else { scopes.join(", ") });
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
        println!("归属绑定  : @{}（未生效，请检查该 handle 是否存在）", cfg.bound_slug);
    }
    println!("服务端    : {}", cfg.base());
    let _ = cfg.save();
    Ok(())
}

/// `login` — exchange email/password for a JWT session (used to mint keys).
pub fn login(cfg: &mut Config, email: &str, password: &str, base_url: Option<&str>) -> Result<(), String> {
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
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
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
        groups.iter().filter_map(|x| x.as_str().map(String::from)).collect()
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
            let admin = s.get("admin_only").and_then(|x| x.as_bool()).unwrap_or(false);
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
    let path = if cfg.has_key() { "/api/v1/meta" } else { "/api/meta" };
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
    let items = v.get("items").and_then(|i| i.as_array()).cloned().unwrap_or_default();
    if items.is_empty() {
        println!("还没有 key。创建一个：ratsa-harness keys create --name agent --scopes sof:read,package:pull");
        return Ok(());
    }
    println!("{:<18} {:<16} {:<26} {:<12} {}", "KEY_ID", "名称", "权限", "归属绑定", "状态");
    for k in items {
        let kid = k.get("key_id").and_then(|x| x.as_str()).unwrap_or("");
        let name = k.get("name").and_then(|x| x.as_str()).unwrap_or("");
        let scopes: Vec<String> = k
            .get("scopes")
            .and_then(|s| s.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
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
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
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
        println!("已设为当前凭证（保存在 {}）", crate::config::config_path().display());
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
        "{:<14} {:<24} {:<12} {:<10} {}\n",
        "SLUG", "名称", "类目", "就绪度", "厂商"
    ));
    for it in rows {
        let slug = it.get("slug").and_then(|x| x.as_str()).unwrap_or("");
        let name = it.get("name").and_then(|x| x.as_str()).unwrap_or("");
        let cat = it.get("category").and_then(|x| x.as_str()).unwrap_or("");
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
            "{:<14} {:<24} {:<12} {:<10} {}\n",
            slug,
            truncate_cells(name, 22),
            truncate_cells(cat, 10),
            ready,
            vendor
        ));
    }
    out
}

pub fn list_sofs(client: &Client, query: &str, category: &str, limit: usize) -> Result<String, String> {
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
    if pkgs.is_empty() {
        return Ok(format!(
            "设备 {slug} 目前没有可拉取的 Harness Package。\n（厂商尚未发布 Harness / Eval Repo / 服务测试包，或可见性未对你开放）"
        ));
    }
    let mut out = format!("设备 {slug} 的 Harness Package（{} 个）\n\n", pkgs.len());
    for p in pkgs {
        let kind = p.get("package_kind").and_then(|x| x.as_str()).unwrap_or("");
        let name = p.get("name").and_then(|x| x.as_str()).unwrap_or("");
        let id = p.get("id").and_then(|x| x.as_str()).unwrap_or("");
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
        if !by.is_empty() {
            out.push_str(&format!("    厂商: {by}\n"));
        }
        if !pull_slug.is_empty() {
            out.push_str(&format!(
                "    拉取: ratsa-harness pull {pull_slug} {flag}\n"
            ));
        }
    }
    Ok(out)
}

// --------------------------------------------------------------------- pull

/// Download a Harness Package into `~/.ratsa/packages/<slug>/` (or `out_dir`).
pub fn pull_report(client: &Client, slug: &str, kind: &str, out_dir: &str) -> Result<String, String> {
    let kind = match kind {
        "" => "device",
        k @ ("device" | "eval-repo" | "service") => k,
        other => return Err(format!("未知 kind「{other}」，可选 device | eval-repo | service")),
    };
    let path = match kind {
        "device" => format!("/api/harness/sof/{slug}/package"),
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
    out.push_str(&format!("  解包  : unzip -o {} -d {}\n", file.display(), dir.display()));
    Ok(out)
}

fn append_artifact_meta(out: &mut String, headers: &Headers) {
    if !headers.checksum.is_empty() {
        out.push_str(&format!("  校验  : sha256 {}（已本地复算通过）\n", headers.checksum));
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
    meta_only: bool,
    out_dir: &str,
) -> Result<String, String> {
    let path = format!("/api/harness/sof/{slug}/package");
    if !meta_only {
        let out = pull_report(client, slug, "device", out_dir)?;
        return Ok(format!(
            "{out}\n提示：SOF 文件（可回灌的规范文档）用 `ratsa sof-file {slug}` 单独拉取。\n"
        ));
    }
    let resp = client.get(&format!("{path}?meta=1"))?;
    if resp.status >= 400 {
        return Err(resp.error_message());
    }
    let v = resp.json();
    let pkg = v.get("package").cloned().unwrap_or(json!({}));
    let rd = v.get("readiness").cloned().unwrap_or(json!({}));
    let mut out = String::new();
    out.push_str(&format!(
        "Harness Package（{}）· 设备 {slug}\n",
        v.get("package_kind").and_then(|x| x.as_str()).unwrap_or("device-harness")
    ));
    let id = pkg.get("id").and_then(|x| x.as_str()).unwrap_or("");
    let ver = pkg.get("version").and_then(|x| x.as_str()).unwrap_or("");
    if !id.is_empty() {
        out.push_str(&format!("  id/version : {id} @ {ver}\n"));
    }
    let score = rd.get("score").map(|s| s.to_string()).unwrap_or_else(|| "-".into());
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
            let p = f.get("path").or_else(|| f.get("name")).and_then(|x| x.as_str()).unwrap_or("");
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
pub fn sof_file_cmd(client: &Client, slug: &str, out_dir: &str) -> Result<String, String> {
    let dir = if out_dir.is_empty() {
        crate::config::home_dir().join("sof")
    } else {
        PathBuf::from(out_dir)
    };
    let (file, size, _h) = client.download_to(
        &format!("/api/sof/{slug}/file"),
        &dir,
        &format!("{slug}.sof.json"),
    )?;
    let mut out = format!("已拉取 SOF 文件（{slug}）\n");
    out.push_str(&format!("  落盘 : {}\n", file.display()));
    out.push_str(&format!("  大小 : {size} 字节\n"));
    out.push_str("  内容 : kind=ratsa.sof.json，`component` 与 `POST/PUT /api/sof` 同构（可直接回灌）\n");
    out.push_str(&format!("  回灌 : 见包内 tools/import-sof.py，或 POST /api/sof（需 sof:write 权限）\n"));
    Ok(out)
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
        checks.push(json!({"name": "failed", "status": if f == 0 { "pass" } else { "fail" }, "value": f}));
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

fn sha256_hex(data: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, data);
    let mut out = String::with_capacity(digest.as_ref().len() * 2);
    for b in digest.as_ref() {
        out.push_str(&format!("{b:02x}"));
    }
    out
}
