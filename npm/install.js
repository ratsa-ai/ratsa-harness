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
//   * verify against `checksums.txt` when the release channel provides one — the
//     check happens BEFORE anything is written to disk, so a tampered or
//     half-mirrored payload can never end up in ~/.ratsa/bin.
//   * which version we fetch comes from releaseVersion() (default `latest`,
//     override with RATSA_VERSION), NOT from this package's own version — those
//     two are not validated against each other anywhere, so treating the package
//     version as the release version meant one un-synced bump broke every install.

const fs = require('fs')
const path = require('path')
const os = require('os')
const https = require('https')
const http = require('http')

const {
  platformTag,
  releaseBase,
  releaseVersion,
  assetName,
  homeBinPath,
  vendorPath,
} = require('./lib/platform.js')

// 仅用于 User-Agent：标示请求来自哪个 npm 包版本。
const VERSION = require('./package.json').version

const SILENT = process.env.RATSA_QUIET === '1' || process.argv.includes('--quiet')

function log(msg) {
  if (!SILENT) process.stderr.write(`${msg}\n`)
}

/**
 * 安全相关的失败必须可见，即使 RATSA_QUIET=1。
 * 校验不通过而静默不装，用户只会看到「找不到可执行文件」，无从判断是被篡改还是没网。
 */
function warn(msg) {
  process.stderr.write(`${msg}\n`)
}

// download() 返回 null 时，这里留着原因，供 bin/ratsa.js 打出准确的提示。
let lastFailure = null
function lastError() {
  return lastFailure
}

// 未被调用。保留仅为将来可能需要「拿到 HTTP 状态码」的场景 —— 但注意它用
// https.get，**不认宿主机的代理/CA 设置**，而 spawnCurl 的整个设计意图就是继承
// 这些设置。校验一律走 spawnCurl，不要接到这里。
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

/**
 * 从通道的 checksums.txt 取出 `asset` 的期望 sha256；拿不到则返回 null。
 *
 * 语义与 install.sh 对齐：有清单才校验，没清单放行（旧版本、自建的 GitHub 前缀
 * 都可能没有 checksums.txt）。RATSA_REQUIRE_CHECKSUM=1 时把「没有清单」变成硬
 * 失败，供 CI 冒烟测试使用。
 *
 * 走 spawnCurl 而不是本文件里的 fetch()：fetch 用 https.get，不认宿主机的代理
 * 设置，在企业 MITM 代理网络里会失败 —— 而那恰恰是最需要校验的网络。
 */
function expectedChecksum(asset) {
  const text = spawnCurl(`${releaseBase()}/checksums.txt`, { maxTime: 30 })
  if (!text) return null
  for (const line of String(text).split('\n')) {
    // sha256sum 输出「<hex>␣␣<name>」；shasum -b 会多一个 `*` 前缀，一并容忍。
    const parts = line.trim().split(/\s+/)
    if (parts.length === 2 && parts[1].replace(/^\*/, '') === asset) return parts[0]
  }
  return null
}

/** Download the platform binary. Returns the path, or null when unavailable. */
function download({ silent = SILENT } = {}) {
  const asset = assetName(releaseVersion())
  const url = `${releaseBase()}/${asset}`
  const requireChecksum = process.env.RATSA_REQUIRE_CHECKSUM === '1'
  lastFailure = null
  try {
    const target = homeBinPath()
    fs.mkdirSync(path.dirname(target), { recursive: true })

    // 最多两次：发版瞬间别名与 checksums.txt 无法原子更新，第二次会把两者都重取一遍。
    // 校验不通过时**绝不落盘** —— 这正是这个循环存在的意义。
    for (let attempt = 0; attempt < 2; attempt++) {
      const want = expectedChecksum(asset)
      if (!want && requireChecksum) {
        throw new Error(`通道未提供 ${asset} 的校验值（RATSA_REQUIRE_CHECKSUM=1）`)
      }
      if (!want) log(`ratsa: 通道未提供 ${asset} 的校验值，跳过校验`)

      // Synchronous HTTP keeps postinstall simple (no async race with npm).
      log(`ratsa: 正在下载 ${url}`)
      const bin = spawnCurl(url)
      if (!bin) throw new Error('下载失败')

      if (want) {
        try {
          verifyChecksum(bin, want)
          log('ratsa: sha256 校验通过')
        } catch (e) {
          if (attempt === 0) {
            log(`ratsa: ${e.message}，重试一次`)
            continue
          }
          // 这一条必须让用户看见，即便 postinstall 是静默的。
          throw new Error(`${e.message}；已丢弃下载内容，未写入 ${target}`)
        }
      }

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
    }
    throw new Error('下载失败')
  } catch (e) {
    lastFailure = e.message
    // 非「下载失败」的都是安全相关（校验不通过 / 缺清单），必须可见。
    const say = e.message === '下载失败' ? log : warn
    say(`ratsa: 自动下载未完成（${e.message}）。可稍后重试，或用 cargo install --path ratsa-harness。`)
    return null
  }
}

/** Use the system curl/wget so we inherit proxy/CA settings of the host. */
function spawnCurl(url, { maxTime = 120 } = {}) {
  const { spawnSync } = require('child_process')
  const attempts = [
    ['curl', ['-fsSL', '--max-time', String(maxTime), url]],
    ['wget', ['-qO-', `--timeout=${maxTime}`, url]],
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

module.exports = {
  download,
  lastError,
  expectedChecksum,
  fetch,
  sha256,
  verifyChecksum,
  assetName,
  platformTag,
}
