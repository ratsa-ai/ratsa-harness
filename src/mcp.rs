//! Minimal MCP (Model Context Protocol) server over stdio.
//!
//! Deliberately dependency-free: MCP's stdio transport is newline-delimited
//! JSON-RPC 2.0, which is little enough protocol to implement directly. That
//! keeps RATSA-Harness installable on a locked-down machine (no async runtime,
//! no network stack beyond `ureq`) and auditable — which matters because the
//! server carries the user's API key.
//!
//! Exposed tools are the whole customer-side loop: discover → read → pull →
//! run → report. Every tool maps to a documented public HTTP endpoint; nothing
//! here talks to an internal service.

use std::io::{self, BufRead, Write};

use serde_json::{json, Value};

use crate::api::{self, Client};
use crate::commands;
use crate::config::Config;

pub const PROTOCOL_VERSION: &str = "2024-11-05";

pub fn serve() -> Result<(), String> {
    let stdin = io::stdin();
    let mut out = io::stdout();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                let resp = json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": {"code": -32700, "message": format!("JSON 解析失败：{e}")}
                });
                write_msg(&mut out, &resp)?;
                continue;
            }
        };
        // Notifications (no id) never get a response.
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        if id.is_none() {
            continue;
        }
        let params = msg.get("params").cloned().unwrap_or(json!({}));
        let result = dispatch(method, &params);
        let resp = match result {
            Ok(v) => json!({"jsonrpc": "2.0", "id": id, "result": v}),
            Err(e) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": e}}),
        };
        write_msg(&mut out, &resp)?;
    }
    Ok(())
}

fn write_msg(out: &mut io::Stdout, msg: &Value) -> Result<(), String> {
    let mut line = serde_json::to_string(msg).map_err(|e| e.to_string())?;
    line.push('\n');
    out.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    out.flush().map_err(|e| e.to_string())
}

