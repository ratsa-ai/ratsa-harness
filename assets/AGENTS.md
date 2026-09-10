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
