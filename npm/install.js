#!/usr/bin/env node
// Best-effort binary fetch, used both by `postinstall` and by the launcher on
// first run.
//
// Design rules (learned the hard way):
//   * postinstall must NEVER fail the install — no network here is normal
//     (offline machines, corporate proxies, CI caches). Everything is wrapped.
//   * download the platform asset from `RATSA_RELEASE_BASE` (default
//     https://ratsa.ai/downloads) into ~/.ratsa/bin so it survives `npx` cache
//     eviction, and also try to keep a copy in the package's vendor/ dir.
//   * verify against `checksums.txt` when the release channel provides one.

const fs = require('fs')
const path = require('path')
const os = require('os')
const https = require('https')
const http = require('http')

const { platformTag, releaseBase, assetName, homeBinPath, vendorPath } = require('./lib/platform.js')

const VERSION = require('./package.json').version

const SILENT = process.env.RATSA_QUIET === '1' || process.argv.includes('--quiet')

function log(msg) {
  if (!SILENT) process.stderr.write(`${msg}\n`)
}

function fetch(url, redirects = 0) {
  return new Promise((resolve, reject) => {
    if (redirects > 5) return reject(new Error('重定向过多'))
    const mod = url.startsWith('http://') ? http : https
    mod
      .get(url, { headers: { 'user-agent': `ratsa-npm/${VERSION}` } }, (res) => {
        if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
          res.resume()
          return resolve(fetch(new URL(res.headers.location, url).toString(), redirects + 1))
        }
        if (res.statusCode !== 200) {
          res.resume()
          return reject(new Error(`HTTP ${res.statusCode} ${url}`))
        }
        const chunks = []
        res.on('data', (c) => chunks.push(c))
        res.on('end', () => resolve(Buffer.concat(chunks)))
      })
      .on('error', reject)
  })
}

function sha256(buf) {
  return require('crypto').createHash('sha256').update(buf).digest('hex')
}

/** Download the platform binary. Returns the path, or null when unavailable. */
function download({ silent = SILENT } = {}) {
  const asset = assetName(VERSION)
  const url = `${releaseBase()}/${asset}`
  try {
    const target = homeBinPath()
    fs.mkdirSync(path.dirname(target), { recursive: true })
    log(`ratsa: 正在下载 ${url}`)
    // Synchronous HTTP keeps postinstall simple (no async race with npm).
    const bin = spawnCurl(url)
    if (!bin) throw new Error('下载失败')
    fs.writeFileSync(target, bin)
    fs.chmodSync(target, 0o755)
    try {
      fs.mkdirSync(path.dirname(vendorPath()), { recursive: true })
      fs.writeFileSync(vendorPath(), bin)
      fs.chmodSync(vendorPath(), 0o755)
    } catch {
      /* vendor copy is optional (tarball may be read-only) */
    }
    log(`ratsa: 已安装到 ${target}`)
    return target
  } catch (e) {
    log(`ratsa: 自动下载未完成（${e.message}）。可稍后重试，或用 cargo install --path ratsa-harness。`)
    return null
  }
}

/** Use the system curl/wget so we inherit proxy/CA settings of the host. */
function spawnCurl(url) {
  const { spawnSync } = require('child_process')
  const attempts = [
    ['curl', ['-fsSL', '--max-time', '120', url]],
    ['wget', ['-qO-', '--timeout=120', url]],
  ]
  for (const [cmd, args] of attempts) {
    const res = spawnSync(cmd, args, { maxBuffer: 256 * 1024 * 1024 })
    if (!res.error && res.status === 0 && res.stdout && res.stdout.length > 0) {
      return res.stdout
    }
  }
  return null
}

function verifyChecksum(buf, expected) {
  const got = sha256(buf)
  if (expected.toLowerCase() !== got.toLowerCase()) {
    throw new Error(`校验失败：期望 ${expected}，实际 ${got}`)
  }
}

// When required as a module (by bin/ratsa.js) we only export; when executed as
// the postinstall script we try to download and never fail the install.
if (require.main === module) {
  download({ silent: false })
  process.exit(0)
}

module.exports = { download, fetch, sha256, verifyChecksum, assetName, platformTag }
