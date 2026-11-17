# NFD2NFC Desktop

Mac에서 Windows로 파일을 보내면 한글 파일명이 `ㅎㅏㄴㄱㅡㄹ.txt`처럼 자소 단위로 갈라져 보입니다. NFD2NFC Desktop은 이 문제를 고치는 macOS 메뉴바 앱입니다. Rust와 Tauri 2로 만들었고, Intel Mac과 Apple Silicon Mac에서 모두 동작합니다(universal binary).

## 왜 자소가 분리되나요?

| | 저장 형식 | `한` |
|---|---|---|
| macOS (APFS/HFS+) | **NFD** (분해형) | `ㅎ` + `ㅏ` + `ㄴ` (U+1112 U+1161 U+11AB) |
| Windows, 웹, 대부분의 OS | **NFC** (완성형) | `한` (U+D55C) |

Windows 탐색기, SMB 공유, Finder로 만든 zip 같은 곳에서는 NFD 이름이 다시 합쳐지지 않습니다. 그래서 자모가 따로따로 보입니다. APFS는 정규화 형식을 구분하지 않기 때문에, 같은 파일을 NFC 이름으로 바꿔 저장해도 macOS에서는 아무 문제가 없습니다.

## 기능

| 탭 | 기능 |
|---|---|
| **홈** | 감시 시작·중지·재시작, 이번 세션 변환 수, 로그인 시 자동 실행, 전체 디스크 접근 권한 상태 |
| **감시 폴더** | 감시(watch)·제외(ignore) 규칙 테이블. 범위는 하위 전체(recursive) 또는 바로 아래만(children)으로 지정하고, 각 규칙의 적용 상태(✓ 적용 / ~ 중복 / ! 무시됨 / ✗ 오류)와 설명을 보여 줍니다. 폴더를 끌어다 놓아 추가할 수 있습니다. |
| **변환 · 압축** | 파일이나 폴더를 끌어다 놓으면 NFD 이름의 개수를 분석합니다. 그 자리에서 이름을 바꾸거나(미리보기 지원, NFC→NFD 역변환 가능), 원본을 건드리지 않고 **Windows용 zip**을 만듭니다. zip은 이름을 NFC로 넣고 UTF-8 플래그를 설정하며, `.DS_Store`와 `._*` 파일을 제외합니다. |
| **탐색기** | 폴더를 돌아다니며 각 항목의 `NFC`/`NFD`/`혼합` 배지를 확인하고 바로 변환합니다. |
| **로그** | 자동 변환, 수동 변환, 오류 기록을 보여 줍니다. 필터와 자동 스크롤을 지원하고 `~/Library/Logs/com.nfd2nfc.desktop/`에 파일로도 저장합니다. |

메뉴바에서는 창을 닫아도 감시가 계속됩니다. 메뉴바 아이콘으로 감시를 켜고 끄거나, 창을 열거나, 앱을 종료할 수 있습니다.

## 구조

```
├── crates/nfd2nfc-core/      # UI와 무관한 핵심 로직 (단위 테스트 포함)
│   ├── normalizer.rs         #   NFD↔NFC 변환, F_GETPATH로 실제 이름 조회, 충돌 검사
│   ├── config.rs             #   watch/ignore 규칙 모델 + 상태 평가 (원본 nfd2nfc 설정 형식과 호환)
│   ├── zipper.rs             #   Windows 호환 NFC zip 생성
│   └── fs_util.rs
├── src-tauri/                # Tauri 2 앱 (Rust)
│   ├── src/watcher.rs        #   FSEvents(notify) 감시 → 50ms 디바운스 → 자동 변환
│   ├── src/state.rs          #   앱 상태, 감시 시작/중지/재시작
│   ├── src/tray.rs           #   메뉴바 아이콘
│   ├── src/commands.rs       #   프런트엔드 invoke 커맨드
│   ├── src/logs.rs           #   링 버퍼 + 파일 로그 + UI 이벤트
│   └── src/fda.rs            #   전체 디스크 접근 권한 감지
└── src/                      # 프런트엔드 (Vite + TypeScript, 프레임워크 없음)
    ├── main.ts               #   탭 라우팅, 드래그앤드롭, 백엔드 이벤트
    └── tabs/                 #   home / config / convert / browser / logs
```

설정 파일 위치는 `~/Library/Application Support/com.nfd2nfc.desktop/config.toml`입니다.

```toml
watcher_enabled = true

[[paths]]
path = "~/Downloads"
action = "watch"      # watch | ignore
mode = "recursive"    # recursive | children (ignore는 항상 recursive)
```

## 개발

### 준비

```bash
# Rust — https://rustup.rs 공식 설치 스크립트 권장
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
# Intel + Apple Silicon 타깃 모두 추가 (universal 빌드에 필요)
rustup target add x86_64-apple-darwin aarch64-apple-darwin

# Node 20+
npm install
```

### 실행과 테스트

```bash
npm run app:dev                 # 개발 모드 (핫 리로드)
cargo test -p nfd2nfc-core      # 핵심 로직 테스트
cargo clippy --workspace -- -D warnings
```

### 배포 빌드

```bash
npm run app:build:universal     # Intel + Apple Silicon 공용 .app / .dmg
npm run app:build:intel         # x86_64 전용
npm run app:build:arm           # arm64 전용
```

결과물은 `target/universal-apple-darwin/release/bundle/{macos,dmg}/`에 생성됩니다.

> 서명하지 않은 빌드를 다른 Mac에서 처음 열 때는 Finder에서 앱을 **우클릭 → 열기**로 실행해야 합니다.
> 정식으로 배포하려면 Apple Developer ID로 서명하고 공증(notarization)을 받아야 합니다.

## 참고 사항

- **감시는 새로 생기거나 이름이 바뀐 항목에만 적용됩니다.** 이미 있는 파일은 ‘감시 폴더’ 탭의 **기존 파일 정리** 버튼이나 ‘변환 · 압축’ 탭으로 한 번에 정리하세요.
- **전체 디스크 접근 권한**은 필수가 아닌 권장 사항입니다. 허용하지 않으면 데스크탑·문서·다운로드 폴더마다 접근 허용 창이 뜹니다. 이 앱은 감시자가 `.app` 안에서 직접 실행되므로, 원본 CLI와 달리 권한을 **앱 단위로 한 번만** 주면 됩니다.
- 이름 충돌 방지: 정규화를 구분하는 볼륨에서 NFC 이름의 다른 파일이 이미 있으면 덮어쓰지 않고 건너뜁니다.
- HFS+처럼 이름을 항상 NFD로 저장하는 볼륨에서는 변환해도 효과가 없습니다. 이때는 오류로 기록하고 같은 경로를 반복해서 처리하지 않습니다.

## 크레딧

규칙 모델, `F_GETPATH`로 실제 디스크 이름을 읽는 기법, 탭 구성은 [elgar328/nfd2nfc](https://github.com/elgar328/nfd2nfc)(MIT)를 참고했습니다.

## 라이선스

MIT
