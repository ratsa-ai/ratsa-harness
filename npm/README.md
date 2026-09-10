# `ratsa` (npm wrapper)

**English** · [中文](./README.zh-CN.md)

Makes `npx ratsa …` work. This package **does not bundle a binary** — it is a launcher: it finds
the real `ratsa` executable (downloading it if necessary) and execs it.

```
npm/
  package.json      name=ratsa, bin={ratsa: bin/ratsa.js}, postinstall=install.js
  bin/ratsa.js      resolution order: RATSA_BIN → vendor/ → ~/.ratsa/bin → local target/{release,debug}
  lib/platform.js   platform label, asset naming, resolution logic (same convention as scripts/build-release.sh)
  install.js        best-effort download (postinstall never fails), keeps a copy in vendor/
  README.md         English docs (this file — shown on the npm page)
  README.zh-CN.md   Chinese docs (top of both files links to the other language)
```

## Why a launcher instead of per-platform sub-packages

- The tarball is only a few KB, so `npm i` never slows down or fails over a 2.5 MB binary download.
- One `npm publish` covers macOS / Linux / Windows (x64 + arm64).
- The binary lives in `~/.ratsa/bin/`, **immune to `npx` cache cleaning**.
- Offline or firewalled environments: `postinstall` silently skips, `npx ratsa` retries on first
  run, and if that fails too it prints the `cargo install --path ratsa-harness` alternative.

## Release process (one-time)

```bash
# 1) Build per-platform artifacts + copy the current platform's binary into npm/vendor/
cd ratsa-harness && ./scripts/build-release.sh --all --vendor-npm

# 2) Upload dist/ to the release channel (either way; asset naming must match)
#    a) the /downloads directory on ratsa.ai (default RATSA_RELEASE_BASE)
#    b) GitHub Releases:
#       RATSA_RELEASE_BASE=https://github.com/<org>/<repo>/releases/download/v0.1.0
#    Assets: ratsa-<version>-<os>-<arch>[.exe] + checksums.txt

# 3) Local check (publishing not required)
node npm/bin/ratsa.js --version
node npm/bin/ratsa.js agents

# 4) Publish
cd npm && npm publish --access public
```

`ratsa` / `@ratsa/cli` / `ratsa-harness` are all currently unclaimed on npm (checked 2026-09-10;
the registry returned 404). If `ratsa` gets taken later, switch to the scoped `@ratsa/cli` and keep
the `bin` name as `ratsa` (`npx @ratsa/cli …`).

## Environment variables

| Variable | Purpose |
| --- | --- |
| `RATSA_BIN` | Point at a specific executable, skipping resolution (CI / self-built) |
| `RATSA_RELEASE_BASE` | Release channel prefix, defaults to `https://ratsa.ai/downloads` |
| `RATSA_QUIET=1` | Silence `postinstall` |
| `RATSA_HOME` | User directory (defaults to `~/.ratsa`); the binary lands in `$RATSA_HOME/bin` |

## Unrelated but often confused

`npx ratsa mcp` **inherits stdin/stdout** (MCP is a stdio protocol); the launcher uses
`spawnSync(bin, args, { stdio: 'inherit' })` and forwards the exit code and signals — do not turn
stdio into a pipe when editing this file, or the Agent-side handshake will hang.
