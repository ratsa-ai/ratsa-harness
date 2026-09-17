// Shared platform helper for the `ratsa` npm wrapper.
//
// The npm package is a **thin launcher**, not a bundled binary per platform:
// it resolves (or downloads) the real `ratsa` executable and then execs it. That
// keeps the tarball tiny and makes `npx @ratsa/cli …` work on macOS / Linux / Windows
// with the same publish.

const os = require('os')
const path = require('path')
const fs = require('fs')

const PKG_ROOT = path.join(__dirname, '..')

function platformTag() {
  const osName = { darwin: 'darwin', linux: 'linux', win32: 'windows' }[process.platform]
  const arch = { x64: 'x64', arm64: 'arm64' }[process.arch]
  if (!osName || !arch) {
    throw new Error(`暂不支持的平台：${process.platform}/${process.arch}`)
  }
  return `${osName}-${arch}`
}

function binaryName() {
  return process.platform === 'win32' ? 'ratsa.exe' : 'ratsa'
}

function ext() {
  return process.platform === 'win32' ? '.exe' : ''
}

/** Vendored binary shipped inside the published tarball (optional path). */
function vendorPath() {
  return path.join(PKG_ROOT, 'vendor', `ratsa-${platformTag()}${ext()}`)
}

/** User-level install dir written by install.js (survives `npx` cache churn). */
function homeBinPath() {
  const home = process.env.RATSA_HOME
    ? path.join(process.env.RATSA_HOME, 'bin')
    : path.join(os.homedir(), '.ratsa', 'bin')
  return path.join(home, binaryName())
}

function isExecutable(p) {
  try {
    const st = fs.statSync(p)
    if (!st.isFile()) return false
    if (process.platform !== 'win32') {
      fs.accessSync(p, fs.constants.X_OK)
    }
    return true
  } catch {
    return false
  }
}

/** Release base — a directory served by ratsa.ai, or a GitHub release prefix. */
function releaseBase() {
  return (process.env.RATSA_RELEASE_BASE || 'https://ratsa.ai/downloads').replace(/\/+$/, '')
}

/**
 * 要下载哪个发布版本，默认 `latest`。
 *
 * 与 install.sh 的 `VERSION="${RATSA_VERSION:-latest}"`（server 侧 cli_release.go）
 * 是同一套约定 —— 两条通道对「不指定版本时拿什么」必须给同一个答案，否则 npm 用户
 * 和 shell 用户会拿到不同版本。
 *
 * 刻意不取 package.json 的 version：那个值跟 release 资产之间没有任何强校验，
 * 一旦 npm 发了新版本而 release 没跟上，所有安装都会 404。这里改为跟随 latest，
 * 想要钉住版本就显式设 RATSA_VERSION（允许带 v 前缀）。
 */
function releaseVersion() {
  const v = (process.env.RATSA_VERSION || '').trim().replace(/^v/, '')
  return v || 'latest'
}

/**
 * Asset name convention shared with `scripts/build-release.sh`.
 *
 * `version` 既可以是具体版本号，也可以是字面量 `'latest'` —— 后者会拼出
 * `ratsa-latest-<os>-<arch>[.exe]`，即发版流程额外产出的版本无关别名。
 * 这里刻意不写分支：别名由模板自然得出，加分支反而会让两条通道的命名漂移。
 */
function assetName(version) {
  return `ratsa-${version}-${platformTag()}${ext()}`
}

/**
 * Resolution order: RATSA_BIN → ~/.ratsa/bin → vendored → local cargo build.
 *
 * `~/.ratsa/bin` deliberately outranks the vendored copy inside the package.
 * The two are written together by `download()`, so on a fresh install they are
 * the same bytes and the order does not matter — but they diverge the moment
 * anything updates one of them, and then the order decides everything:
 *
 *   * `~/.ratsa/bin` is the copy that survives removing/reinstalling the package,
 *     and it is what `install.sh` and the Rust side's `current_bin()` write.
 *   * `vendor/` is frozen at install time. Because the package version is
 *     independent of the CLI version, npm reports "up to date" forever and never
 *     re-runs our postinstall — so a vendored copy can never refresh itself.
 *
 * Ranking the frozen copy first is what made those two diverge silently: the MCP
 * entry we write says `~/.ratsa/bin/ratsa` while the process actually running was
 * vendor's. Whoever went to debug that would be looking at the wrong file.
 */
function resolveBinary() {
  const candidates = []
  if (process.env.RATSA_BIN) candidates.push(process.env.RATSA_BIN)
  candidates.push(homeBinPath(), vendorPath())
  // Dev convenience: a build inside the repo (never used by published tarballs).
  for (const profile of ['release', 'debug']) {
    candidates.push(
      path.join(PKG_ROOT, '..', 'target', profile, binaryName()),
      path.join(PKG_ROOT, 'target', profile, binaryName()),
    )
  }
  return candidates.find(isExecutable) || null
}

module.exports = {
  PKG_ROOT,
  platformTag,
  binaryName,
  ext,
  vendorPath,
  homeBinPath,
  isExecutable,
  releaseBase,
  releaseVersion,
  assetName,
  resolveBinary,
}
