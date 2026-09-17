# `ratsa` (npm wrapper)

**English** · [中文](./README.zh-CN.md)

Makes `npx @ratsa/cli …` work. This package **does not bundle a binary** — it is a launcher: it finds
the real `ratsa` executable (downloading it if necessary) and execs it.

```
npm/
  package.json      name=@ratsa/cli, bin={ratsa: bin/ratsa.js}, postinstall=install.js
  bin/ratsa.js      resolution order: RATSA_BIN → ~/.ratsa/bin → vendor/ → local target/{release,debug}
  lib/platform.js   platform label, asset naming, resolution logic (same convention as scripts/build-release.sh)
  install.js        best-effort download (postinstall never fails), keeps a copy in vendor/
  README.md         English docs (this file — shown on the npm page)
  README.zh-CN.md   Chinese docs (top of both files links to the other language)
```

## Why a launcher instead of per-platform sub-packages

- The tarball is only a few KB, so `npm i` never slows down or fails over a 2.5 MB binary download.
- One `npm publish` covers macOS / Linux / Windows (x64 + arm64).
- The binary lives in `~/.ratsa/bin/`, **immune to `npx` cache cleaning**.
- Offline or firewalled environments: `postinstall` silently skips, `npx @ratsa/cli` retries on first
  run, and if that fails too it prints the `cargo install --path ratsa-harness` alternative.

## Release process

**Two independent releases.** What you changed decides what to bump:

| Changed | Bump | Tag `vX.Y.Z` | `npm publish` |
|---|---|---|---|
| the CLI binary (`src/`) | `Cargo.toml` **and** `Cargo.lock` | yes | no |
| this package (`install.js` / `lib/` / `bin/`) | `npm/package.json` | no | yes |

They are independent because `install.js` resolves the download URL from `latest`
(override with `RATSA_VERSION`), not from this package's own version. So a binary-only
fix reaches users without a pointless npm version bump, and an npm-only fix needs no
rebuild. Earlier versions derived the URL from `package.json`, which forced the two to
move together — do not reintroduce that.

One consequence to keep in mind: **an installed user does not pick up a binary-only
release by themselves.** Nothing re-runs `postinstall` while this package's version
stays put, so npm cheerfully reports "up to date" while `~/.ratsa/bin/ratsa` stays
frozen at install time. `ratsa upgrade` is the supported way out — it fetches from the
release channel directly and does not involve npm at all. (Bumping this package's
version does refresh the binary as a side effect, since `postinstall` runs again — a
reason to bump when you want a rollout, never a reason to couple the numbers.)

### Binary release

```bash
# 1) bump `version` in Cargo.toml AND Cargo.lock. The lock file matters: CI builds
#    with `cargo build --locked`, which fails outright if the lock is stale.
# 2) commit, then tag — pushing the tag is what starts the release:
git tag vX.Y.Z && git push origin vX.Y.Z
```

CI (`.github/workflows/release.yml`) then builds all six platform targets, publishes
`ratsa-<version>-<os>-<arch>[.exe]` + `checksums.txt` to the GitHub Release, and mirrors
the same set plus the `ratsa-latest-<os>-<arch>[.exe]` aliases to Huawei OBS behind
`https://ratsa.ai/downloads`. Those `ratsa-latest-*` aliases are exactly what makes the
two releases independent — they are the name every install fetches.

Offline fallback / CI down: `./scripts/build-release.sh --all --vendor-npm` produces the
same asset names under `dist/`; upload them wherever `RATSA_RELEASE_BASE` points.

### npm package release

```bash
node npm/bin/ratsa.js --version   # local sanity check
node npm/bin/ratsa.js agents
cd npm && npm publish             # registry + access are pinned by publishConfig
```

Published as **`@ratsa/cli`**. The unscoped name `ratsa` is rejected by npm's typosquatting check
("too similar to existing packages ramda,nats"). That check runs *only at publish time*, so a
registry `404` does **not** mean a name is publishable — `ratsa` returned 404 and still could not be
published. The `bin` name stays `ratsa`, so once installed the command is plain `ratsa`; only the
`npx` form carries the scope.

## Environment variables

| Variable | Purpose |
| --- | --- |
| `RATSA_BIN` | Point at a specific executable, skipping resolution (CI / self-built) |
| `RATSA_RELEASE_BASE` | Release channel prefix, defaults to `https://ratsa.ai/downloads` |
| `RATSA_QUIET=1` | Silence `postinstall` |
| `RATSA_HOME` | User directory (defaults to `~/.ratsa`); the binary lands in `$RATSA_HOME/bin` |

## Unrelated but often confused

`npx @ratsa/cli mcp` **inherits stdin/stdout** (MCP is a stdio protocol); the launcher uses
`spawnSync(bin, args, { stdio: 'inherit' })` and forwards the exit code and signals — do not turn
stdio into a pipe when editing this file, or the Agent-side handshake will hang.
