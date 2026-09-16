# 설치 가이드

`inno-creed`는 아마란스(`gw.innogrid.com`) 그룹웨어를 다루는 **MCP 서버**입니다. 단독으로 실행하는 앱이 아니라, **Claude Code 같은 MCP 클라이언트에 등록해서** 대화로 사용합니다.

---

## 0. 비개발자라면 — GUI 인스톨러 (권장)

Claude Desktop 앱의 **채팅·Cowork 탭**에서 쓸 거라면, 아래 1~6번의 JSON 편집을 직접 할 필요가 없습니다.

1. [릴리즈](https://github.com/zilhak/inno-creed/releases/latest)에서 `inno-creed-installer-<OS>.zip`을 받습니다.
   - Windows x64: `inno-creed-installer-windows-x86_64.zip`
   - Windows ARM64: `inno-creed-installer-windows-aarch64.zip`
   - macOS: `inno-creed-installer-macos-arm64.zip`
   - Linux: `inno-creed-installer-linux-x86_64.zip` / `-linux-aarch64.zip`
2. **압축을 통째로 풉니다.** 안에 `installer`(Windows는 `installer.exe`)와 `payload/` 폴더가 나란히 들어있는데, **이 둘을 같은 자리에 둔 채로** `installer`를 실행하세요 — `installer`만 다른 곳으로 옮기면 설치할 파일을 못 찾습니다.
3. 실행이 막히면 — 미서명 배포판이라 OS마다 한 번씩 걸립니다.
   - **Windows**: SmartScreen("Windows가 PC를 보호했습니다") 경고 → **추가 정보 → 실행**.
   - **macOS**: Gatekeeper가 "'installer'을(를) 열지 않음"이라며 **아예 막습니다**. 화면 캡처로 따라가는
     [macOS 첫 실행 허용하기](mac-first-run.html)([웹](https://zilhak.github.io/inno-creed/mac-first-run.html))를 보세요 —
     한 번 실행을 시도해 경고를 띄운 뒤, **시스템 설정 → 개인정보 보호 및 보안 → [그래도 열기]**입니다.
     설치 도중 본체(`inno-creed`)에 대해 **같은 경고가 한 번 더** 뜨며, 그때도 같은 절차를 거쳐야 합니다.
     터미널을 쓸 수 있다면 압축 푼 폴더에 대고 한 줄로도 됩니다:
     ```sh
     xattr -dr com.apple.quarantine .        # 압축 푼 폴더 안에서
     ./installer
     ```
     (최신 macOS에서는 예전의 **우클릭 → 열기** 우회가 더 이상 통하지 않습니다.)
   - **Linux**: 압축 프로그램이 실행 권한을 떨어뜨렸다면 `chmod +x installer payload/inno-creed`.
4. 화면 안내를 따라갑니다: 환영 → Claude Desktop 설정 파일 자동 감지 → 설치 위치 확인 → (Claude Desktop이 켜져 있으면 종료 요청) → 설치 → 확장 프로그램 연결 안내 → 완료.
5. 완료 화면에 `doctor` 인증 확인 결과가 함께 뜹니다. Claude Desktop을 (다시) 켜면 채팅·Cowork 탭에서 바로 도구를 쓸 수 있습니다.
   **Code 탭과 Claude Code CLI는 설정 파일이 따로입니다**(`~/.claude.json`) — 거기서도 쓰려면 [3번](#3-mcp-클라이언트에-등록)의 `claude mcp add`로 한 번 더 등록하세요.

제거하고 싶으면 같은 `installer`를 `--uninstall` 옵션으로 실행하거나(`installer --uninstall`), Windows는 **설정 → 앱 → inno-creed → 제거**에서도 됩니다.

> **Claude Code CLI(터미널)만 쓸 거라면** 이 인스톨러 대신 [3번](#3-mcp-클라이언트에-등록)의 `claude mcp add` 한 줄이 더 간단합니다. GUI 인스톨러는 Claude Desktop의 설정 파일(`claude_desktop_config.json`)에 등록하는 방식이라 **채팅·Cowork 탭**을 겨냥합니다 — Code 탭은 Claude Code와 같은 설정을 쓰므로 이 인스톨러로는 붙지 않습니다.
>
> GUI 인스톨러가 안 되거나(사내 정책으로 실행 파일이 막힘 등), 직접 확인하며 진행하고 싶다면 아래 1번부터 수동으로 진행하세요.

### GUI와 CLI 인스톨러

**다음 릴리즈 및 현재 소스 빌드부터** 같은 ZIP에 두 실행 파일이 들어갑니다. 기존 릴리즈에 `installer-cli`가 없다면 아래의 기존 실행 방법을 사용하세요.

| 실행 파일 | 용도 |
|---|---|
| `installer.exe` (macOS/Linux: `installer`) | GUI 설치 화면 |
| `installer-cli.exe` (macOS/Linux: `installer-cli`) | 그래픽 초기화 없이 터미널에서 설치 |

Windows에서는 **`installer-cli.exe`를 더블클릭**하면 콘솔 창이 열립니다. 설치 완료·취소·오류 후에는 Enter를 눌러 닫을 수 있습니다. 이미 연 PowerShell에서는 다음처럼 실행하면 됩니다. 별도의 `--cli`나 `Start-Process`는 필요 없습니다.

```powershell
.\installer-cli.exe
# 제거
.\installer-cli.exe --uninstall
```

macOS/Linux에서는 터미널에서 `./installer-cli`를 실행합니다. 실행 파일과 `payload/`를 같은 폴더에 두세요. 두 실행 파일은 같은 설치·제거 로직을 사용하며, CLI로 설치하면 Windows 앱 목록의 제거 기능도 CLI로 실행됩니다.

설정 파일 후보를 확인하고, 다른 파일을 쓰려면 전체 경로를 입력합니다. 설치 위치는 Enter로 기본값을 사용하거나, 다른 상위 폴더를 입력하면 그 안의 `inno-creed` 폴더를 사용합니다. **설치 확인에서 `y`를 입력해야 설치됩니다**(Enter는 취소). Claude Desktop이 켜져 있으면 종료 후 다시 확인합니다. 확장 프로그램 연결 안내(전 OS)와 선택적 `doctor` 인증 진단도 제공됩니다. Ctrl+C로 중단할 수 있습니다.

제거 시 설치했던 위치를 선택하세요. 선택한 `inno-creed` 폴더의 파일을 삭제하므로 확인된 대상 폴더를 읽고 진행하세요. 설정 파일을 찾지 못하면 `-`로 등록 해제를 건너뛸 수 있으며, 이 경우 Claude Desktop 설정에 남은 등록은 직접 정리해야 합니다.

#### 기존 릴리즈 (`installer-cli`가 없는 v2.2.0 이후 배포본)

Windows PowerShell에서는 입력이 섞이지 않도록 반드시 `-Wait`를 사용합니다.

```powershell
Start-Process .\installer.exe -ArgumentList '--cli' -NoNewWindow -Wait
# 제거
Start-Process .\installer.exe -ArgumentList '--cli','--uninstall' -NoNewWindow -Wait
```

macOS/Linux에서는 `./installer --cli` 또는 `./installer --cli --uninstall`을 사용합니다. 새 빌드에서도 기존 `installer --cli` 명령은 지원하며, Windows에서는 CLI 전용 실행 파일을 별도 콘솔 창으로 엽니다.

---

## 0-1. Claude Code CLI(터미널) 사용자라면 — 그냥 부탁하세요

설치 명령을 몰라도 됩니다. Claude Code를 켜고, 이 저장소 GitHub 주소와 함께 설치를 부탁하세요.

```
https://github.com/zilhak/inno-creed
이 MCP 설치해줘
```

그 후, Claude의 안내에 따라 확장 프로그램을 설치하세요(**전 OS 필수** — [4번](#4-크레덴셜-연결--chromeedge-확장-프로그램-필수) 참고).

---

## 0-2. 전제 조건

- **MCP 클라이언트** — [Claude Code](https://claude.com/claude-code)(권장) 또는 stdio MCP를 지원하는 클라이언트. 이게 없으면 바이너리를 실행해도 아무 일도 안 합니다(입력을 기다리다 종료).
- **로그인된 브라우저** — Chrome/Edge로 `https://gw.innogrid.com` 에 로그인된 **데스크톱** 환경. (헤드리스 서버·외부인 사용 불가)
- **이노그리드 사내 계정.**
- **Chrome/Edge 확장 프로그램 — 필수입니다(전 OS).** [4번](#4-크레덴셜-연결--chromeedge-확장-프로그램-필수) 참고. 인스톨러를 쓰면 설치 과정에 포함되고, 맨 바이너리로 설치했다면 `inno-creed-extension.zip`을 따로 받아 로드합니다. 확장 없이 쿠키 DB를 직접 읽는 폴백이 있긴 하지만 **환경에 따라 되기도 안 되기도 해서 보장되지 않습니다**(Windows는 사실상 항상 실패).

지원 바이너리: **macOS(Apple Silicon)**, **Linux x86_64 / aarch64**, **Windows x86_64 / ARM64**.
> Intel 맥용 바이너리는 제공하지 않습니다(필요하면 [소스 빌드](#부록-소스-빌드)).

> **(Linux, 폴백을 쓸 때만) `libsecret-tools`가 필요합니다.** 확장 프로그램을 쓰면 필요 없습니다. Chrome 쿠키를 GNOME Keyring/KWallet에서
> 자동 복호화하려면 `secret-tool`이 있어야 합니다. 없으면 미리 설치하세요:
> `sudo apt install libsecret-tools`(Debian/Ubuntu 계열, 데스크톱 세션에서 키링이
> 잠금 해제돼 있어야 합니다). 자세한 내용은 [6. 크레덴셜이 안 잡힐 때](#6-크레덴셜이-안-잡힐-때-문제-해결) 참고.

---

## 1. 다운로드

[**릴리즈 최신본**](https://github.com/zilhak/inno-creed/releases/latest)에서 OS에 맞는 파일을 받습니다.

| OS / arch | 파일 |
|---|---|
| macOS (Apple Silicon) | `inno-creed-macos-arm64` |
| Linux x86_64 | `inno-creed-linux-x86_64` |
| Linux aarch64 | `inno-creed-linux-aarch64` |
| Windows x86_64 | `inno-creed-windows-x86_64.exe` |
| Windows ARM64 | `inno-creed-windows-aarch64.exe` |
| **확장 프로그램(전 OS 필수)** | `inno-creed-extension.zip` — 4번에서 씁니다 |

> 위는 **수동 설치용 맨 바이너리**입니다. GUI 인스톨러(`inno-creed-installer-<OS>.zip`, [0번](#0-비개발자라면--gui-인스톨러-권장))를 쓴다면 이 표의 파일은 받을 필요가 없습니다 — 인스톨러 zip이 `payload/` 안에 실행 파일과 확장 프로그램을 이미 담고 있습니다(전 OS) — 위 표의 확장 zip도 받을 필요가 없습니다.

---

## 2. OS별 설치

### macOS (Apple Silicon)

```sh
cd ~/Downloads
chmod +x inno-creed-macos-arm64
xattr -d com.apple.quarantine inno-creed-macos-arm64    # Gatekeeper 차단 해제(미서명 바이너리)
mkdir -p ~/bin && mv inno-creed-macos-arm64 ~/bin/inno-creed
```
> Gatekeeper 경고가 뜨면 위 `xattr` 명령으로 해제하거나, Finder에서 **우클릭 → 열기**를 한 번 해줍니다.

### Linux (x86_64 / aarch64)

```sh
cd ~/Downloads
chmod +x inno-creed-linux-*
mkdir -p ~/bin && mv inno-creed-linux-* ~/bin/inno-creed
```

### Windows (x86_64 / ARM64)

1. x64 PC는 `inno-creed-windows-x86_64.exe`, ARM64 PC는 `inno-creed-windows-aarch64.exe`를 받아 원하는 폴더로 옮깁니다(예: `C:\Tools\inno-creed.exe`).
2. 처음 실행 시 SmartScreen **"Windows가 PC를 보호했습니다"** 창이 뜨면 → **추가 정보 → 실행**.

---

## 3. MCP 클라이언트에 등록

**Claude Code:**

```sh
claude mcp add inno-creed --scope user -- /절대경로/inno-creed          # Windows: ...\inno-creed.exe
```

> ⚠️ **`--scope user`를 빼먹으면 등록한 그 디렉토리에서만 보입니다**(기본값이 `local`). 다른 프로젝트에서 목록에 없으면 이걸 의심하세요 — `claude mcp list`로 확인합니다.

또는 설정 JSON에 직접:

```json
{
  "mcpServers": {
    "inno-creed": {
      "command": "/절대경로/inno-creed"
    }
  }
}
```

**Claude Desktop:**

설정 파일을 직접 편집합니다. 위치는 OS·설치 방식마다 다른데, **`inno-creed doctor`가 실제 경로를 찾아서 알려줍니다**(아래 5장). 대표적으로:

| 환경 | `claude_desktop_config.json` 위치 |
|---|---|
| macOS | `~/Library/Application Support/Claude/` |
| Windows (일반 설치) | `%APPDATA%\Claude\` |
| Windows (**Microsoft Store / MSIX**) | `%LOCALAPPDATA%\Packages\Claude_<패키지ID>\LocalCache\Roaming\Claude\` |

> ⚠️ **Store 버전은 `%APPDATA%\Claude`가 아예 없습니다.** MSIX 샌드박스가 경로를 가상화하기 때문입니다. 패키지 폴더 이름(`Claude_pzs8sxrjxfjjc` 등)은 환경마다 다르니 문서 값을 그대로 믿지 말고 `doctor`가 찾은 경로를 쓰세요.

```json
{
  "mcpServers": {
    "inno-creed": {
      "command": "C:/Users/<사용자>/tools/inno-creed.exe"
    }
  }
}
```

> 💡 **Windows 경로에는 `/`를 쓰세요.** JSON에서 백슬래시는 `\\`로 두 번 써야 하고, 하나라도 틀리면 파싱이 깨져 **설정 전체가 무시됩니다**(증상은 "도구가 안 보인다"뿐이라 원인을 찾기 어렵습니다). `doctor`가 JSON 파싱 실패를 잡아줍니다.

등록 후 **클라이언트를 재시작**하면 도구가 노출됩니다. Claude Desktop은 창을 닫아도 트레이에 남으므로 **완전히 종료**해야 설정을 다시 읽습니다.

---

## 4. 크레덴셜 연결 — Chrome/Edge 확장 프로그램 (필수)

> 📸 **화면 그대로 따라가고 싶다면 → [그림으로 보는 확장 프로그램 설치 방법](https://zilhak.github.io/inno-creed/extension-install.html)**
> (다운로드부터 아마란스 로그인까지 12단계를 실제 화면 캡처로 안내합니다. 아래는 같은 절차의 요약본입니다.)

`gw.innogrid.com`의 로그인 쿠키(`BIZCUBE_AT`/`BIZCUBE_HK`)는 **세션 쿠키**라 브라우저가 켜져 있는 동안만 존재합니다. 확장 프로그램은 브라우저가 공식으로 열어준 `cookies` API로 그 값을 바로 받아 넘기는 경로이고, **모든 OS에서 이것이 정식 설치 단계입니다.**

- **Windows**는 여기에 더해 실행 중 쿠키 파일 배타 잠금과 `v20` app-bound 암호화까지 겹쳐, 쿠키 DB 직접 읽기가 구조적으로 거의 항상 실패합니다.
- **macOS·Linux**는 직접 읽기가 **되기도 합니다.** 다만 되는지가 환경에 달려 있습니다 — Chrome **"중단한 위치에서 계속하기"**가 꺼져 있으면 세션 쿠키가 디스크에 아예 없고, macOS는 키체인 접근을 거부하면, Linux는 `secret-tool`이 없거나 키링이 잠겨 있으면 그대로 실패합니다. 보장되는 경로가 아니라서 가이드는 전 OS 공통으로 확장을 필수로 안내합니다. (직접 읽기는 확장이 아직 없을 때를 받아주는 [폴백](#6-크레덴셜이-안-잡힐-때-문제-해결)으로 남아 있습니다.)
- 그리고 쿠키 DB 직접 읽기는 [DBSC](#dbsc란--쿠키-db-직접-읽기가-왜-점점-막히나) 때문에 갈수록 막히는 방향입니다. 확장 경로는 DBSC와 무관합니다.

### 인스톨러로 설치했다면 — 이미 끝났습니다

GUI 인스톨러와 `installer --cli`는 확장 파일을 설치 폴더에 깔고 native host 등록까지 마친 뒤, 브라우저에 올리는 안내 화면을 띄웁니다(전 OS 공통). 그 화면을 따라갔다면 이 절은 건너뛰고 [5번](#5-로그인--확인)으로 가세요.

### 맨 바이너리로 설치했다면

1. [릴리즈](https://github.com/zilhak/inno-creed/releases/latest)에서 **`inno-creed-extension.zip`**을 받아 **압축을 풉니다**(예: `C:\Tools\inno-creed-extension\`, macOS·Linux는 `~/inno-creed-extension` 등 보관해 둘 자리면 어디든).
   ⚠️ **푼 폴더를 지우지 마세요** — 브라우저는 압축해제 확장을 원본 폴더에서 계속 읽습니다. 지우면 확장도 사라집니다.
   (저장소를 clone했다면 `extension/` 폴더를 그대로 써도 같습니다.)
2. 확장 프로그램 관리 화면을 엽니다.
   - **Chrome**: 주소창에 `chrome://extensions` 입력, 또는 툴바 오른쪽 위 퍼즐 아이콘 → **확장 프로그램 관리**.
   - **Edge**: 주소창에 `edge://extensions` 입력, 또는 `…` 메뉴 → **확장**.
3. **개발자 모드**를 켭니다. **Chrome은 화면 우측 상단**, **Edge는 화면 좌측 하단**에 토글이 있습니다(둘 다 껐다 켜져 있는지 헷갈리기 쉬우니 위치를 참고하세요).
4. **압축해제된 확장 프로그램을 로드합니다**(Chrome은 이 이름 그대로, Edge는 **압축 풀린 파일 로드**) → 1번에서 압축을 푼 `inno-creed-extension` 폴더를 선택합니다. 목록에 "inno-creed 크레덴셜 브릿지" 카드가 뜨고 토글이 켜져 있으면 성공입니다.
5. `https://gw.innogrid.com`에 로그인돼 있으면(또는 방금 로그인하면) 자동으로 크레덴셜이 전달됩니다. 이후로도 로그인·로그아웃할 때마다 자동으로 동기화됩니다 — 매번 다시 로드할 필요 없습니다.

> **native host 등록은 신경 쓰지 않아도 됩니다.** MCP 서버가 뜰 때마다 자기 실행 경로에 맞춰 자동으로 등록합니다(바이너리를 옮겨도 다음 기동에 스스로 고쳐집니다). 수동으로 다시 걸려면 `inno-creed --install-extension-host`입니다.
>
> 등록되는 브라우저는 **Windows·Linux는 Chrome과 Edge, macOS는 Chrome만**입니다(맥은 Edge를 지원하지 않습니다). Chrome/Edge 둘 다에서 쓰려면 확장을 두 브라우저 각각에 로드하면 됩니다 — 같은 폴더를 그대로 쓰면 되고, 등록은 이미 양쪽 다 돼 있습니다.

> ⚠️ **Edge를 새로 시작하면 "개발자 모드에서 확장 사용 해제" 경고 팝업이 뜰 수 있습니다.** 여기서 **[확장 사용 해제]를 누르면 방금 설치한 확장이 꺼집니다** — 이 버튼은 누르지 말고 **[나중에]**를 누르세요. (Edge가 개발자 모드 확장 전체에 주기적으로 띄우는 일반적인 경고이지, inno-creed에 문제가 있다는 뜻이 아닙니다.)
>
> 카드에 "서비스 워커: 비활성"이라고 떠도 정상입니다 — 요청이 올 때만 깨어나는 방식이라 평소엔 비활성 상태입니다.
>
> 확장 ID는 `manifest.json`의 고정 공개키(`key`)로 결정되므로 **어디에 풀든, 몇 번을 다시 로드하든 바뀌지 않습니다**(`hpabcmnjaahhdenpdmfjlmkfjljdldbf`). 등록되는 허용 origin이 이 ID라서, 그 값이 흔들리면 브릿지가 조용히 끊깁니다 — 그래서 키를 박아두었습니다.
>
> 압축을 풀면 `manifest.json`·`background.js`·`icons/` 세 가지가 나옵니다. **셋 다 있어야 로드됩니다** — 매니페스트가 아이콘 파일을 선언하고 있어서, `icons/`를 지우면 Chrome이 로드 자체를 거부합니다.
>
> 소스에서 직접 쓰고 싶다면 저장소의 `extension/` 폴더를 그대로 로드해도 같습니다(zip은 그 폴더의 런타임 파일만 추린 것입니다).

## 5. 로그인 & 확인

1. Chrome(또는 Edge)으로 `https://gw.innogrid.com` 에 로그인합니다. 4번의 확장이 올라가 있으면 이 시점에 크레덴셜이 전달됩니다.
2. (확장 없이 폴백으로 쓰는 macOS + Chrome) 첫 실행 시 키체인 `Chrome Safe Storage` 접근 허용 프롬프트가 **1회** 뜹니다 → 허용.
3. **`inno-creed doctor`로 확인합니다.**

```sh
inno-creed doctor          # Windows: C:\...\inno-creed.exe doctor
```

크레덴셜을 어느 소스에서 잡았는지(또는 어디서 막혔는지), 익스텐션 브릿지·크레덴셜 파일·Claude Desktop 설정 파일의 실제 위치, 그리고 **gw에 1회 요청해 실제로 인증이 되는지**까지 한 화면에 보여줍니다. 토큰 값은 출력하지 않으므로 그대로 캡처해 공유해도 됩니다.

> `[익스텐션 브릿지]`의 캐시가 "없음"인데 `Chrome`에서 취득에 성공했다면, **지금은 폴백으로 돌고 있다는 뜻**입니다. 당장은 되지만 위에 적은 이유로 보장되지 않으니 4번을 마저 하세요.

> ⚠️ **도구 목록이 뜨는 것과 인증 성공은 별개입니다.** 서버는 크레덴셜이 없어도 기동하고, 도구를 부를 때 로그인 안내를 반환합니다. `doctor`의 `[실제 인증 확인]`이 ✅여야 끝난 것입니다.

정상 기동이면 클라이언트 로그에 이렇게 찍힙니다:
```
[inno-creed] 크레덴셜 취득 완료 (authToken NN자). MCP 서버 시작 (stdio)
```

---

## DBSC란 — 쿠키 DB 직접 읽기가 왜 점점 막히나

Chrome은 **Device Bound Session Credentials(DBSC)**를 2026년 4월(Chrome 146) Windows GA로 켜서, 세션 쿠키를 기기에 암호학적으로 묶어 브라우저 프로세스 밖에서 파일·COM으로 훔쳐 쓰지 못하게 막습니다(관리자 설정으로도 못 끔). Edge도 같은 Chromium이라 뒤따를 걸로 보입니다(2025년 10월 Origin Trial 종료, GA 미발표). 쿠키 DB 직접 읽기는 **지금은 운 좋게 되더라도 가까운 미래에 완전히 막힐 걸 전제로** 쓰세요. 확장 프로그램 경로(4번)는 브라우저 자신의 공식 `cookies` API를 쓰므로 DBSC와 무관합니다.

**Firefox는 DBSC를 공식적으로 도입하지 않기로 했습니다**(Mozilla `standards-positions` 저장소 `position: negative`). 다만 이건 "DBSC로는 안 막힌다"일 뿐이고, `gw.innogrid.com`은 별개 이유로 이미 Firefox에서도 파일 읽기가 안 됩니다 — `BIZCUBE_AT`/`HK`가 세션 쿠키라 **Firefox가 브라우저 실행 중엔 `cookies.sqlite`에 아예 쓰지 않는다**는 걸 실측으로 확인했습니다.

## 6. 크레덴셜이 안 잡힐 때 (문제 해결)

**먼저 `inno-creed doctor`를 실행하세요.** 어느 소스가 왜 막혔는지와 처방이 함께 나옵니다.

크레덴셜은 이 순서로 시도합니다: **`환경변수` → `익스텐션 캐시` → `Chrome` → `Edge`(Windows) → `Firefox`(비-Windows) → `크레덴셜 파일`.** 먼저 성공한 것을 쓰고 나머지는 시도하지 않습니다. `doctor` 출력에서 `·`(해당 없음)은 **대응이 필요 없는 항목**이고(Firefox 미설치 등), `❌`만 손댈 곳입니다.

### 자주 걸리는 경우

- **아직 확장 프로그램을 안 썼다면** → 4번으로 가서 설치하세요. OS를 불문하고 가장 확실합니다.
- **`BIZCUBE_AT`이 세션 쿠키라 DB에 없음** → DevTools에서 `BIZCUBE_AT` 행의 **Expires**가 `Session`이면 그 값은 **디스크에 기록되지 않습니다**. 쿠키 DB 직접 읽기로는 절대 잡히지 않으니 확장 프로그램(4번)을 쓰거나, 아래 **크레덴셜 직접 지정**을 쓰세요.
  (macOS/Linux에서 확장 없이 쓰고 있다면) Chrome **설정 → 시작 그룹 → "중단한 위치에서 계속하기"**를 켜면 세션 쿠키가 디스크에 보존되면서 자동 추출로 전환됩니다.
- **Ubuntu 등에서 Firefox가 snap/flatpak** → 프로필 경로가 표준(`~/.mozilla/firefox`)과 달라 못 찾습니다. 환경변수로 지정:
  ```sh
  # snap Firefox
  export INNO_CREED_FIREFOX_DIR=~/snap/firefox/common/.mozilla/firefox
  # flatpak Firefox
  export INNO_CREED_FIREFOX_DIR=~/.var/app/org.mozilla.firefox/.mozilla/firefox
  ```
- **Windows에서 Chrome/Edge 쿠키를 못 읽음(`os error 32` / 파일 사용 중)** → 최신 Chrome/Edge는 실행 중 쿠키 파일을 **배타적으로 잠급니다**. 완전히 종료한 뒤 다시 실행하면 파일은 읽히지만, `BIZCUBE_AT`/`HK`는 세션 쿠키라 그 시점엔 이미 사라져 있습니다(종료 전엔 잠겨서 못 읽고, 종료 후엔 쿠키가 없는 캐치-22) — **확장 프로그램(4번)이 유일하게 확실한 해법**입니다.
- **Chrome/Edge는 있는데 "복호화 실패"라고 나옴** →
  - **Linux 키링(gnome-keyring/kwallet, `v11`)**: 키링에서 키를 자동 조회하지만 **`secret-tool`이 필요**합니다. 없으면 설치 후 재시도: `sudo apt install libsecret-tools` (데스크톱 세션에서 키링이 잠금 해제돼 있어야 함).
  - **Windows Chrome/Edge app-bound(`v20`)**: 호출자 프로세스 경로를 검증하므로 inno-creed 같은 제3자 프로세스로는 **설계상 항상 거부**됩니다(버전·설정과 무관 — best-effort로 시도는 하지만 성공을 기대하지 마세요). 확장 프로그램(4번)을 쓰세요.
  - Firefox도 이 사이트에서는 세션 쿠키라 안 됩니다(위 DBSC 섹션 참고). 그래도 안 되면 아래 **크레덴셜 직접 지정**이 가장 확실합니다.

### 크레덴셜 직접 지정 (브라우저 읽기 우회)

확장 프로그램도 브라우저 읽기도 안 되는 환경(사내 정책으로 개발자 모드가 막힌 경우 등)의 **최후 수단**입니다. 브라우저 DevTools(F12) → **Application → Cookies → `https://gw.innogrid.com`** 에서 `BIZCUBE_AT`·`BIZCUBE_HK` 값을 복사한 뒤:

```sh
inno-creed auth set      # 두 값을 물어봅니다 → ~/.config/inno-creed/creds.json 에 저장(unix는 0600)
inno-creed auth clear    # 저장된 값 삭제
```

> 🔒 값은 **stdin으로만** 받습니다. 인자로 주면 셸 히스토리(PowerShell 포함)와 프로세스 목록에 세션 토큰이 남기 때문입니다. 이 두 값은 **그룹웨어 로그인 세션 그 자체**이니 설정 파일을 공유하거나 캡처해 올리지 마세요. 이미 노출했다면 로그아웃 후 재로그인해 세션을 새로 발급받으면 됩니다.
> Windows에서는 파일 권한을 좁히지 않습니다(등가 수단이 없음) — 홈 디렉토리 권한에만 의존합니다.

환경변수로도 지정할 수 있습니다. **둘 다** 설정해야 사용됩니다:

| 환경변수 | 값 |
|---|---|
| `INNO_CREED_AUTH_TOKEN` | `BIZCUBE_AT` 쿠키 값 (`%7C` 인코딩 그대로 가능) |
| `INNO_CREED_SIGN_KEY` | `BIZCUBE_HK` 쿠키 값 |

> ⚠️ **환경변수와 크레덴셜 파일을 같이 쓰지 마세요.** 환경변수가 이기므로 `auth set`으로 새 값을 저장해도 반영되지 않습니다. `doctor`가 이 조합을 경고합니다.

**크레덴셜 파일은 브라우저·익스텐션보다 아래**입니다(순서 표는 위 참고). 위에 두면 만료된 파일 하나가 멀쩡한 브라우저 세션을 영영 가리기 때문입니다 — 환경변수가 지금 가진 성질이 그렇습니다.

### 세션이 만료되면

도구 호출이 로그인 안내를 반환합니다. 복구 방법은 어느 소스를 쓰느냐에 따라 다릅니다.

| 소스 | 복구 | 재시작 |
|---|---|---|
| 익스텐션 브릿지 | 브라우저로 다시 로그인 | **불필요** — 익스텐션이 즉시 새 값을 밀어넣습니다 |
| 브라우저(Chrome/Edge/Firefox) | 브라우저로 다시 로그인 | **불필요** — 401을 만나면 다음 호출에서 쿠키를 다시 읽습니다 |
| 크레덴셜 파일 | `inno-creed auth set`으로 새 값 저장 | **불필요** — 같은 경로로 파일을 다시 읽습니다 |
| 환경변수 | MCP 설정의 `env` 값 교체 | **필요** — 재취득해도 같은 값이 돌아오므로 클라이언트를 재시작해야 합니다 |

환경변수만 재시작이 필요합니다. 그게 번거로우면 `auth set`(크레덴셜 파일) 쪽을 쓰세요.

### 경로 오버라이드 환경변수

| 환경변수 | 용도 |
|---|---|
| `INNO_CREED_EXTENSION_CACHE` | 확장 프로그램 캐시 파일 경로(직접) — 기본값은 OS별 표준 로컬 데이터 디렉토리 |
| `INNO_CREED_FIREFOX_COOKIES` | Firefox `cookies.sqlite` 파일 경로(직접) |
| `INNO_CREED_FIREFOX_DIR` | Firefox 프로필 **디렉토리**(스캔) |
| `INNO_CREED_CHROME_COOKIES` | Chrome `Cookies` DB 파일 경로(직접) |
| `INNO_CREED_CHROME_USER_DATA` | Chrome `User Data` 루트 |
| `INNO_CREED_EDGE_COOKIES` | Edge `Cookies` DB 파일 경로(직접, Windows 전용) |
| `INNO_CREED_EDGE_USER_DATA` | Edge `User Data` 루트(Windows 전용) |
| `INNO_CREED_AUTH_TOKEN` | `BIZCUBE_AT` 값 직접 지정(브라우저 우회) |
| `INNO_CREED_SIGN_KEY` | `BIZCUBE_HK` 값 직접 지정(브라우저 우회) |

### ⚠️ MCP 클라이언트로 실행할 땐 `env`에 넣어야 합니다

MCP 클라이언트(Claude Code·Claude Desktop)가 서버를 띄우면 셸의 `export`가 전달되지 않습니다. 등록 설정의 `env` 블록에 넣으세요:

```json
{
  "mcpServers": {
    "inno-creed": {
      "command": "/절대경로/inno-creed",
      "env": {
        "INNO_CREED_FIREFOX_DIR": "/home/you/snap/firefox/common/.mozilla/firefox"
      }
    }
  }
}
```

---

## 부록: 소스 빌드

프리빌트가 없는 환경(예: Intel 맥)이나 직접 빌드하려면:

```sh
git clone https://github.com/zilhak/inno-creed && cd inno-creed
cargo build --release        # → target/release/inno-creed (Windows는 inno-creed.exe)
```

**Rust 1.96+**(edition 2024, 번들 `libsqlite3-sys`가 최신 toolchain 요구)와 **C 컴파일러**(rusqlite 번들 SQLite 컴파일용)가 필요합니다.

`extension/`의 빌드 결과(`background.js`)는 저장소에 커밋돼 있어 4번 절차에는 별도 빌드가 필요 없습니다. 소스(`extension/src/background.ts`)를 고쳤다면 [Bun](https://bun.sh)으로 다시 빌드하세요:

```sh
cd extension
bun install
bun run build   # → background.js 갱신
```

---

전체 기능·안전 규약·동작 방식은 [README](../README.md)를 참고하세요.