fn dispatch(method: &str, params: &Value) -> Result<Value, String> {
    match method {
        "initialize" => {
            let version = params
                .get("protocolVersion")
                .and_then(|v| v.as_str())
                .unwrap_or(PROTOCOL_VERSION);
            Ok(json!({
                "protocolVersion": version,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {
                    "name": "ratsa-harness",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "instructions": "RATSA.ai 平台接入：查看身份与权限、检索 SOF、拉取 Harness Package（设备 Harness / Eval Repo / 服务测试包）、回传本地运行结果与反馈。凭证来自 ~/.ratsa/config.json。"
            }))
        }
        "ping" => Ok(json!({})),
        "notifications/initialized" | "notifications/cancelled" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => call_tool(params),
        "resources/list" => Ok(json!({ "resources": [] })),
        "prompts/list" => Ok(json!({ "prompts": [] })),
        other => Err(format!("不支持的方法：{other}")),
    }
}

fn tools() -> Vec<Value> {
    let str_prop = |desc: &str| json!({"type": "string", "description": desc});
    vec![
        json!({
            "name": "ratsa_whoami",
            "description": "查看当前 RATSA 身份：账号、key 名称、权限表（scopes）、以及该 key 绑定的归属账号。未配置凭证时返回配置指引。",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
        }),
        json!({
            "name": "ratsa_scopes",
            "description": "列出 RATSA 权限表（可授予 key 的全部 scope 及其含义）与默认集合，用于生成最小权限的 key。",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
        }),
        json!({
            "name": "ratsa_search_sofs",
            "description": "检索 SOF（设备/零部件）目录。返回 slug、名称、类目、就绪度与厂商。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": str_prop("关键字（名称/描述），留空返回全部"),
                    "category": str_prop("类目过滤，如 核心零部件"),
                    "limit": {"type": "integer", "description": "最多返回条数，默认 20"}
                },
                "additionalProperties": false
            }
        }),
        json!({
            "name": "ratsa_read_sof",
            "description": "读取一个 SOF 的完整信息（规格、认证、Agentic Spec、资源链接）。",
            "inputSchema": {
                "type": "object",
                "properties": {"slug": str_prop("SOF 的 slug，如 tgkgz48zc4")},
                "required": ["slug"], "additionalProperties": false
            }
        }),
        json!({
            "name": "ratsa_device_harness",
            "description": "读取设备 Harness：派生 manifest、就绪度（readiness 分数 + 问题清单）、以及该设备下的统一 packages 清单（设备 Harness 包 / Eval Repo / 服务测试包）。",
            "inputSchema": {
                "type": "object",
                "properties": {"slug": str_prop("SOF 的 slug")},
                "required": ["slug"], "additionalProperties": false
            }
        }),
        json!({
            "name": "ratsa_list_packages",
            "description": "只列出某设备可拉取的 Harness Package 清单（含 package_kind、下载地址、版本、厂商）。",
            "inputSchema": {
                "type": "object",
                "properties": {"slug": str_prop("SOF 的 slug")},
                "required": ["slug"], "additionalProperties": false
            }
        }),
        json!({
            "name": "ratsa_pull_package",
            "description": "把 Harness Package 下载到本地目录（默认 ~/.ratsa/packages/<slug>）。kind=device 拉设备 Harness 包，kind=eval-repo 拉评测仓库，kind=service 拉服务测试包。返回实际落盘路径。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "slug": str_prop("SOF / Eval Repo / 服务 的 slug"),
                    "kind": {"type": "string", "enum": ["device", "eval-repo", "service"], "description": "默认 device"},
                    "out_dir": str_prop("可选：落盘目录")
                },
                "required": ["slug"], "additionalProperties": false
            }
        }),
        json!({
            "name": "ratsa_list_agents",
            "description": "列出 RATSA-Harness 支持的 Agent 接入目标（id、形态、写入哪些文件、是否注册 MCP）以及安装命令。当用户问“怎么把 RATSA 接到我的 Agent”时用它。",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
        }),
        json!({
            "name": "ratsa_generate_package",
            "description": "让平台为某个 SOF 生成设备 Harness 包（kind=device-harness，内含 sof.json / ratsa.harness.json / harness/harness.py / manifest.json）。meta=true 时只返回描述（包内文件、就绪度、大小），不下载。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "slug": str_prop("SOF 的 slug"),
                    "meta": {"type": "boolean", "description": "true = 只看描述不下载"},
                    "out_dir": str_prop("可选：落盘目录")
                },
                "required": ["slug"], "additionalProperties": false
            }
        }),
        json!({
            "name": "ratsa_get_sof_file",
            "description": "拉取 SOF 文件（规范化文档 ratsa.sof.json，component 与 POST /api/sof 同构，可直接回灌）。厂商用它备份/比对/回灌自己的 SOF。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "slug": str_prop("SOF 的 slug"),
                    "out_dir": str_prop("可选：落盘目录")
                },
                "required": ["slug"], "additionalProperties": false
            }
        }),
        json!({
            "name": "ratsa_report_run",
            "description": "把本地跑完的结果回传为一次实测 Harness Eval（source=agent）。需要 key 具备 eval:report 权限。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "slug": str_prop("被测设备的 SOF slug"),
                    "score": {"type": "number", "description": "0-100 的实测分数"},
                    "passed": {"type": "integer", "description": "通过用例数"},
                    "failed": {"type": "integer", "description": "失败用例数"},
                    "summary": str_prop("一句话结论"),
                    "notes": str_prop("补充说明 / 失败详情")
                },
                "required": ["slug", "score"], "additionalProperties": false
            }
        }),
        json!({
            "name": "ratsa_submit_feedback",
            "description": "向厂商提交一条 Feedback（bug / 质量 / 兼容性等）。需要 key 具备 feedback:submit 权限。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "slug": str_prop("SOF slug"),
                    "title": str_prop("标题"),
                    "detail": str_prop("详细描述"),
                    "category": {"type": "string", "enum": ["bug", "quality", "feature", "compatibility", "support"]},
                    "severity": {"type": "string", "enum": ["low", "medium", "high", "critical"]}
                },
                "required": ["slug", "title"], "additionalProperties": false
            }
        }),
    ]
}

fn call_tool(params: &Value) -> Result<Value, String> {
    let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    let client = Client::new(&Config::load());
    let text = match run_tool(name, &args, &client) {
        Ok(t) => t,
        Err(e) => {
            return Ok(json!({
                "content": [{"type": "text", "text": e}],
                "isError": true
            }))
        }
    };
    Ok(json!({
        "content": [{"type": "text", "text": text}],
        "isError": false
    }))
}

