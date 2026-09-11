# RATSA-Harness（命令行 `ratsa`）

[English](./README.md) · **中文**

把 RATSA.ai 接进**你已经在用的 Agent**：装一个 skill / MCP server，用一把**最小权限**的 key，
跑通「检索 → 生成/拉包 → 本地跑 → 回传」的闭环。

```bash
# 1. 装（六种 Agent 目标，见下）
npx ratsa install --agent all           # 当前项目
npx ratsa install --agent all --global  # 你的用户目录（每个工作区都能用）
npx ratsa agents                        # 支持哪些 Agent / 各写哪些文件

# 2. 凭证（推荐：自选权限 + 绑定归属）
npx ratsa login --email you@example.com
npx ratsa scopes
npx ratsa keys create --name laptop-agent \
  --scopes sof:read,harness:read,package:pull --bound-slug your-handle --use

# 3. 闭环
npx ratsa whoami
npx ratsa search 舵机
npx ratsa package <slug> --meta   # 平台官方生成设备 Harness 包（先看描述）
npx ratsa package <slug>          # 下载（自动校验 sha256）
npx ratsa sof-file <slug>         # 拉取 SOF 文件（可回灌）
npx ratsa report <slug> --score 88 --passed 22 --failed 2
```

## 为什么是 CLI + 插件，而不是又一个 SDK

Agent 时代真正缺的不是 API，而是**让 Agent 知道该怎么用**。所以本工程有两半：

1. **一个二进制**（Rust，无运行时依赖）：做网络调用、权限、校验、落盘。
2. **一份可安装的说明**（skill / rules / AGENTS.md）：告诉 Agent 权限边界、包的种类、
   失败怎么看 —— 包括「不要重试 `missing_scope`」「校验失败就停」这类**行为约束**。

MCP 工具面同时暴露，Agent 可以完全不用记命令。

## 边界（读到这里的 Agent 也该知道）

- 平台**不执行**你的代码，**不中转**业务数据；设备扫描与 Eval 全部在本地跑。
- CLI 只做三件事：拉制品、推结构化结果、管理自己的凭证。**没有 daemon，没有埋点。**
- 拉取会校验服务端给出的 sha256（`X-Ratsa-Checksum`），不一致直接报错并拒绝落盘执行。

## 安装目标

| `--agent` | 落盘 | 作用 |
|---|---|---|
| `claude` | `.claude/skills/ratsa/SKILL.md`（`--global`：`~/.claude/skills/ratsa/SKILL.md`） | Claude Code skill |
| `mcp` | `.mcp.json`（合并 `mcpServers.ratsa`） | Claude Code / 兼容 MCP 客户端的 stdio server |
| `cursor` | `.cursor/rules/ratsa.mdc` + `.cursor/mcp.json` | Cursor 规则 + MCP |
| `copilot` | `.github/copilot-instructions.md` + `.github/skills/ratsa/SKILL.md` + `.vscode/mcp.json` | GitHub Copilot（`--global` 时另写 VS Code 用户 prompts 目录） |
| `agents` | `AGENTS.md` | 通用约定（Codex / 其它读 AGENTS.md 的 Agent） |
| `codex` | `<dir>/.codex/config.toml`（追加 `[mcp_servers.ratsa]`）+ `<dir>/.codex/RATSA.md`；默认 `<dir>` = `$HOME`（Codex 只读用户级配置） | OpenAI Codex CLI |

- 默认 **幂等**：已存在同名条目就跳过；`--force` 覆盖（改已有文件前先留 `.bak`）。
- `--dry-run` 只打印将要写入的内容与合并结果，不落盘。
- `--dir <path>` 指定根目录（可用于 CI / 容器镜像构建）。

## 命令

| 命令 | 说明 |
|---|---|
| `install` | 安装 / 更新各 Agent 的 skill 与 MCP 条目 |
| `agents` | 列出支持的 Agent、形态、写入文件、是否注册 MCP（`--json`） |
| `login` / `logout` | 会话（仅用于创建与管理 key）；`RATSA_PASSWORD` 可免交互 |
| `whoami` | 身份 + key 权限 + 归属绑定（`--json`） |
| `scopes` | 权限表（从 `/api/meta` 取，含每项含义与默认集合） |
| `keys list` / `keys create` / `keys revoke` | 权限自选 + 可选 `--bound-slug`；`--use` 立即应用 |
| `config set/show/clear` | 本地凭证（`~/.ratsa/config.json`，0600） |
| `search` / `sof` | 跨厂商检索与详情（`--json`） |
| `manifest` / `packages` | 就绪度 + 问题清单 + 统一 `packages[]` |
| `package` | **平台官方生成设备 Harness 包**并下载；`--meta` 只看描述（包内文件/就绪度/大小） |
| `sof-file` | 拉取 SOF 文件（`kind=ratsa.sof.json`，可直接回灌 `POST /api/sof`） |
| `pull` | 拉取任意 kind：`--kind device\|eval-repo\|service`，落盘 `~/.ratsa/packages/<slug>/<kind>/` |
| `kb` | 拉取 SOF 知识库：无参看索引，`--sof <slug>` 看设备分层，`--doc <key\|slug>` 打印/落盘 Markdown 原文（校验 checksum） |
| `report` | 回传实测（`source=agent`），写入被测设备的 Harness Eval |
| `feedback` | 提交反馈给厂商 |
| `mcp` | 以 MCP stdio server 运行（14 个工具） |

