# RATSA-Harness (CLI: `ratsa`)

**English** · [中文](./README.zh-CN.md)

Wire RATSA.ai into **the Agent you already use**: install a skill / MCP server, bring a
**least-privilege** key, and close the loop — discover → generate/pull → run locally → report back.

```bash
# 1) Install (six agent targets, see below)
npx @ratsa/cli install --agent all           # current project
npx @ratsa/cli install --agent all --global  # your home directory (works in every workspace)
npx @ratsa/cli agents                        # which agents are supported / which files get written

# 2) Credentials (recommended: choose scopes + bind an owning account)
npx @ratsa/cli login --email you@example.com
npx @ratsa/cli scopes
npx @ratsa/cli keys create --name laptop-agent \
  --scopes sof:read,harness:read,package:pull --bound-slug your-handle --use

# 3) The loop
npx @ratsa/cli whoami
npx @ratsa/cli search servo
npx @ratsa/cli package <slug> --meta   # platform-generated device Harness package (describe only)
npx @ratsa/cli package <slug>          # download (sha256 verified)
npx @ratsa/cli sof-file <slug>         # fetch the SOF file (can be posted back)
npx @ratsa/cli report <slug> --score 88 --passed 22 --failed 2
```

## Why a CLI + plugin instead of yet another SDK

What the agent era actually lacks is not another API — it is **making the Agent know how to use
it**. So this project ships two halves:

1. **One binary** (Rust, no runtime dependencies): does the network calls, permissions,
   checksum verification, and writes files to disk.
2. **One installable document** (skill / rules / `AGENTS.md`): tells the Agent the permission
   boundary, the kinds of packages, and how to read failures — including behavioural rules such
   as "do not retry `missing_scope`" and "stop on checksum mismatch".

The MCP tool surface is exposed at the same time, so an Agent never has to memorise commands.

## Boundaries (an Agent reading this should know them too)

- The platform **never executes** your code and **never relays** your business data; device
  scanning and evaluation run locally.
- The CLI does exactly three things: pull artifacts, push structured results, manage its own
  credentials. **No daemon, no telemetry.**
- Pulls verify the server-provided sha256 (`X-Ratsa-Checksum`); on mismatch it errors out and
  refuses to persist or execute the artifact.

## Install targets

| `--agent` | Files written | Purpose |
|---|---|---|
| `claude` | `.claude/skills/ratsa/SKILL.md` (`--global`: `~/.claude/skills/ratsa/SKILL.md`) | Claude Code skill |
| `mcp` | `.mcp.json` (merges `mcpServers.ratsa`) | stdio server for Claude Code / MCP-compatible clients |
| `cursor` | `.cursor/rules/ratsa.mdc` + `.cursor/mcp.json` | Cursor rules + MCP |
| `copilot` | `.github/copilot-instructions.md` + `.github/skills/ratsa/SKILL.md` + `.vscode/mcp.json` | GitHub Copilot (`--global` also writes the VS Code user prompts directory) |
| `agents` | `AGENTS.md` | Generic convention (Codex / any Agent that reads `AGENTS.md`) |
| `codex` | `<dir>/.codex/config.toml` (appends `[mcp_servers.ratsa]`) + `<dir>/.codex/RATSA.md`; `<dir>` defaults to `$HOME` (Codex only reads user-level config) | OpenAI Codex CLI |

- **Idempotent** by default: an existing entry with the same name is skipped; `--force`
  overwrites (an existing file is backed up to `.bak` first).
- `--dry-run` prints what would be written and the merge result, without touching disk.
- `--dir <path>` sets the root directory (useful for CI / container image builds).

## Commands

| Command | Description |
|---|---|
| `install` | Install / update each Agent's skill and MCP entry |
| `agents` | List supported Agents, their form, files written, and MCP registration (`--json`) |
| `login` / `logout` | Session (used only to create and manage keys); `RATSA_PASSWORD` for non-interactive use |
| `whoami` | Identity + key scopes + owner binding (`--json`) |
| `scopes` | Permission table (fetched from `/api/meta`, with per-scope meaning and default set) |
| `keys list` / `keys create` / `keys revoke` | Choose scopes + optional `--bound-slug`; `--use` applies immediately |
| `config set/show/clear` | Local credentials (`~/.ratsa/config.json`, mode 0600) |
| `search` / `sof` | Cross-vendor search and detail (`--json`) |
| `manifest` / `packages` | Readiness + issue list + unified `packages[]` |
| `package` | **Generate the device Harness package server-side** and download it; `--meta` describes it only (files / readiness / size) |
| `sof-file` | Fetch the SOF file (`kind=ratsa.sof.json`, can be posted straight back to `POST /api/sof`) |
| `pull` | Pull any kind: `--kind device\|eval-repo\|service`, written to `~/.ratsa/packages/<slug>/<kind>/` |
| `kb` | Pull the SOF knowledge base: index by default, `--sof <slug>` for a device level, `--doc <key\|slug>` prints or saves the Markdown (checksum verified) |
| `report` | Report a measured run (`source=agent`) into the device's Harness Eval |
| `feedback` | Submit feedback to the vendor |
| `mcp` | Run as an MCP stdio server (14 tools) |
| `upgrade` | Update the `ratsa` binary itself from the release channel |

