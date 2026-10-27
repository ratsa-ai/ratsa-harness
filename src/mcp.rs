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
            "name": "ratsa_avatar",
            "description": "查看 / 上传 / 移除当前账号的头像。file 为本地图片路径（PNG/JPEG/GIF/WebP，≤5MB）；clear=true 移除自定义头像并恢复 GitHub 头像；不传参数则返回当前头像 URL。上传与移除需要 `ratsa login` 会话。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file": str_prop("本地头像图片路径；省略则只查看"),
                    "clear": {"type": "boolean", "description": "移除自定义头像（恢复 GitHub 头像）"}
                },
                "additionalProperties": false
            }
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
                "properties": {
                    "slug": str_prop("SOF 的 slug，如 tgkgz48zc4"),
                    "version": str_prop("可选：历史版本（semver，如 1.0.0；省略 = 当前版本）")
                },
                "required": ["slug"], "additionalProperties": false
            }
        }),
        json!({
            "name": "ratsa_rth_platform",
            "description": "读取官方 RTH Platform：运行方式（RTH Layer / RTH Application / HAL contribution）与 BSP / Suite / SDK / HAL / OS 包家族。",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
        }),
        json!({
            "name": "ratsa_list_rth_releases",
            "description": "列出一个 RTH 项目的 Release（版本 / 通道 / 模式 / 状态）。",
            "inputSchema": {
                "type": "object",
                "properties": {"slug": str_prop("RTH 项目 slug")},
                "required": ["slug"], "additionalProperties": false
            }
        }),
        json!({
            "name": "ratsa_read_rth_release",
            "description": "读取一个 RTH Release 及其 Package、下载与验签地址。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "slug": str_prop("RTH 项目 slug"),
                    "version": str_prop("Release 版本（semver）")
                },
                "required": ["slug", "version"], "additionalProperties": false
            }
        }),
        json!({
            "name": "ratsa_rth_spec",
            "description": "读取 RTH（Ratsa Runtime Harness）规范：RTHLoop/服务面、能力、签名与准入流程、Use Cases。",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
        }),
        json!({
            "name": "ratsa_list_rth_projects",
            "description": "列出公开 RTH 项目（RTHLoop 版本、状态、Agentic 验证状态）。",
            "inputSchema": {
                "type": "object",
                "properties": {"q": str_prop("可选：名称 / 简介关键字")},
                "additionalProperties": false
            }
        }),
        json!({
            "name": "ratsa_read_rth_project",
            "description": "读取一个 RTH 项目：RTHLoop 版本、签名公钥、Package、SOF/Lab 关联与 Agentic 验证结果。",
            "inputSchema": {
                "type": "object",
                "properties": {"slug": str_prop("RTH 项目 slug")},
                "required": ["slug"], "additionalProperties": false
            }
        }),
        json!({
            "name": "ratsa_rth_channel",
            "description": "读取 RATSA AI → RTH → 设备的签名通道描述（含已准入 Package 与验证入口）。",
            "inputSchema": {
                "type": "object",
                "properties": {"slug": str_prop("RTH 项目 slug")},
                "required": ["slug"], "additionalProperties": false
            }
        }),
        json!({
            "name": "ratsa_verify_rth_package",
            "description": "验证一个 RTH Package 的签名、校验和与准入状态。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "slug": str_prop("RTH 项目 slug"),
                    "package_id": {"type": "integer", "description": "Package ID"}
                },
                "required": ["slug", "package_id"], "additionalProperties": false
            }
        }),
        json!({
            "name": "ratsa_list_sof_versions",
            "description": "列出某个 SOF 的版本历史（版本号、说明、时间、当前版本）。配合 version 参数可读取或拉取任意历史版本。",
            "inputSchema": {
                "type": "object",
                "properties": {"slug": str_prop("SOF 的 slug")},
                "required": ["slug"], "additionalProperties": false
            }
        }),
        json!({
            "name": "ratsa_device_harness",
            "description": "读取设备 Harness：派生 manifest、就绪度（readiness 分数 + 问题清单）、以及该设备下的统一 packages 清单（设备 Harness 包 / Eval Repo / 服务测试包）。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "slug": str_prop("SOF 的 slug"),
                    "version": str_prop("可选：历史版本（semver；省略 = 当前版本）")
                },
                "required": ["slug"], "additionalProperties": false
            }
        }),
        json!({
            "name": "ratsa_list_packages",
            "description": "只列出某设备可拉取的 Harness Package 清单（含 package_kind、下载地址、版本、厂商）。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "slug": str_prop("SOF 的 slug"),
                    "version": str_prop("可选：历史版本（semver；省略 = 当前版本）")
                },
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
                    "version": str_prop("可选：设备 Harness 包的历史版本（semver；省略 = 当前版本）"),
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
                    "version": str_prop("可选：历史版本（semver；省略 = 当前版本）"),
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
                    "version": str_prop("可选：历史版本（semver；省略 = 当前版本）"),
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
        json!({
            "name": "ratsa_list_kb",
            "description": "列出厂商 SOF 知识库（Markdown 文档索引：文档键 / 级别 / 标题 / 分组 / checksum）。知识库不对外开放，需要 kb:read 且 key 属于该厂商账号或绑定到该账号。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "sof": str_prop("可选：设备 slug（省略 = 账号级：common + scope）"),
                    "type": {"type": "string", "enum": ["common", "scope", "sof"],
                             "description": "可选：只看某一类（common 公共/领域基础、scope 用户 Scope/账号介绍、sof 设备）"}
                },
                "required": [], "additionalProperties": false
            }
        }),
        json!({
            "name": "ratsa_read_kb",
            "description": "读取 SOF 知识库里的一篇 Markdown 原文（doc 可传文档键 key 或 slug）。需要 kb:read，且 key 属于该厂商账号或绑定到该账号。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "doc": str_prop("文档键 key（如 quickstart）或文档 slug"),
                    "sof": str_prop("可选：设备 slug（省略 = 账号级：common + scope）"),
                    "type": {"type": "string", "enum": ["common", "scope", "sof"],
                             "description": "可选：只在某一类里查找（同 ratsa_list_kb）"}
                },
                "required": ["doc"], "additionalProperties": false
            }
        }),
        json!({
            "name": "ratsa_list_labs",
            "description": "列出本账号（或 API key 绑定账号）名下的 RATSA Lab（Cloud Lab）。Lab 完全私有，没有公开列表；需要 lab:read。",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
        }),
        json!({
            "name": "ratsa_read_lab",
            "description": "读取一个 Cloud Lab 的完整档案：领域、审核状态、Agent 接入能力、资料、可提供的服务与过往服务内容。需要 lab:read。",
            "inputSchema": {
                "type": "object",
                "properties": {"slug": str_prop("Lab 的 slug")},
                "required": ["slug"], "additionalProperties": false
            }
        }),
        json!({
            "name": "ratsa_apply_lab",
            "description": "创建 / 更新本账号的 Cloud Lab 档案；submit=true 时同时提交平台审核（需先上传至少一份资料）。需要 lab:write，且账号为付费的 bio/chem/env/materials 属性。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "profile": {"type": "object", "description": "Lab 档案（name / vertical / description / location / contact_email / capabilities / agent_capability…）"},
                    "submit": {"type": "boolean", "description": "是否提交审核"},
                    "note": str_prop("申请备注（可选）")
                },
                "required": ["profile"], "additionalProperties": false
            }
        }),
        json!({
            "name": "ratsa_upload_lab_document",
            "description": "把本地资质 / 证书 / SOP / 设备清单上传到本账号的 Cloud Lab。需要 lab:write。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "slug": str_prop("Lab 的 slug"),
                    "path": str_prop("本地文件路径"),
                    "kind": {"type": "string", "enum": ["license", "certificate", "sop", "equipment", "other"]}
                },
                "required": ["slug", "path"], "additionalProperties": false
            }
        }),
        json!({
            "name": "ratsa_list_lab_services",
            "description": "列出某个 Cloud Lab 可提供的服务（类别、周期、样本要求、交付物、Agent 是否可调用）。需要 lab:read。",
            "inputSchema": {
                "type": "object",
                "properties": {"slug": str_prop("Lab 的 slug")},
                "required": ["slug"], "additionalProperties": false
            }
        }),
        json!({
            "name": "ratsa_list_lab_records",
            "description": "列出某个 Cloud Lab 的过往服务内容 / 实绩。需要 lab:read。",
            "inputSchema": {
                "type": "object",
                "properties": {"slug": str_prop("Lab 的 slug")},
                "required": ["slug"], "additionalProperties": false
            }
        }),
        json!({
            "name": "ratsa_delete_lab",
            "description": "删除本账号（或绑定账号）的 Cloud Lab，连同资料、服务与过往服务记录；审核中不可删除。需要 lab:write。",
            "inputSchema": {
                "type": "object",
                "properties": {"slug": str_prop("Lab 的 slug")},
                "required": ["slug"], "additionalProperties": false
            }
        }),
        json!({
            "name": "ratsa_list_lab_devices",
            "description": "列出某个 Cloud Lab 绑定的 RATSA 设备及其运行状态（在用 / 空闲 / 维护中 / 停用），含厂商与最近确认时间。需要 lab:read。",
            "inputSchema": {
                "type": "object",
                "properties": {"slug": str_prop("Lab 的 slug")},
                "required": ["slug"], "additionalProperties": false
            }
        }),
        json!({
            "name": "ratsa_add_lab_device",
            "description": "把 RATSA AI 目录中的一台设备绑定到 Lab（Lab 之后维护其运行状态；设备资料仍归厂商 SOF）。需要 lab:write，且 Lab 已通过审核。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "slug": str_prop("Lab 的 slug"),
                    "sof_slug": str_prop("设备的 SOF slug（可先用 ratsa_list_lab_vendors 查）"),
                    "model": str_prop("型号 / 编号（可选）"),
                    "note": str_prop("备注（可选）"),
                    "status": {"type": "string", "enum": ["active", "idle", "maintenance", "offline"]}
                },
                "required": ["slug", "sof_slug"], "additionalProperties": false
            }
        }),
        json!({
            "name": "ratsa_update_lab_device",
            "description": "维护 Lab 内某台设备的运行状态 / 备注（会刷新 last_checked_at）。需要 lab:write。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "slug": str_prop("Lab 的 slug"),
                    "id": {"type": "integer", "description": "设备 id（见 ratsa_list_lab_devices）"},
                    "status": {"type": "string", "enum": ["active", "idle", "maintenance", "offline"]},
                    "note": str_prop("备注（可选）")
                },
                "required": ["slug", "id"], "additionalProperties": false
            }
        }),
        json!({
            "name": "ratsa_list_lab_vendors",
            "description": "从 RATSA AI 目录检索设备厂商与可绑定设备（公开设备 + 本账号自己的设备）。需要 lab:read。",
            "inputSchema": {
                "type": "object",
                "properties": {"q": str_prop("关键字：设备名 / 厂商 / 类目（可选）")},
                "additionalProperties": false
            }
        }),
        json!({
            "name": "ratsa_list_lab_feedback",
            "description": "列出 Lab 的客户服务交易反馈，并返回资质 / 营销汇总（交易数、已核验、平均评分、可展示案例、是否达到参考门槛）。需要 lab:read。",
            "inputSchema": {
                "type": "object",
                "properties": {"slug": str_prop("Lab 的 slug")},
                "required": ["slug"], "additionalProperties": false
            }
        }),
        json!({
            "name": "ratsa_add_lab_feedback",
            "description": "用实验室账号登记一笔客户服务交易（平台核验后可作为 Lab 资质证据；consent=true 时可用于营销案例）。需要 lab:write，且 Lab 已通过审核。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "slug": str_prop("Lab 的 slug"),
                    "customer_name": str_prop("客户联系人"),
                    "customer_company": str_prop("客户公司"),
                    "customer_contact": str_prop("客户联系方式"),
                    "service_id": {"type": "integer", "description": "关联的 Lab 服务 id（可选）"},
                    "order_ref": str_prop("订单 / 合同号"),
                    "amount": {"type": "number", "description": "金额"},
                    "currency": str_prop("币种，默认 CNY"),
                    "rating": {"type": "integer", "description": "评分 0-5（0 = 暂不评分）"},
                    "category": {"type": "string", "enum": ["quality", "turnaround", "support", "other"]},
                    "title": str_prop("一句话结论"),
                    "detail": str_prop("详情"),
                    "occurred_at": str_prop("交易日期 YYYY-MM-DD"),
                    "consent": {"type": "boolean", "description": "客户同意作为公开案例"}
                },
                "required": ["slug"], "additionalProperties": false
            }
        }),
        json!({
            "name": "ratsa_lab_feedback_summary",
            "description": "读取 Lab 的资质 / 营销汇总与可展示案例。需要 lab:read。",
            "inputSchema": {
                "type": "object",
                "properties": {"slug": str_prop("Lab 的 slug")},
                "required": ["slug"], "additionalProperties": false
            }
        }),
        json!({
            "name": "ratsa_read_lab_spec",
            "description": "读取 Cloud Lab 的机器可读契约 ratsa.lab.json（含 Agent 协议、端点、服务与实绩），供 Agent / 云端集成直接消费。需要 lab:read。",
            "inputSchema": {
                "type": "object",
                "properties": {"slug": str_prop("Lab 的 slug")},
                "required": ["slug"], "additionalProperties": false
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
        "ratsa_avatar" => {
            if !client_has_creds(client) {
                return Ok(commands::cli_not_configured_hint());
            }
            let file = arg_str(args, "file");
            let clear = args.get("clear").and_then(|v| v.as_bool()).unwrap_or(false);
            commands::profile_avatar(
                client,
                if file.trim().is_empty() {
                    None
                } else {
                    Some(&file)
                },
                clear,
                true,
            )
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
        "ratsa_rth_platform" => commands::rth_platform(client, false),
        "ratsa_list_rth_releases" => {
            let slug = api::slug_or_url(&arg_str(args, "slug"));
            commands::rth_releases(client, &slug, false)
        }
        "ratsa_read_rth_release" => {
            let slug = api::slug_or_url(&arg_str(args, "slug"));
            commands::rth_release(client, &slug, &arg_str(args, "version"), false)
        }
        "ratsa_rth_spec" => commands::rth_spec(client, false),
        "ratsa_list_rth_projects" => commands::rth_projects(client, &arg_str(args, "q"), false),
        "ratsa_read_rth_project" => {
            let slug = api::slug_or_url(&arg_str(args, "slug"));
            commands::rth_get(client, &slug, false)
        }
        "ratsa_rth_channel" => {
            let slug = api::slug_or_url(&arg_str(args, "slug"));
            commands::rth_channel(client, &slug, false)
        }
        "ratsa_verify_rth_package" => {
            let slug = api::slug_or_url(&arg_str(args, "slug"));
            let id = args
                .get("package_id")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| "缺少 package_id".to_string())?;
            commands::rth_verify(client, &slug, id, false)
        }
        "ratsa_read_sof" => {
            let slug = api::slug_or_url(&arg_str(args, "slug"));
            let path =
                commands::with_version(&format!("/api/sof/{slug}"), &arg_str(args, "version"));
            let resp = client.get(&path)?;
            if resp.status >= 400 {
                return Err(resp.error_message());
            }
            Ok(serde_json::to_string_pretty(&resp.json()).unwrap_or_default())
        }
        "ratsa_list_sof_versions" => {
            let slug = api::slug_or_url(&arg_str(args, "slug"));
            let resp = client.get(&format!("/api/sof/{slug}/versions"))?;
            if resp.status >= 400 {
                return Err(resp.error_message());
            }
            Ok(serde_json::to_string_pretty(&resp.json()).unwrap_or_default())
        }
        "ratsa_device_harness" => {
            let slug = api::slug_or_url(&arg_str(args, "slug"));
            let path = commands::with_version(
                &format!("/api/harness/sof/{slug}"),
                &arg_str(args, "version"),
            );
            let resp = client.get(&path)?;
            if resp.status >= 400 {
                return Err(resp.error_message());
            }
            Ok(serde_json::to_string_pretty(&resp.json()).unwrap_or_default())
        }
        "ratsa_list_kb" => {
            if !client_has_creds(client) {
                return Ok(commands::cli_not_configured_hint());
            }
            commands::kb_index(client, &arg_str(args, "sof"), &arg_str(args, "type"), false)
        }
        "ratsa_read_kb" => {
            if !client_has_creds(client) {
                return Ok(commands::cli_not_configured_hint());
            }
            commands::kb_read(
                client,
                &arg_str(args, "sof"),
                &arg_str(args, "type"),
                &arg_str(args, "doc"),
                "",
            )
        }
        "ratsa_list_packages" => {
            let slug = api::slug_or_url(&arg_str(args, "slug"));
            let path = commands::with_version(
                &format!("/api/harness/sof/{slug}"),
                &arg_str(args, "version"),
            );
            let resp = client.get(&path)?;
            if resp.status >= 400 {
                return Err(resp.error_message());
            }
            commands::format_packages(&resp.json())
        }
        "ratsa_pull_package" => {
            let slug = api::slug_or_url(&arg_str(args, "slug"));
            let kind = {
                let k = arg_str(args, "kind");
                if k.is_empty() {
                    "device".to_string()
                } else {
                    k
                }
            };
            let out = arg_str(args, "out_dir");
            let version = arg_str(args, "version");
            commands::pull_report(client, &slug, &kind, &version, &out)
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
                client, &slug, score, passed, failed, &summary, &notes, "agent",
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
            let version = arg_str(args, "version");
            commands::package_cmd(client, &slug, &version, meta, &out)
        }
        "ratsa_get_sof_file" => {
            let slug = api::slug_or_url(&arg_str(args, "slug"));
            let out = arg_str(args, "out_dir");
            let version = arg_str(args, "version");
            commands::sof_file_cmd(client, &slug, &version, &out)
        }
        "ratsa_list_labs" => {
            if !client_has_creds(client) {
                return Ok(commands::cli_not_configured_hint());
            }
            commands::lab_list(client, true)
        }
        "ratsa_read_lab" => {
            if !client_has_creds(client) {
                return Ok(commands::cli_not_configured_hint());
            }
            let slug = api::slug_or_url(&arg_str(args, "slug"));
            commands::lab_get(client, &slug, true)
        }
        "ratsa_apply_lab" => {
            if !client_has_creds(client) {
                return Ok(commands::cli_not_configured_hint());
            }
            let profile = args
                .get("profile")
                .cloned()
                .ok_or_else(|| "缺少 profile（Lab 档案对象）".to_string())?;
            if !profile.is_object() {
                return Err("profile 必须是对象（字段同 POST /api/lab）".into());
            }
            let submit = args
                .get("submit")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let mut resp = client.post("/api/v1/labs", &profile)?;
            if resp.status >= 400 {
                return Err(resp.error_message());
            }
            if submit {
                resp = client.post(
                    "/api/v1/labs/apply",
                    &json!({"note": arg_str(args, "note")}),
                )?;
                if resp.status >= 400 {
                    return Err(resp.error_message());
                }
            }
            Ok(serde_json::to_string_pretty(&resp.json()).unwrap_or_default())
        }
        "ratsa_upload_lab_document" => {
            if !client_has_creds(client) {
                return Ok(commands::cli_not_configured_hint());
            }
            let slug = api::slug_or_url(&arg_str(args, "slug"));
            let path = arg_str(args, "path");
            let kind = {
                let k = arg_str(args, "kind");
                if k.is_empty() {
                    "other".to_string()
                } else {
                    k
                }
            };
            commands::lab_upload(client, &slug, &path, &kind, true)
        }
        "ratsa_list_lab_services" => {
            if !client_has_creds(client) {
                return Ok(commands::cli_not_configured_hint());
            }
            let slug = api::slug_or_url(&arg_str(args, "slug"));
            commands::lab_services(client, &slug, true)
        }
        "ratsa_list_lab_records" => {
            if !client_has_creds(client) {
                return Ok(commands::cli_not_configured_hint());
            }
            let slug = api::slug_or_url(&arg_str(args, "slug"));
            commands::lab_records(client, &slug, true)
        }
        "ratsa_delete_lab" => {
            if !client_has_creds(client) {
                return Ok(commands::cli_not_configured_hint());
            }
            let slug = api::slug_or_url(&arg_str(args, "slug"));
            commands::lab_delete(client, &slug)
        }
        "ratsa_list_lab_devices" => {
            if !client_has_creds(client) {
                return Ok(commands::cli_not_configured_hint());
            }
            let slug = api::slug_or_url(&arg_str(args, "slug"));
            commands::lab_devices(client, &slug, true)
        }
        "ratsa_add_lab_device" => {
            if !client_has_creds(client) {
                return Ok(commands::cli_not_configured_hint());
            }
            let slug = api::slug_or_url(&arg_str(args, "slug"));
            let sof = api::slug_or_url(&arg_str(args, "sof_slug"));
            commands::lab_device_add(
                client,
                &slug,
                &sof,
                &arg_str(args, "model"),
                &arg_str(args, "note"),
                &arg_str(args, "status"),
                true,
            )
        }
        "ratsa_update_lab_device" => {
            if !client_has_creds(client) {
                return Ok(commands::cli_not_configured_hint());
            }
            let slug = api::slug_or_url(&arg_str(args, "slug"));
            let id = args
                .get("id")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| "缺少 id（见 ratsa_list_lab_devices）".to_string())?;
            let status = arg_str(args, "status");
            let note = args.get("note").and_then(|v| v.as_str());
            commands::lab_device_status(client, &slug, id, &status, note)
        }
        "ratsa_list_lab_vendors" => {
            if !client_has_creds(client) {
                return Ok(commands::cli_not_configured_hint());
            }
            commands::lab_vendors(client, &arg_str(args, "q"), true)
        }
        "ratsa_list_lab_feedback" => {
            if !client_has_creds(client) {
                return Ok(commands::cli_not_configured_hint());
            }
            let slug = api::slug_or_url(&arg_str(args, "slug"));
            commands::lab_feedback(client, &slug, true)
        }
        "ratsa_add_lab_feedback" => {
            if !client_has_creds(client) {
                return Ok(commands::cli_not_configured_hint());
            }
            let slug = api::slug_or_url(&arg_str(args, "slug"));
            let rating = args
                .get("rating")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                .min(5) as u8;
            let amount = args.get("amount").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let service = args.get("service_id").and_then(|v| v.as_u64());
            let consent = args
                .get("consent")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            commands::lab_feedback_add(
                client,
                &slug,
                &arg_str(args, "customer_name"),
                &arg_str(args, "customer_company"),
                &arg_str(args, "customer_contact"),
                service,
                &arg_str(args, "order_ref"),
                amount,
                &arg_str(args, "currency"),
                rating,
                &arg_str(args, "category"),
                &arg_str(args, "title"),
                &arg_str(args, "detail"),
                &arg_str(args, "occurred_at"),
                consent,
                true,
            )
        }
        "ratsa_lab_feedback_summary" => {
            if !client_has_creds(client) {
                return Ok(commands::cli_not_configured_hint());
            }
            let slug = api::slug_or_url(&arg_str(args, "slug"));
            commands::lab_feedback_summary(client, &slug, true)
        }
        "ratsa_read_lab_spec" => {
            if !client_has_creds(client) {
                return Ok(commands::cli_not_configured_hint());
            }
            let slug = api::slug_or_url(&arg_str(args, "slug"));
            let resp = client.get(&format!("/api/v1/labs/{slug}/spec"))?;
            if resp.status >= 400 {
                return Err(resp.error_message());
            }
            Ok(serde_json::to_string_pretty(&resp.json()).unwrap_or_default())
        }
        other => Err(format!("未知工具：{other}")),
    }
}

fn client_has_creds(c: &Client) -> bool {
    !c.key_id.is_empty() && !c.secret.is_empty()
}