环境变量（适合 CI / 容器，无需配置文件）：
`RATSA_BASE_URL`、`RATSA_KEY_ID`、`RATSA_KEY_SECRET`、`RATSA_CONFIG`、`RATSA_HOME`、`RATSA_PASSWORD`。

## 权限表（key / secret 的 scope）

key 能做什么**只**由权限表决定；默认集合是「读 + 拉取」，也就是让 Agent 能干活的最小集合。

| scope | 组 | 含义 |
|---|---|---|
| `sof:read` | 读取 | 读公开 SOF、目录、`/api/meta` |
| `sof:read:private` | 读取 | 额外可见 key 所有者（或绑定 slug）的私有 SOF |
| `harness:read` | 读取 | Harness 清单、就绪度、包清单 |
| `eval:read` | 读取 | Eval 列表、Eval Repo 元数据与 manifest |
| `package:pull` | 拉取 | 下载设备 Harness 包 / Eval Repo / 服务测试包 |
| `feedback:submit` | 回传 | 以 Agent / 设备身份提交反馈 |
| `eval:report` | 回传 | 上报实测（`kind=harness`、`source=sdk\|agent`） |
| `eval:publish` | 发布 | 创建 / 更新 Eval Repo |
| `sof:write` | 发布 | 创建 / 更新 / 删除属于该身份的 SOF |
| `order:manage` | 发布 | 作为服务商受理 / 开工 / 交付评测委托 |
| `key:manage` | 管理 | 列出 / 轮换 / 删除该账号的 key |
| `admin` | 管理 | 仅管理员可授予；包含所有权限 |

- **默认**（不指定时）：`sof:read,harness:read,eval:read,package:pull`。
- 可见性仍然叠加在上层：权限表决定「这一类操作能不能做」，发布者设定的**可见性档位**
  决定「这一条数据能不能看」（登录可见 → 401 `login_required`；私有 → 403/404）。
- 缺权限返回 **403 `missing_scope`**，响应体带 `scope` 与 `scopes`，便于 Agent 直接
  告诉用户该补哪一项。

## 归属绑定（`bound_slug`）

给 key 指定一个账号 handle，该 key 之后**以该账号身份行动**：它读取的私有资产、
它的发布与提交，都归属到那个账号。

- 用途：把「谁在用这把钥匙」和「这把钥匙代表谁」分开 —— 例如外包商 / 集成商替客户
  做事、或一台设备用绑定 key 上报实测。
- 限制：**普通用户只能绑定自己的 handle**（防止提权）。绑定他人 handle 目前需要管理员；
  若需要「厂商授权第三方持有自己身份的 key」，需要一条**授权邀请**流程（见下）。
- 平台侧解析：`/api/v1/me` 返回 `acting_as`；key 列表返回 `bound_user`。

## MCP 工具（14 个）

`ratsa_whoami` · `ratsa_scopes` · `ratsa_search_sofs` · `ratsa_read_sof` ·
`ratsa_device_harness` · `ratsa_list_packages` · `ratsa_pull_package` ·
`ratsa_list_agents` · `ratsa_generate_package` · `ratsa_get_sof_file` ·
`ratsa_report_run` · `ratsa_submit_feedback` ·
`ratsa_list_kb` · `ratsa_read_kb`

协议为 MCP stdio（换行分隔的 JSON-RPC 2.0），无额外依赖，便于审计。

## 构建与分发

```bash
cd ratsa-harness
cargo build --release            # 产出 target/release/ratsa
cargo build --offline            # 依赖已缓存时可离线构建
./scripts/build-release.sh --all --vendor-npm   # 各平台产物 + npm/vendor/ 内置当前平台二进制
```

`npx ratsa` 依靠 `npm/` 下的**启动器包**（不打包二进制，运行时解析或下载，见 `npm/README.zh-CN.md`）：

```bash
node npm/bin/ratsa.js --version   # 本地验证
cd npm && npm publish --access public
```

发布通道由 `RATSA_RELEASE_BASE` 指定（默认 `https://ratsa.ai/downloads`，也可指向 GitHub
Releases 前缀）；资产命名 `ratsa-<version>-<os>-<arch>[.exe]` + `checksums.txt`。

## 已知限制 / 待办

- 「绑定他人 handle」需要授权邀请机制（当前仅管理员可为，见 `prd/ratsa-harness-issues.md` A-10）。
- Windows 未单独验证（代码只用 std + ureq，理论上可用）。
- `--global` 的 VS Code Copilot 目标写入用户 prompts 目录，仅覆盖 instructions（不写用户级 MCP）。
- 新 Agent 目标（Windsurf / Zed / Cline / Continue / Aider / Gemini CLI 等）未单列：加一个
  目标 = `src/install.rs` 的 `target_table()` 加一条 + `payloads()` 给一行落盘路径，并同步
  `web/src/components/HarnessCliIntro.jsx` 的 AGENTS 表与 `prd/ratsa-harness-cli.md` §2。
