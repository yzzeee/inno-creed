//! `inno-creed doctor` — 설치가 막혔을 때 한 화면으로 원인을 보여준다.
//!
//! **왜 있나**: 크레덴셜 취득이 실패하면 그 사실은 stderr로만 나가고, MCP 클라이언트에서
//! 그걸 보려면 로그 파일을 뒤져야 한다. 실제 설치 사례에서 사용자가 네 번 막혔는데
//! (Chrome 잠금 → Firefox 잡음 → 설정 파일 경로 → 세션 쿠키) 매번 필요했던 것이 정확히
//! 이 한 화면이었다. 익스텐션 브릿지가 전 OS 정식 경로가 되면서 막힐 수 있는 지점은
//! 오히려 늘었다(익스텐션 로드 → 네이티브 호스트 등록 → 캐시 생성).
//!
//! ⚠️ **진단을 여기서 다시 구현하지 않는다.** `creds::diagnose()`가 만든 결과를 그리기만 한다 —
//! 따로 구현하면 "doctor는 OK인데 서버는 실패"처럼 서로 어긋나는 순간이 생긴다.
//!
//! ⚠️ **토큰 값은 절대 찍지 않는다.** 길이만 보여준다. 사용자가 이 출력을 그대로 캡처해
//! 붙여넣는 것이 이 명령의 정상 사용법이기 때문이다.

use crate::creds::{self, Outcome};
use std::path::PathBuf;

