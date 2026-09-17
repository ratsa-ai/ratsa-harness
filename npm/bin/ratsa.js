#!/usr/bin/env node
// `npx @ratsa/cli …` entry point.
//
// Resolves the real binary (vendored → ~/.ratsa/bin → local build), downloads it
// on first use when missing, then execs it with stdio inherited. Inherited stdio
// matters: `ratsa mcp` is a stdio MCP server, so stdin/stdout must pass straight
// through, and the child's exit code/signals must propagate to the caller.

const { spawnSync } = require('child_process')
const { resolveBinary, releaseBase } = require('../lib/platform.js')

function main() {
  let bin = resolveBinary()
  let why = null

  if (!bin) {
    // First run: try to fetch the release binary (best effort, never throws).
    try {
      const install = require('../install.js')
      bin = install.download({ silent: false })
      if (!bin) why = install.lastError()
    } catch {
      /* fall through to the message below */
    }
  }

  if (!bin) {
    const lines = ['未找到 ratsa 可执行文件。']
    if (why) {
      // 把原因原样带出来：校验不通过和「没网」的处置方式完全不同，不能混成一句。
      lines.push('', `自动下载未成功，原因：${why}`)
    }
    lines.push(
      '',
      '任选一种方式：',
      '  1) 从发布通道下载：RATSA_RELEASE_BASE=' + releaseBase() + ' npx @ratsa/cli --version',
      '  2) 用 Rust 自行编译：cargo install --path ratsa-harness',
      '  3) 已有二进制：RATSA_BIN=/path/to/ratsa npx @ratsa/cli …',
      '',
      '详见 https://ratsa.ai/harness',
      '',
    )
    process.stderr.write(lines.join('\n'))
    process.exit(1)
  }

  const res = spawnSync(bin, process.argv.slice(2), { stdio: 'inherit' })
  if (res.error) {
    process.stderr.write(`执行失败：${res.error.message}\n`)
    process.exit(1)
  }
  if (res.signal) {
    process.kill(process.pid, res.signal)
    return
  }
  process.exit(res.status === null ? 1 : res.status)
}

main()
