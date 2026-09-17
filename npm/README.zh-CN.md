# `ratsa`（npm 包装）

[English](./README.md) · **中文**

让 `npx @ratsa/cli …` 可用。这个包**不打包二进制**，只是一个启动器：找到（必要时下载）真正的
`ratsa` 可执行文件，然后把它 exec 出去。

```
npm/
  package.json      name=@ratsa/cli, bin={ratsa: bin/ratsa.js}, postinstall=install.js
  bin/ratsa.js      解析顺序：RATSA_BIN → ~/.ratsa/bin → vendor/ → 本地 target/{release,debug}
  lib/platform.js   平台标签、资产命名、解析逻辑（与 scripts/build-release.sh 同一约定）
  install.js        尽力下载（postinstall 永不失败），并留一份到 vendor/
  README.md         英文说明（npm 页面显示）
  README.zh-CN.md   中文说明（与上者顶部互为语言切换）
```

## 为什么是「启动器」而不是各平台子包

- tarball 只有几 KB，`npm i` 不会因为下载 2.5 MB 二进制而变慢或失败；
- 一个 `npm publish` 覆盖 macOS / Linux / Windows（x64 + arm64）；
- 二进制放在 `~/.ratsa/bin/`，**不受 `npx` 缓存清理影响**；
- 离线/被墙环境里 postinstall 会安静跳过，`npx @ratsa/cli` 首次运行再试一次，仍失败则打印
  `cargo install --path ratsa-harness` 的替代方案。

## 发布流程

**两种发布彼此独立。** 改了什么决定动哪个版本号：

| 改的是 | 要动 | 打 tag `vX.Y.Z` | `npm publish` |
|---|---|---|---|
| CLI 二进制（`src/`） | `Cargo.toml` **和** `Cargo.lock` | 要 | 不要 |
| 本包（`install.js` / `lib/` / `bin/`） | `npm/package.json` | 不要 | 要 |

之所以能独立，是因为 `install.js` 拼下载地址用的是 `latest`（可用 `RATSA_VERSION` 覆盖），
而不是本包自己的版本号。于是只改二进制时不必白白发一次 npm，只改 npm 时也不必重新构建。
早先的实现是从 `package.json` 取版本号，逼得两者必须同进同退 —— 别再退回去。

**要记住的代价：已安装的用户不会自己拿到只改二进制的发布。** 本包版本号没变时
`postinstall` 不会重跑，npm 会一直报 "up to date"，而 `~/.ratsa/bin/ratsa` 就冻在安装
那一刻。出路是 `ratsa upgrade` —— 它直接走发布通道，完全不经过 npm。（反过来，动本包
版本号会顺带刷新二进制，因为 `postinstall` 会重跑 —— 这是"想推进一轮更新时该动它"的
理由，但从来不是把两个号绑在一起的理由。）

### 二进制发布

```bash
# 1. 改 Cargo.toml 的 version，并同步 Cargo.lock。Cargo.lock 不能漏：
#    CI 用 `cargo build --locked`，lock 过期会直接构建失败。
# 2. 提交，然后打 tag —— 推 tag 才是触发发布的动作：
git tag vX.Y.Z && git push origin vX.Y.Z
```

随后 CI（`.github/workflows/release.yml`）会构建全部 6 个平台目标，把
`ratsa-<version>-<os>-<arch>[.exe]` + `checksums.txt` 发到 GitHub Release，
并把同一批外加 `ratsa-latest-<os>-<arch>[.exe]` 别名镜像到华为云 OBS
（对外即 `https://ratsa.ai/downloads`）。**那批 `ratsa-latest-*` 别名正是两种发布
得以独立的原因** —— 所有安装实际取的就是这个名字。

离线兜底 / CI 故障：`./scripts/build-release.sh --all --vendor-npm` 会在 `dist/`
下产出同样的资产名，传到 `RATSA_RELEASE_BASE` 指向的地方即可。

### npm 包发布

```bash
node npm/bin/ratsa.js --version   # 本地自检
node npm/bin/ratsa.js agents
cd npm && npm publish             # 目标 registry 与 access 由 publishConfig 固定
```

以 **`@ratsa/cli`** 发布。无作用域名 `ratsa` 被 npm 的防抢注检查拒绝（"too similar to
existing packages ramda,nats"）。该检查**只在发布时**执行 —— 所以 registry 返回 404 **并不**
代表这个名字可以发布：`ratsa` 查出来是 404，却依然发不上去。`bin` 名保持 `ratsa`，
装完之后命令仍是 `ratsa`，只有 `npx` 那一段带作用域。

## 环境变量

| 变量 | 用途 |
| --- | --- |
| `RATSA_BIN` | 指定可执行文件，跳过解析（CI / 自编译） |
| `RATSA_RELEASE_BASE` | 发布通道前缀，默认 `https://ratsa.ai/downloads` |
| `RATSA_QUIET=1` | 静默 postinstall |
| `RATSA_HOME` | 用户目录（默认 `~/.ratsa`），二进制落在 `$RATSA_HOME/bin` |

## 与它无关但常被混淆

`npx @ratsa/cli mcp` 会**继承 stdin/stdout**（MCP 是 stdio 协议），启动器用
`spawnSync(bin, args, { stdio: 'inherit' })` 并透传退出码与信号 —— 改这个文件时别把 stdio
改成 pipe，否则 Agent 侧握手会挂住。
