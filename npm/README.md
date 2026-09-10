# `ratsa`（npm 包装）

让 `npx ratsa …` 可用。这个包**不打包二进制**，只是一个启动器：找到（必要时下载）真正的
`ratsa` 可执行文件，然后把它 exec 出去。

```
npm/
  package.json      name=ratsa, bin={ratsa: bin/ratsa.js}, postinstall=install.js
  bin/ratsa.js      解析顺序：RATSA_BIN → vendor/ → ~/.ratsa/bin → 本地 target/{release,debug}
  lib/platform.js   平台标签、资产命名、解析逻辑（与 scripts/build-release.sh 同一约定）
  install.js        尽力下载（postinstall 永不失败），并留一份到 vendor/
```

## 为什么是「启动器」而不是各平台子包

- tarball 只有几 KB，`npm i` 不会因为下载 2.5 MB 二进制而变慢或失败；
- 一个 `npm publish` 覆盖 macOS / Linux / Windows（x64 + arm64）；
- 二进制放在 `~/.ratsa/bin/`，**不受 `npx` 缓存清理影响**；
- 离线/被墙环境里 postinstall 会安静跳过，`npx ratsa` 首次运行再试一次，仍失败则打印
  `cargo install --path ratsa-harness` 的替代方案。

## 发布流程（一次性）

```bash
# 1. 产出各平台产物 + 把当前平台二进制塞进 npm/vendor/
cd ratsa-harness && ./scripts/build-release.sh --all --vendor-npm

# 2. 把 dist/ 传到发布通道（二选一，asset 命名必须一致）
#    a) ratsa.ai 的 /downloads 目录（默认 RATSA_RELEASE_BASE）
#    b) GitHub Releases：
#       RATSA_RELEASE_BASE=https://github.com/<org>/<repo>/releases/download/v0.1.0
#    产物：ratsa-<version>-<os>-<arch>[.exe] + checksums.txt

# 3. 本地验证（不需要发布）
node npm/bin/ratsa.js --version
node npm/bin/ratsa.js agents

# 4. 发布
cd npm && npm publish --access public
```

`ratsa` / `@ratsa/cli` / `ratsa-harness` 在 npm 上目前都未被占用（2026-09-10 查询 registry
返回 404）。若日后 `ratsa` 被占，改用作用域包 `@ratsa/cli` 并把 `bin` 名称保持为 `ratsa`
（`npx @ratsa/cli …`）。

## 环境变量

| 变量 | 用途 |
| --- | --- |
| `RATSA_BIN` | 指定可执行文件，跳过解析（CI / 自编译） |
| `RATSA_RELEASE_BASE` | 发布通道前缀，默认 `https://ratsa.ai/downloads` |
| `RATSA_QUIET=1` | 静默 postinstall |
| `RATSA_HOME` | 用户目录（默认 `~/.ratsa`），二进制落在 `$RATSA_HOME/bin` |

## 与它无关但常被混淆

`npx ratsa mcp` 会**继承 stdin/stdout**（MCP 是 stdio 协议），启动器用
`spawnSync(bin, args, { stdio: 'inherit' })` 并透传退出码与信号 —— 改这个文件时别把 stdio
改成 pipe，否则 Agent 侧握手会挂住。
