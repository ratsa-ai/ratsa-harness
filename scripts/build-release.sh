#!/usr/bin/env bash
# Build RATSA-Harness release artifacts.
#
#   ./scripts/build-release.sh                 # 当前平台 → dist/
#   ./scripts/build-release.sh --all           # 交叉编译 darwin/linux × x64/arm64
#   ./scripts/build-release.sh --vendor-npm    # 额外把当前平台二进制放进 npm/vendor/
#
# 产物命名约定（npm 包装与 /downloads 通道共用，改这里就要同步改
# npm/lib/platform.js 的 assetName()）：
#   dist/ratsa-<version>-<os>-<arch>[.exe]
#   dist/ratsa-latest-<os>-<arch>[.exe]   ← 稳定别名（安装脚本 / 下载按钮用它）
#   dist/checksums.txt
#
# 托管方式任选：
#   * ratsa.ai 的 /downloads 目录（server 侧静态服务，见 prd/ratsa-harness-cli.md）
#   * GitHub Releases：RATSA_RELEASE_BASE=https://github.com/<org>/<repo>/releases/download/v<ver>
set -euo pipefail

cd "$(dirname "$0")/.."
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
OUT=dist
ALL=0
VENDOR=0

for arg in "$@"; do
  case "$arg" in
    --all) ALL=1 ;;
    --vendor-npm) VENDOR=1 ;;
    *) echo "未知参数：$arg" >&2; exit 2 ;;
  esac
done

mkdir -p "$OUT"
: > "$OUT/checksums.txt"

build_one() {
  local target="$1" os="$2" arch="$3"
  local base="ratsa-${VERSION}-${os}-${arch}"
  echo "==> $base ($target)"
  if [ -n "$target" ]; then
    cargo build --release --offline --target "$target" >/dev/null
    cp "target/${target}/release/ratsa" "$OUT/${base}"
  else
    cargo build --release --offline >/dev/null
    cp target/release/ratsa "$OUT/${base}"
  fi
  chmod +x "$OUT/${base}"
  (cd "$OUT" && shasum -a 256 "${base}" >> checksums.txt)
  # Stable alias: the installer + download buttons use the version-free name
  # (see the server's /downloads channel and web HarnessDownloads).
  local alias="ratsa-latest-${os}-${arch}"
  cp "$OUT/${base}" "$OUT/${alias}"
  (cd "$OUT" && shasum -a 256 "${alias}" >> checksums.txt)
  if [ "$VENDOR" = "1" ]; then
    local here_os here_arch
    here_os=$(uname -s | tr '[:upper:]' '[:lower:]')
    [ "$here_os" = "darwin" ] && here_os=darwin
    case "$(uname -m)" in arm64) here_arch=arm64 ;; x86_64) here_arch=x64 ;; esac
    if [ "$os" = "$here_os" ] && [ "$arch" = "$here_arch" ]; then
      mkdir -p npm/vendor
      cp "$OUT/${base}" "npm/vendor/ratsa-${os}-${arch}"
      chmod +x "npm/vendor/ratsa-${os}-${arch}"
      echo "    vendored → npm/vendor/ratsa-${os}-${arch}"
    fi
  fi
}

if [ "$ALL" = "1" ]; then
  build_one aarch64-apple-darwin darwin arm64
  build_one x86_64-apple-darwin darwin x64
  build_one x86_64-unknown-linux-gnu linux x64
  build_one aarch64-unknown-linux-gnu linux arm64
else
  case "$(uname -s)" in
    Darwin) os=darwin ;;
    Linux) os=linux ;;
    *) echo "未知 OS：$(uname -s)" >&2; exit 2 ;;
  esac
  case "$(uname -m)" in
    arm64 | aarch64) arch=arm64 ;;
    x86_64 | amd64) arch=x64 ;;
    *) echo "未知架构：$(uname -m)" >&2; exit 2 ;;
  esac
  build_one "" "$os" "$arch"
fi

echo
echo "产物："
ls -lh "$OUT" | tail -n +2
echo
echo "npm 本地验证：node npm/bin/ratsa.js --version"
echo "npm 发布：cd npm && npm publish --access public"
