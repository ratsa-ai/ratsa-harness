# RATSA.ai 接入（GitHub Copilot）

本仓库的 Agent 可以通过 **RATSA-Harness** 访问 RATSA.ai 平台：检索 SOF（设备 / 零部件）、拉取 Harness Package、在本地跑评测并把结果回传。

- 命令：`{{BIN}}`
- base_url：`{{BASE_URL}}`
- 当前 key：`{{KEY_NAME}}`
- 该 key 权限：`{{SCOPES}}`
- 归属绑定：`{{BOUND}}`

## 使用规则

1. **平台不执行第三方代码、不中转业务数据。** RATSA 只提供目录、制品和结果记录；设备扫描与评测在本地完成。
2. **权限由 key 的权限表决定。** 碰到 403 `missing_scope` 时不要重试或换命令绕开 —— 直接告诉用户缺哪一项权限（`{{BIN}} scopes` 可列出全部权限表）。
3. **可见性是发布者的选择。** 401 `login_required` 表示"登录可见"，403/404 表示"私有，仅发布者可见"。
4. **不要编造 slug / 包名**，只能引用工具返回的真实值。
5. **校验失败即停**：`pull` 会自动核对服务器提供的 sha256，不一致时不要执行该包。
6. 需要新权限时，让用户自己创建（不要尝试提权）：

```bash
{{BIN}} keys create --name <用途> --scopes <scope 列表> --bound-slug <handle> --use
```

## 常用命令

```bash
{{BIN}} whoami
{{BIN}} scopes
{{BIN}} search <关键字> [--category <类目>] [--json]
{{BIN}} sof <slug> [--json]
{{BIN}} manifest <slug>
{{BIN}} packages <slug> [--json]
{{BIN}} pull <slug> --kind device|eval-repo|service
{{BIN}} report <slug> --score <0-100> --passed <n> --failed <n>
{{BIN}} feedback <slug> --title "…" --detail "…"
```

## MCP 工具

若 VS Code 已注册 RATSA 的 MCP server（`{{BIN}} mcp`，见 `.vscode/mcp.json`），可直接调用：
`ratsa_whoami`、`ratsa_scopes`、`ratsa_search_sofs`、`ratsa_read_sof`、`ratsa_list_sof_versions`、`ratsa_device_harness`、`ratsa_rth_spec`、`ratsa_list_rth_projects`、`ratsa_read_rth_project`、`ratsa_rth_channel`、`ratsa_verify_rth_package`、`ratsa_list_packages`、`ratsa_pull_package`、`ratsa_report_run`、`ratsa_submit_feedback`。

读取 / 拉取类工具可传 `version` 指定 SOF 历史版本。

## RTH（Ratsa Runtime Harness）

平台侧：`ratsa rth create/package/sign/validate/verify/channel`；运行在 RTH 本地：`ratsa rth init/status/plugin/mem/perception/ontology/build-graph`（读写 `ratsa.rth.json` + `.rth/`）。

## 三种 Harness Package

| package_kind | 说明 | 拉取 |
|---|---|---|
| `device-harness` | 平台按 SOF 派生的设备 Harness 包（`sof.json`、`ratsa.harness.json`、`harness/harness.py`、`manifest.json`） | `pull <slug> --kind device` |
| `eval-repo` | 可本地运行的评测仓库 | `pull <slug> --kind eval-repo` |
| `service-harness` | 评测服务商提供的测试包 | `pull <slug> --kind service` |

## RATSA Lab（Cloud Lab，完全私有）

任何登录账号都可自助创建 Lab 并自选领域，审核由平台运营把关；没有公开列表。权限：`lab:read` / `lab:write`。

```bash
{{BIN}} lab list
{{BIN}} lab get <slug>
{{BIN}} lab upload <slug> <file> --kind license|certificate|sop|equipment|other
{{BIN}} lab apply <profile.json> --submit --note "…"
{{BIN}} lab spec <slug>          # ratsa.lab.json（Agent / 云接入契约）
{{BIN}} lab services <slug>
{{BIN}} lab records <slug>
{{BIN}} lab status <slug>
{{BIN}} lab delete <slug>
# 设备（厂商设备来自 RATSA AI 目录）
{{BIN}} lab vendors [关键字]
{{BIN}} lab devices <slug>
{{BIN}} lab device-add <slug> <sof-slug>
{{BIN}} lab device-status <slug> <id> active|idle|maintenance|offline
{{BIN}} lab device-remove <slug> <id>
{{BIN}} lab feedback <slug>
{{BIN}} lab feedback-add <slug> --company "客户公司" --rating 5 [--consent]
{{BIN}} lab feedback-summary <slug>
```

404 = 不是本账号的 Lab（完全私有）；审核中不可删除。
