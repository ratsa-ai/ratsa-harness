# AGENTS.md — RATSA.ai 接入

本仓库的 Agent 可用 **RATSA-Harness**（`{{BIN}}`）访问 RATSA.ai 平台：检索 SOF（设备 / 零部件）、拉取 Harness Package、本地运行评测并回传结果。

## 环境

| 项 | 值 |
|---|---|
| 可执行文件 | `{{BIN}}` |
| base_url | `{{BASE_URL}}` |
| key | `{{KEY_NAME}}` |
| 权限（scope） | `{{SCOPES}}` |
| 归属绑定 | `{{BOUND}}` |

凭证来自 `~/.ratsa/config.json`，也可用环境变量注入：`RATSA_BASE_URL` / `RATSA_KEY_ID` / `RATSA_KEY_SECRET`。

## 硬性约束

1. 平台**不执行第三方代码、不中转业务数据**；扫描与评测在本地跑。
2. key 的**权限表**决定能做什么。403 `missing_scope` 时不要重试，告知用户缺哪项（`{{BIN}} scopes`）。
3. 401 `login_required` = 登录可见；403 / 404 = 私有（仅发布者可见）。都不是网络错误。
4. 不要编造 slug；只使用 `search` / `packages` 的真实返回。
5. `pull` 会校验服务器给出的 sha256；不一致时**不要执行**该包。
6. 不要提权；需要新权限时让用户执行：
   `{{BIN}} keys create --name <用途> --scopes <列表> --bound-slug <handle> --use`

## 命令

```bash
{{BIN}} whoami
{{BIN}} scopes
{{BIN}} search <关键字> [--category <类目>] [--json]
{{BIN}} sof <slug> [--json]
{{BIN}} manifest <slug>        # 就绪度 + 问题清单 + 统一 packages[]
{{BIN}} packages <slug> [--json]
{{BIN}} pull <slug> --kind device|eval-repo|service [--out <目录>]
{{BIN}} report <slug> --score 88 --passed 22 --failed 2 --summary "…"
{{BIN}} feedback <slug> --title "…" --detail "…" --category bug --severity high
{{BIN}} mcp                    # 以 MCP stdio server 运行
```

## 闭环顺序

1. `whoami` → 确认身份与权限
2. `search` → 找到设备 slug
3. `manifest` → 看就绪度与可拉取的包
4. `pull` → 落盘到 `~/.ratsa/packages/<slug>/<kind>/`
5. 本地运行 → `report` 回传分数（来源标记为 `agent`）
6. 有问题 → `feedback` 反馈厂商

静态就绪度（平台按 SOF 声明派生）与实测结果（你回传的）在平台上是**分开**展示的两条线。

## RATSA Lab（Cloud Lab，完全私有）

任何登录账号都可**自助创建** Lab 并自选领域（bio / chem / env / materials），审核由平台运营把关；没有公开列表，只能访问本账号（或 key 绑定账号）名下的 Lab。需要的权限：`lab:read` / `lab:write`。

```bash
{{BIN}} lab list                                  # 我的 Lab（无公开列表）
{{BIN}} lab get <slug>                            # 档案 + 服务 + 过往服务 + 资料
{{BIN}} lab upload <slug> <file> --kind license|certificate|sop|equipment|other
{{BIN}} lab apply <profile.json> --submit --note "…"
{{BIN}} lab status <slug>
{{BIN}} lab delete <slug>
{{BIN}} lab spec <slug>                           # ratsa.lab.json（Agent / 云接入契约，sha256 校验）
{{BIN}} lab services <slug>
{{BIN}} lab records <slug>
# 设备：从 RATSA AI 目录绑定厂商设备，并维护运行状态
{{BIN}} lab vendors [关键字]                      # 平台上的设备厂商 / 设备
{{BIN}} lab devices <slug>
{{BIN}} lab device-add <slug> <sof-slug> [--model …] [--note …]
{{BIN}} lab device-status <slug> <id> active|idle|maintenance|offline [--note …]
{{BIN}} lab device-remove <slug> <id>
# 客户服务交易反馈（用实验室账号 key 登记 → 平台核验 → 资质 / 营销）
{{BIN}} lab feedback <slug>
{{BIN}} lab feedback-add <slug> --company "客户公司" --rating 5 [--order-ref …] [--consent]
{{BIN}} lab feedback-summary <slug>
{{BIN}} lab feedback-delete <slug> <id>
```

- Lab 的 404 = 不是本账号的 Lab（完全私有），**不要猜 slug、不要重试**。
- 提交审核前必须先 `lab upload` 至少一份资料。
- 设备只能从 RATSA AI 目录绑定（公开设备或本账号自己的设备）；状态由 Lab 维护，SOF 资料仍归厂商。