fn arg_str(args: &Value, key: &str) -> String {
    args.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn run_tool(name: &str, args: &Value, client: &Client) -> Result<String, String> {
    match name {
        "ratsa_whoami" => {
            if !client_has_creds(client) {
                return Ok(commands::cli_not_configured_hint());
            }
            let me = client.get("/api/v1/me")?;
            if me.status >= 400 {
                return Err(me.error_message());
            }
            Ok(serde_json::to_string_pretty(&me.json()).unwrap_or_default())
        }
        "ratsa_scopes" => {
            let meta = client.get("/api/v1/meta")?;
            Ok(commands::format_scope_table(&meta.json()))
        }
        "ratsa_search_sofs" => {
            let query = arg_str(args, "query");
            let category = arg_str(args, "category");
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20);
            let body = commands::list_sofs(client, &query, &category, limit as usize)?;
            Ok(body)
        }
        "ratsa_read_sof" => {
            let slug = api::slug_or_url(&arg_str(args, "slug"));
            let resp = client.get(&format!("/api/sof/{slug}"))?;
            if resp.status >= 400 {
                return Err(resp.error_message());
            }
            Ok(serde_json::to_string_pretty(&resp.json()).unwrap_or_default())
        }
        "ratsa_device_harness" => {
            let slug = api::slug_or_url(&arg_str(args, "slug"));
            let resp = client.get(&format!("/api/harness/sof/{slug}"))?;
            if resp.status >= 400 {
                return Err(resp.error_message());
            }
            Ok(serde_json::to_string_pretty(&resp.json()).unwrap_or_default())
        }
        "ratsa_list_packages" => {
            let slug = api::slug_or_url(&arg_str(args, "slug"));
            let resp = client.get(&format!("/api/harness/sof/{slug}"))?;
            if resp.status >= 400 {
                return Err(resp.error_message());
            }
            commands::format_packages(&resp.json())
        }
        "ratsa_pull_package" => {
            let slug = api::slug_or_url(&arg_str(args, "slug"));
            let kind = {
                let k = arg_str(args, "kind");
                if k.is_empty() { "device".to_string() } else { k }
            };
            let out = arg_str(args, "out_dir");
            commands::pull_report(client, &slug, &kind, &out)
        }
        "ratsa_report_run" => {
            let slug = api::slug_or_url(&arg_str(args, "slug"));
            let score = args
                .get("score")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| "缺少 score（0-100）".to_string())?;
            let passed = args.get("passed").and_then(|v| v.as_i64());
            let failed = args.get("failed").and_then(|v| v.as_i64());
            let summary = arg_str(args, "summary");
            let notes = arg_str(args, "notes");
            commands::report_run(
                client,
                &slug,
                score,
                passed,
                failed,
                &summary,
                &notes,
                "agent",
            )
        }
        "ratsa_submit_feedback" => {
            let slug = api::slug_or_url(&arg_str(args, "slug"));
            let title = arg_str(args, "title");
            let detail = arg_str(args, "detail");
            let category = arg_str(args, "category");
            let severity = arg_str(args, "severity");
            commands::submit_feedback(client, &slug, &title, &detail, &category, &severity)
        }
        "ratsa_list_agents" => {
            let rows: Vec<Value> = crate::install::target_table()
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
            Ok(serde_json::to_string_pretty(&json!({
                "install": "ratsa install --agent all",
                "global": "ratsa install --agent all --global",
                "agents": rows,
            }))
            .unwrap_or_default())
        }
        "ratsa_generate_package" => {
            let slug = api::slug_or_url(&arg_str(args, "slug"));
            let meta = args.get("meta").and_then(|v| v.as_bool()).unwrap_or(false);
            let out = arg_str(args, "out_dir");
            commands::package_cmd(client, &slug, meta, &out)
        }
        "ratsa_get_sof_file" => {
            let slug = api::slug_or_url(&arg_str(args, "slug"));
            let out = arg_str(args, "out_dir");
            commands::sof_file_cmd(client, &slug, &out)
        }
        other => Err(format!("未知工具：{other}")),
    }
}

fn client_has_creds(c: &Client) -> bool {
    !c.key_id.is_empty() && !c.secret.is_empty()
}
