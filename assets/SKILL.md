---
name: ratsa
description: 通过 RATSA.ai 平台检索 SOF（设备/零部件）、拉取 Harness Package、在本地跑评测并把结果回传。当用户提到 RATSA、SOF、Harness 包、设备 harness、eval repo、服务测试包、厂商设备接入、或"帮我把某个硬件接进来做评测"时使用。
---

# RATSA 接入 Skill

RATSA-Harness 已经装在这台机器上，命令是 `{{BIN}}`。

- 服务端：`{{BASE_URL}}`
- 当前 key：`{{KEY_NAME}}`
- 该 key 的权限：`{{SCOPES}}`
- 归属绑定：`{{BOUND}}`

## 核心事实（先读这段，避免走弯路）

1. **平台不跑你的代码，也不碰你的数据。** RATSA 只提供目录 + 制品 + 结构化结果记录。设备扫描、Eval 执行全部在**本地**完成。
2. **权限由 key 的权限表决定。** 如果某个命令返回 `missing_scope`（HTTP 403），说明当前 key 没有那项权限 —— 不要反复重试，直接告诉用户缺哪个 scope，以及可以用 `{{BIN}} scopes` 查看完整权限表。
3. **可见性由发布者决定。** 返回 401 `login_required` 表示该制品是"登录可见"，403/404 表示"私有，仅发布者本人可见"。这不是网络问题。
4. **归属 = key 的 bound slug。** 如果 key 绑定了某个 handle，那么你做的发布/提交在平台上都算在该账号名下。
5. **不要编造 slug。** 所有 slug 必须来自 `search` / `packages` 的真实返回。

## 标准工作流

### 1. 确认身份与权限（开头必做一次）

```bash
{{BIN}} whoami          # 账号、key、权限、归属绑定
{{BIN}} scopes          # 完整权限表 + 每项含义
```

### 2. 找到目标设备

```bash
{{BIN}} search 舵机                 # 关键字
{{BIN}} search --category 核心零部件 # 按类目
{{BIN}} search --json | jq ...      # 需要字段时用 JSON
```

拿到 slug 后读详情：

```bash
{{BIN}} sof <slug> --json
```

### 3. 看这台设备能拉什么包

```bash
{{BIN}} manifest <slug>       # 就绪度 + 问题清单 + 统一 packages 清单
{{BIN}} packages <slug>       # 只要包清单
```

`packages[]` 里每个条目都带 `package_kind`，一共三种：

| package_kind | 含义 | 拉取命令 |
|---|---|---|
| `device-harness` | 平台按 SOF 派生出的设备 Harness 包（含 `sof.json`、`ratsa.harness.json`、`harness/harness.py`、`manifest.json`），厂商在其上填实现 | `pull <slug> --kind device` |
| `eval-repo` | 厂商/第三方发布的评测仓库（拉下来本地跑） | `pull <eval-slug> --kind eval-repo` |
| `service-harness` | 评测服务商发布的测试包 | `pull <service-slug> --kind service` |

### 4. 拉包并本地运行

```bash
{{BIN}} pull <slug> --kind device
# → 落到 ~/.ratsa/packages/<slug>/<kind>/
unzip -o ~/.ratsa/packages/<slug>/device/*.zip -d ~/.ratsa/packages/<slug>/device/
```

包内 `manifest.json` 列出每个文件的 sha256；`X-Ratsa-Checksum` 由 CLI 自动校验，校验失败会直接报错 —— **校验失败的包不要执行**。

### 5. 在本地跑，把结果回传（这才是有价值的一步）

```bash
{{BIN}} report <slug> --score 88 --passed 22 --failed 2 --summary "PWM 响应正常" --notes "…"
```

回传后会出现在平台的 Harness Eval（就绪度看板，来源标为 `agent`），与厂商自述的静态就绪度**分开展示**。

### 6. 有问题就反馈给厂商

```bash
{{BIN}} feedback <slug> --title "上电后第 3 秒抖动" --detail "…" \
  --category bug --severity high
```

## 需要新权限时

不要自己扩权。告诉用户：

```bash
{{BIN}} keys create --name <用途> --scopes <需要的 scope 列表> --bound-slug <handle> --use
```

可用 scope 见 `{{BIN}} scopes`。默认集合是最小可用的读 + 拉取权限。

## 常见错误对照

| 现象 | 含义 | 处理 |
|---|---|---|
| 403 `missing_scope` | key 缺这项权限 | 告知用户缺哪个 scope，不要重试 |
| 401 `login_required` | 制品是登录可见 | 让用户配置带该权限的 key |
| 404 / 403 且提示私有 | 私有制品，仅发布者可见 | 告知用户，不要猜 slug |
| `没有匹配的 SOF` | 关键字/类目不对 | 换关键字或去掉类目过滤 |
| `校验失败` | 包内容与服务器 sha256 不符 | 停止，报告用户 |

## 用 MCP 工具（如果宿主已注册）

若宿主把 RATSA 注册成了 MCP server（`{{BIN}} mcp`），直接用这些工具，不必拼命令行：

`ratsa_whoami` · `ratsa_scopes` · `ratsa_search_sofs` · `ratsa_read_sof` · `ratsa_device_harness` · `ratsa_list_packages` · `ratsa_pull_package` · `ratsa_report_run` · `ratsa_submit_feedback`
