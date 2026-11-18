#!/usr/bin/env bash
# 이 Mac의 CPU에 맞는 빌드 결과물(.app)을 /Applications에 설치한다.
#   ./scripts/install-local.sh            # 자동 판별 (Intel → x86_64, Apple Silicon → aarch64)
#   ./scripts/install-local.sh aarch64    # 강제 지정
set -euo pipefail

cd "$(dirname "$0")/.."

ARCH="${1:-}"
if [[ -z "$ARCH" ]]; then
  # Rosetta로 실행 중이어도 실제 하드웨어를 판별
  if [[ "$(sysctl -in hw.optional.arm64 2>/dev/null)" == "1" ]]; then ARCH=aarch64; else ARCH=x86_64; fi
fi

APP_NAME="NFD2NFC.app"
SRC="target/${ARCH}-apple-darwin/release/bundle/macos/${APP_NAME}"
DEST="/Applications/${APP_NAME}"

if [[ ! -d "$SRC" ]]; then
  echo "빌드 결과물이 없습니다: $SRC" >&2
  echo "먼저 빌드하세요: npm run app:build:$([[ $ARCH == aarch64 ]] && echo arm || echo intel)" >&2
  exit 1
fi

echo "▶ 설치 대상: $SRC ($ARCH)"

# 실행 중이면 종료 (메뉴바 상주 앱이라 창을 닫아도 살아 있음)
if pgrep -xq NFD2NFC; then
  echo "▶ 실행 중인 NFD2NFC 종료"
  osascript -e 'tell application id "com.nfd2nfc.desktop" to quit' >/dev/null 2>&1 || pkill -x NFD2NFC || true
  for _ in {1..20}; do pgrep -xq NFD2NFC || break; sleep 0.25; done
fi

# 이전 버전 교체
if [[ -d "$DEST" ]]; then
  echo "▶ 기존 버전 제거: $DEST"
  rm -rf "$DEST"
fi
ditto "$SRC" "$DEST"

# 직접 빌드한 앱은 격리 속성이 없지만, 복사·압축 해제 과정에서 붙었을 수 있어 제거
xattr -dr com.apple.quarantine "$DEST" 2>/dev/null || true

echo "▶ 서명 확인"
codesign --verify --deep --strict "$DEST" && echo "  서명 OK ($(codesign -dv "$DEST" 2>&1 | grep -E '^Signature' | cut -d= -f2))"
lipo -archs "$DEST/Contents/MacOS/NFD2NFC" | sed 's/^/  아키텍처: /'

echo "▶ 실행"
open "$DEST"
echo "완료: $DEST"