/// 진단을 실행하고 출력한다. 크레덴셜을 못 잡으면 1, 잡으면 0.
///
/// **stdout으로 찍고 즉시 끝나야 한다** — MCP stdio는 stdout이 JSON-RPC 채널이라,
/// 서버가 뜬 뒤에 무언가 찍으면 프로토콜이 깨진다. 그래서 `main`에서 `--version`과
/// 같은 자리(서버 기동 전)에 놓는다.
pub async fn run() -> i32 {
    println!(
        "inno-creed {} ({} / {})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    );

    let d = creds::diagnose();
    let creds_missing = d.creds.is_none();

    // 순서는 **`creds`가 정본**이다 — 여기에 손으로 적으면 코드와 어긋난다.
    let all_sources = creds::source_names();
    println!(
        "\n[크레덴셜] 소스를 위에서부터 시도합니다 — {}",
        all_sources.join(" → ")
    );
    for r in &d.reports {
        match &r.outcome {
            Outcome::Ok => println!("  ✅ {} — 취득 성공", r.source),
            Outcome::Absent(m) => println!("  ·  {} — 해당 없음: {m}", r.source),
            Outcome::Failed(m) => println!("  ❌ {} — {m}", r.source),
        }
    }
    // 성공하면 그 아래 소스는 시도하지 않는다(서버의 실제 동작과 같다). 그 사실을 밝힌다.
    if d.creds.is_some() && d.reports.len() < all_sources.len() {
        println!("  ·  (이후 소스는 시도하지 않음 — 위에서 이미 성공)");
    }

    match &d.creds {
        Some(c) => {
            println!(
                "\n  → 사용 가능. authToken {}자 / signKey {}자. (값은 표시하지 않습니다)",
                c.auth_token.len(),
                c.sign_key.len()
            );
            warn_if_direct_read(&d.reports);
        }
        None => {
            println!("\n  → 크레덴셜 없음. 서버는 기동하지만 도구 호출은 로그인 안내를 반환합니다.");
            print_next_steps();
        }
    }

    // 환경변수와 파일을 같이 쓰면 환경변수가 이긴다 — 파일을 새로 저장해도 안 먹는 함정이라
    // 미리 경고한다(`auth set`을 해놓고 왜 옛 토큰이냐는 상황).
    //
    // **환경변수가 실제로 이겼을 때만** 경고한다. 한쪽만 설정돼 실패한 경우까지 경고하면
    // "환경변수가 이깁니다"가 거짓말이 되고, 바로 위에 찍힌 ❌와도 어긋난다.
    let env_won = d
        .reports
        .iter()
        .any(|r| r.source == "환경변수" && r.outcome == Outcome::Ok);
    if env_won && creds::creds_file_path().is_some_and(|p| p.exists()) {
        println!(
            "  ⚠️ 환경변수와 크레덴셜 파일이 둘 다 있습니다 — 환경변수가 이깁니다. \
             파일을 쓰려면 MCP 설정에서 INNO_CREED_AUTH_TOKEN/INNO_CREED_SIGN_KEY를 지우세요."
        );
    }

    report_extension_bridge();

    println!("\n[크레덴셜 파일] 브라우저·익스텐션이 모두 안 될 때의 최후 수단");
    match creds::creds_file_path() {
        Some(p) if p.exists() => println!("  {} (있음)", p.display()),
        Some(p) => println!("  {} (없음 — `inno-creed auth set`으로 저장)", p.display()),
        None => println!("  경로를 정할 수 없음(HOME·XDG_CONFIG_HOME 둘 다 없음)"),
    }

    println!("\n[Claude Desktop 설정 파일]");
    report_desktop_configs();

    println!(
        "\n[Claude Code] `claude mcp list`로 확인하세요. 목록에 없으면 등록 시 --scope user를 빼먹었을 수 있습니다."
    );

    // 크레덴셜을 **가졌다는 것과 그게 통한다는 것은 다르다.** 만료된 토큰도 형태는 멀쩡하다.
    // 도구 목록이 뜨는 것만 보고 인증까지 됐다고 오해하는 것이 실제 설치에서 나온 함정이라,
    // 여기서 실제로 한 번 왕복해 확인한다. (크레덴셜이 있을 때만 — 네트워크를 쓴다.)
    if !creds_missing {
        println!("\n[실제 인증 확인] gw.innogrid.com에 1회 요청합니다");
        match verify(d.creds).await {
            Ok(()) => println!("  ✅ 인증 성공 — 도구를 바로 쓸 수 있습니다."),
            Err(e) => {
                println!("  ❌ 인증 실패: {e:#}");
                return 1;
            }
        }
    }

    i32::from(creds_missing)
}

/// 익스텐션 브릿지의 두 조각을 따로 보여준다 — **캐시 파일과 네이티브 호스트 등록은 서로
/// 다른 단계라 따로 실패한다.** 등록만 하고 확장 프로그램을 로드하지 않으면 캐시가 안 생기고,
/// 확장만 로드하고 등록을 안 하면 확장 쪽에서 조용히 연결이 끊긴다. 한 줄로 뭉뚱그리면
/// 둘 중 어디서 멈췄는지 알 수 없다.
fn report_extension_bridge() {
    println!("\n[익스텐션 브릿지] 확장 → native host → 캐시 파일 — **전 OS 공통 정식 경로**");
    match creds::extension_cache_path() {
        Ok(p) if p.exists() => println!("  캐시: {} (있음)", p.display()),
        Ok(p) => println!(
            "  캐시: {} (없음 — 확장을 아직 브라우저에 올리지 않았거나, 올린 뒤 gw.innogrid.com에 로그인하지 않았습니다)",
            p.display()
        ),
        Err(e) => println!("  캐시: 경로를 정할 수 없음 ({e:#})"),
    }
    let targets = crate::native_host::manifest_targets();
    if targets.is_empty() {
        println!("  native host 등록: 놓을 자리를 정할 수 없음(HOME·LOCALAPPDATA 둘 다 없음)");
        return;
    }
    // 등록은 MCP 서버가 뜰 때마다 스스로 맞춘다(`native_host::ensure_installed`). 그래서 여기
    // "없음"은 **서버가 한 번도 안 떴거나 그 쓰기가 실패했다**는 뜻이지, 사용자가 잊은 게 아니다.
    for (browser, p) in targets {
        let state = if p.exists() {
            "있음"
        } else {
            "없음 — 그 브라우저를 안 쓰면 문제없습니다. 쓰는데도 없으면 `inno-creed --install-extension-host`의 경고를 보세요"
        };
        println!("  native host 등록({browser}): {} ({state})", p.display());
    }
    // 등록 자리는 브라우저가 스스로 훑는 곳이라, 목록에 없는 브라우저는 매니페스트를 못 본다.
    // 그 경우 확장은 올라가지만 브릿지는 **에러 없이 조용히** 안 붙는다 — 미리 말해 준다.
    #[cfg(not(target_os = "windows"))]
    println!(
        "  (위 목록에 없는 브라우저 — Chromium·Brave·Vivaldi, snap/flatpak으로 깐 것 등 — 에는 등록되지 않습니다. \
         그 브라우저를 쓴다면 브릿지가 조용히 안 붙습니다.)"
    );
}

/// 브라우저 쿠키를 **직접 읽어** 성공한 경우의 경고.
///
/// 이 경로는 되기도 하고 안 되기도 한다 — 그런데 "지금 됐다"는 화면을 보면 사용자는 확장을
/// 건너뛴다. 실제로 그렇게 미룬 사용자가 나중에 Claude Desktop에서 통째로 막혔다(호스트가
/// 띄우는 프로세스는 권한이 달라 쿠키 DB를 못 연다). 그래서 성공했을 때도 말해 준다.
fn warn_if_direct_read(reports: &[creds::SourceReport]) {
    let Some(ok) = reports.iter().find(|r| r.outcome == Outcome::Ok) else {
        return;
    };
    if !matches!(ok.source, "Chrome" | "Edge" | "Firefox") {
        return;
    }
    println!(
        "  ⚠️ 지금은 브라우저 쿠키를 직접 읽어 성공했지만, 이 경로는 **보장되지 않습니다** — \
         Claude Desktop이 이 서버를 띄우면 권한이 달라 실패할 수 있고(macOS에서 실측), \
         세션 쿠키가 디스크에 없거나 키체인·키링이 잠기면 그대로 막힙니다."
    );
    println!("     아래 [익스텐션 브릿지]를 지금 올려두세요 — 그게 정식 경로입니다.");
}

/// 크레덴셜이 하나도 없을 때 **다음에 할 일 하나**만 보여준다. 고를 수 있는 방법을 나열하면
/// 사용자는 가장 쉬워 보이는 것(브라우저로 로그인만 다시 해보기)을 고르고 같은 자리를 맴돈다.
fn print_next_steps() {
    println!("\n  ▶ 지금 할 일 — 확장 프로그램을 브라우저에 올리세요 (전 OS 공통 정식 경로)");
    println!("     1. chrome://extensions (Edge는 edge://extensions)를 열고 개발자 모드를 켭니다.");
    match extension_folder_hint() {
        Some(dir) => println!(
            "     2. [압축해제된 확장 프로그램 로드]로 이 폴더를 고릅니다: {}",
            dir.display()
        ),
        None => println!(
            "     2. [압축해제된 확장 프로그램 로드]로 확장 폴더를 고릅니다 — 인스톨러로 설치했다면 설치 폴더 안의 \
             extension/, 맨 바이너리로 설치했다면 릴리즈의 inno-creed-extension.zip을 받아 푼 폴더입니다."
        ),
    }
    println!("     3. https://gw.innogrid.com 에 로그인합니다 — 로그인 즉시 자동으로 전달됩니다.");
    println!("     (native host 등록은 서버가 뜰 때마다 스스로 맞춥니다. 사람이 할 일은 위 세 가지뿐입니다.)");
}

/// 인스톨러는 확장 파일을 **본체 옆**에 둔다. 그 자리를 알면 사용자가 폴더를 찾아 헤매지 않는다.
fn extension_folder_hint() -> Option<PathBuf> {
    let dir = std::env::current_exe().ok()?.parent()?.join("extension");
    dir.is_dir().then_some(dir)
}

/// 세션 조회 1회로 크레덴셜이 실제로 통하는지 본다. 부작용 없는 조회다.
async fn verify(creds: Option<crate::creds::Creds>) -> anyhow::Result<()> {
    crate::client::GwClient::new(creds).ensure_session().await
}

/// 설정 파일 후보를 **런타임에 찾아서** 보고한다.
///
/// 경로를 문서에 박아두지 않는 이유: MS Store(MSIX)판은 샌드박스 경로 가상화 때문에
/// `%APPDATA%\Claude`가 아예 없고, 실제 경로에 든 패키지 이름(`Claude_pzs8sxrjxfjjc`)은
/// 환경에 따라 다르다. 문서에 적으면 틀릴 수 있는 값이라 여기서 훑는다.
fn report_desktop_configs() {
    let found: Vec<PathBuf> = config_kit::desktop_config_candidates()
        .into_iter()
        .filter(|p| p.exists())
        .collect();
    if found.is_empty() {
        println!("  찾지 못했습니다. Claude Desktop을 한 번 실행하면 생성됩니다.");
        for p in config_kit::desktop_config_candidates() {
            println!("    (확인한 경로) {}", p.display());
        }
        return;
    }
    for p in found {
        println!("  {}", p.display());
        match std::fs::read_to_string(&p) {
            Err(e) => println!("    ❌ 읽기 실패: {e}"),
            Ok(raw) => match serde_json::from_str::<serde_json::Value>(&raw) {
                // JSON이 깨지면 설정 **전체**가 무시된다(경로의 백슬래시를 한 번만 쓴 경우가 흔하다).
                // 그 상태에서는 "등록했는데 도구가 안 보인다"로만 보여서 원인을 찾기 어렵다.
                Err(e) => println!(
                    "    ❌ JSON 파싱 실패: {e} — 이 상태면 설정 전체가 무시됩니다. \
                     경로의 백슬래시는 `\\\\`로 두 번 쓰거나 `/`를 쓰세요."
                ),
                Ok(v) => report_entry(&v),
            },
        }
        // MCP가 한 번이라도 기동했는지는 로그 폴더 유무로 알 수 있다.
        if let Some(dir) = p.parent() {
            let logs = dir.join("logs");
            println!(
                "    로그: {} ({})",
                logs.display(),
                if logs.exists() { "있음" } else { "없음 — MCP가 아직 기동하지 않았습니다" }
            );
        }
    }
}

/// 설정 안의 `inno-creed` 항목을 확인한다. `command`가 실재하는 파일인지까지 본다 —
/// 경로 오타는 "도구가 안 보인다"로만 나타나서 스스로 드러나지 않는다.
fn report_entry(v: &serde_json::Value) {
    let Some(entry) = v.get("mcpServers").and_then(|m| m.get("inno-creed")) else {
        println!("    ⚠️ mcpServers에 `inno-creed` 항목이 없습니다.");
        return;
    };
    match entry.get("command").and_then(|c| c.as_str()) {
        Some(cmd) => {
            let exists = std::path::Path::new(cmd).exists();
            println!(
                "    ✅ 등록됨: command = {cmd} ({})",
                if exists { "실재" } else { "❌ 그 경로에 파일이 없습니다" }
            );
        }
        None => println!("    ⚠️ `inno-creed` 항목에 command가 없습니다."),
    }
    if let Some(env) = entry.get("env").and_then(|e| e.as_object()) {
        let keys: Vec<&str> = env.keys().map(String::as_str).collect();
        if !keys.is_empty() {
            println!("    env: {} (값은 표시하지 않습니다)", keys.join(", "));
        }
    }
}
