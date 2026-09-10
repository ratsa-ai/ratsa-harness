// Shared platform helper for the `ratsa` npm wrapper.
//
// The npm package is a **thin launcher**, not a bundled binary per platform:
// it resolves (or downloads) the real `ratsa` executable and then execs it. That
// keeps the tarball tiny and makes `npx ratsa …` work on macOS / Linux / Windows
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

/** Asset name convention shared with `scripts/build-release.sh`. */
function assetName(version) {
  return `ratsa-${version}-${platformTag()}${ext()}`
}

/** Resolution order: RATSA_BIN → vendored → ~/.ratsa/bin → local cargo build. */
function resolveBinary() {
  const candidates = []
  if (process.env.RATSA_BIN) candidates.push(process.env.RATSA_BIN)
  candidates.push(vendorPath(), homeBinPath())
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
  assetName,
  resolveBinary,
}