`upgrade` exists because the npm package version is independent of the CLI release tag
(see `npm/README.md`). npm therefore reports "up to date" forever and never re-runs our
`postinstall`, so a binary-only release reaches nobody who already installed. `upgrade`
fetches `ratsa-latest-<os>-<arch>` from `RATSA_RELEASE_BASE`, verifies it against
`checksums.txt`, and replaces `~/.ratsa/bin/ratsa` in place — refusing to install (and
leaving the existing binary untouched) if the checksum does not match.

Environment variables (handy for CI / containers, no config file needed):
`RATSA_BASE_URL`, `RATSA_KEY_ID`, `RATSA_KEY_SECRET`, `RATSA_CONFIG`, `RATSA_HOME`, `RATSA_PASSWORD`.

## Permission table (key / secret scopes)

What a key can do is decided **only** by its scopes; the default set is "read + pull", i.e. the
minimum for an Agent to get work done.

| scope | Group | Meaning |
|---|---|---|
| `sof:read` | Read | Read public SOFs, the catalog, `/api/meta` |
| `sof:read:private` | Read | Additionally see private SOFs of the key owner (or its bound slug) |
| `harness:read` | Read | Harness manifests, readiness, package listings |
| `eval:read` | Read | Eval listings, Eval Repo metadata and manifests |
| `package:pull` | Pull | Download device Harness packages / Eval Repos / service test packs |
| `feedback:submit` | Report | Submit feedback as an Agent / device |
| `eval:report` | Report | Report measured runs (`kind=harness`, `source=sdk\|agent`) |
| `eval:publish` | Publish | Create / update Eval Repos |
| `sof:write` | Publish | Create / update / delete SOFs owned by that identity |
| `order:manage` | Publish | Take on / start / deliver evaluation engagements as a service provider |
| `key:manage` | Admin | List / rotate / delete keys of that account |
| `admin` | Admin | Grantable by admins only; implies every scope |

- **Default** (when unspecified): `sof:read,harness:read,eval:read,package:pull`.
- Visibility still stacks on top: the scope decides *whether this class of operation is
  allowed*, while the publisher's **visibility tier** decides *whether this particular record
  is visible* (login-only → 401 `login_required`; private → 403/404).
- A missing scope returns **403 `missing_scope`** with `scope` and `scopes` in the body, so the
  Agent can tell the user exactly which one to add.

## Owner binding (`bound_slug`)

Point a key at an account handle and the key then **acts as that account**: the private assets
it reads, its publishes and submissions all belong to that account.

- Use case: separate "who holds this key" from "whom this key speaks for" — e.g. an outsourcer
  or integrator working on a client's behalf, or a device reporting measurements with a bound key.
- Limits: **a regular user can only bind their own handle** (privilege-escalation guard). Binding
  someone else's handle currently requires an admin; a proper **authorisation invite** flow is
  needed for "vendor authorises a third party to hold a key under its identity" (see below).
- Server side: `/api/v1/me` returns `acting_as`; the key list returns `bound_user`.

## MCP tools (14)

`ratsa_whoami` · `ratsa_scopes` · `ratsa_search_sofs` · `ratsa_read_sof` ·
`ratsa_device_harness` · `ratsa_list_packages` · `ratsa_pull_package` ·
`ratsa_list_agents` · `ratsa_generate_package` · `ratsa_get_sof_file` ·
`ratsa_report_run` · `ratsa_submit_feedback` ·
`ratsa_list_kb` · `ratsa_read_kb`

The protocol is MCP over stdio (newline-delimited JSON-RPC 2.0) with no extra dependencies,
which keeps it easy to audit.

## Build and distribution

```bash
cd ratsa-harness
cargo build --release            # produces target/release/ratsa
cargo build --offline            # offline build once dependencies are cached
./scripts/build-release.sh --all --vendor-npm   # per-platform artifacts + npm/vendor/ with the current platform binary
```

`npx @ratsa/cli` relies on the **launcher package** in `npm/` (it does not embed binaries — it
resolves or downloads one at runtime, see `npm/README.md`):

```bash
node npm/bin/ratsa.js --version   # local check
cd npm && npm publish             # registry + access are pinned by package.json's publishConfig
```

The release channel is set by `RATSA_RELEASE_BASE` (defaults to `https://ratsa.ai/downloads`;
a GitHub Releases prefix works too). Asset naming is
`ratsa-<version>-<os>-<arch>[.exe]` plus `checksums.txt`.

## Known limitations / TODO

- Binding **someone else's** handle needs an authorisation invite mechanism (currently
  admin-only, see `prd/ratsa-harness-issues.md` A-10).
- Windows has not been verified separately (the code only uses std + ureq, so it should work).
- The `--global` VS Code Copilot target writes into the user prompts directory and covers
  instructions only (it does not write a user-level MCP config).
- New Agent targets (Windsurf / Zed / Cline / Continue / Aider / Gemini CLI, …) are not listed
  individually: adding one = one entry in `target_table()` in `src/install.rs` + one line in
  `payloads()` for the path, then sync the AGENTS table in
  `web/src/components/HarnessCliIntro.jsx` and `prd/ratsa-harness-cli.md` §2.
