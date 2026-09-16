#!/usr/bin/env bash
# inno-creed GUI·CLI 인스톨러 배포 zip을 만든다 (macOS / Linux).
#
# 전제: inno-creed 본체와 installer가 이미 release로 빌드돼 있어야 한다.
#   cargo build --release --bin inno-creed
#   cargo build --release -p installer
#
# 확장 프로그램은 **전 OS 공통 정식 경로**다(docs/INSTALL.md 참고). macOS/Linux도
# 쿠키 직접 읽기는 세션 쿠키·키체인/키링 권한 때문에 되는지가 환경에 달려 있어
# 보장되지 않는다. 그래서 .ps1과 똑같이 payload/extension/을 담는다 — 여기서 빠지면
# installer가 확장 안내 화면 자체를 건너뛴다(그 화면은 이 폴더 유무로 켜진다).
#
# exe 안에 exe를 내장하지 않는다 — installer와 payload/를 zip 안에서
# 나란히 두고, installer는 실행 시 자기 옆에서 payload를 찾는다.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

OS="$(uname -s)"
case "$OS" in
  Darwin) TARGET_NAME="macos-arm64" ;;
  Linux)
    ARCH="$(uname -m)"
    case "$ARCH" in
      x86_64) TARGET_NAME="linux-x86_64" ;;
      aarch64) TARGET_NAME="linux-aarch64" ;;
      *) echo "지원하지 않는 아키텍처: $ARCH" >&2; exit 1 ;;
    esac
    ;;
  *) echo "지원하지 않는 OS: $OS" >&2; exit 1 ;;
esac

for f in target/release/installer target/release/installer-cli target/release/inno-creed; do
  if [ ! -f "$f" ]; then
    echo "$f 가 없습니다. 먼저 'cargo build --release --bin inno-creed' 와 'cargo build --release -p installer' 를 실행하세요." >&2
    exit 1
  fi
done

mkdir -p "$ROOT/.claude-workspace"
STAGE="$(mktemp -d "$ROOT/.claude-workspace/installer-stage.XXXXXX")"
trap 'rm -rf "$STAGE"' EXIT

mkdir -p "$STAGE/payload/extension/icons"
cp target/release/installer "$STAGE/installer"
cp target/release/installer-cli "$STAGE/installer-cli"
cp target/release/inno-creed "$STAGE/payload/inno-creed"
chmod +x "$STAGE/installer" "$STAGE/installer-cli" "$STAGE/payload/inno-creed"
# icons/를 빠뜨리면 manifest.json이 선언한 파일이 없어 Chrome이 로드를 **거부**한다.
cp extension/manifest.json extension/background.js "$STAGE/payload/extension/"
cp extension/icons/* "$STAGE/payload/extension/icons/"

OUT_DIR="${1:-dist}"
mkdir -p "$OUT_DIR"
ZIP_PATH="$(cd "$OUT_DIR" && pwd)/inno-creed-installer-${TARGET_NAME}.zip"
rm -f "$ZIP_PATH"

# `zip`은 최소 설치된 리눅스에 없는 경우가 있다(실제로 빌드 머신 한 대가 그랬다).
# python3는 있으므로 폴백을 둔다 — 다만 **실행 권한을 zip 엔트리에 직접 실어야**
# 한다. 기본 writestr은 모드를 0으로 남겨서, 풀면 installer가 실행 불가가 된다.
if command -v zip >/dev/null 2>&1; then
  (cd "$STAGE" && zip -r -q "$ZIP_PATH" .)
elif command -v python3 >/dev/null 2>&1; then
  python3 - "$STAGE" "$ZIP_PATH" <<'PY'
import os, sys, zipfile

stage, out = sys.argv[1], sys.argv[2]
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
    for root, _, files in os.walk(stage):
        for name in sorted(files):
            full = os.path.join(root, name)
            info = zipfile.ZipInfo(os.path.relpath(full, stage).replace(os.sep, "/"))
            info.external_attr = (os.stat(full).st_mode & 0xFFFF) << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            with open(full, "rb") as f:
                z.writestr(info, f.read())
PY
else
  echo "zip 또는 python3 중 하나가 필요합니다." >&2
  exit 1
fi

echo "만든 파일: $ZIP_PATH"
