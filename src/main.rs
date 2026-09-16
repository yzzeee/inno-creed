//! inno-creed MCP 서버 (stdio).
//! 크레덴셜(익스텐션 브릿지/브라우저 쿠키/크레덴셜 파일) 취득 → gw API 도구를 rmcp로 노출.
//!
//! 인자 없이 실행하면 MCP 서버로 뜬다. `doctor`/`auth`는 설치를 돕는 보조 명령이다.

use anyhow::{bail, Result};
use inno_creed::{client::GwClient, creds, doctor, mcp::Amaranth, native_host};
use rmcp::{transport::stdio, ServiceExt};
use std::io::{IsTerminal, Write};

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // ⚠️ **인자 처리는 전부 서버가 뜨기 전에 끝낸다.** MCP stdio는 stdout이 JSON-RPC 채널이라,
    // 서버 기동 후에 무언가 찍으면 프로토콜이 깨진다.

    // --version/-V: 인자 파싱기가 따로 없어 설치본 버전을 확인할 방법이 없었다.
    // 크레덴셜 취득(브라우저 쿠키 읽기) 전에 먼저 처리해 부작용 없이 즉시 종료한다.
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("inno-creed {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    if args.iter().any(|a| a == "--help" || a == "-h" || a == "help") {
        print_help();
        return Ok(());
    }

    // --native-host: 브라우저 익스텐션이 Native Messaging으로 스폰하는 1회성 모드.
    // MCP 서버로 안 뜨고 메시지 하나 처리한 뒤 즉시 종료한다(자세한 이유는 native_host.rs).
    //
    // ⚠️ Chrome/Edge는 native host를 실행할 때 우리가 지정한 플래그를 안 붙이고, 대신
    // 확장앱 origin(`chrome-extension://<id>/`)을 인자로 넘긴다(Native Messaging 스펙
    // 그대로 — "path"가 곧 실행 커맨드라 우리 쪽 커스텀 인자를 끼워넣을 방법이 없다).
    // 그래서 `--native-host`뿐 아니라 origin 패턴도 같이 감지한다. `--native-host`는
    // 수동 테스트용으로 남겨둔다(브라우저는 안 쓰지만 CLI로 직접 찔러볼 때 편함).
    if args
        .iter()
        .any(|a| a == "--native-host" || a.starts_with("chrome-extension://") || a.starts_with("moz-extension://"))
    {
        return native_host::run();
    }

    // --install-extension-host [확장ID]: Chrome/Edge에 native messaging host를 등록하는
    // 1회성 설정 명령. 확장 ID 생략 시 이 저장소의 `extension/manifest.json`에 고정된
    // 기본값을 쓴다(직접 빌드한 익스텐션을 다른 키로 서명했다면 인자로 넘기면 됨).
    if args.iter().any(|a| a == "--install-extension-host") {
        let id = args
            .iter()
            .position(|a| a == "--install-extension-host")
            .and_then(|i| args.get(i + 1))
            .map(String::as_str)
            .unwrap_or(native_host::DEFAULT_EXTENSION_ID);
        native_host::install(id)?;
        return Ok(());
    }

    // 보조 서브커맨드. **native host 감지보다 뒤에 둔다** — 브라우저는 native host를 스폰할 때
    // 확장 origin과 `--parent-window` 같은 인자를 제멋대로 붙이므로, 그 경로가 먼저 빠져나가야
    // 아래 "알 수 없는 인자" 검사에 걸리지 않는다.
    match args.first().map(String::as_str) {
        Some("doctor") => std::process::exit(doctor::run().await),
        Some("auth") => return auth_cmd(args.get(1).map(String::as_str)),
        Some("extension") => return extension_cmd(args.get(1).map(String::as_str)),
        // 오타를 조용히 삼키고 서버로 뜨면, 사용자는 "명령이 먹통"으로만 본다.
        Some(other) => bail!("알 수 없는 인자: {other}\n`inno-creed --help`로 사용법을 확인하세요."),
        None => {}
    }

    // 확장 브릿지의 native host 등록은 **서버가 뜰 때마다 스스로 맞춘다**(멱등).
    // 사람이 기억해야 하는 별도 단계로 두면 빠지고, 빠지면 브릿지가 조용히 안 붙는다.
    // 실패해도 서버는 뜬다 — 쿠키 직접 읽기 폴백이 남아 있고, 원인은 `doctor`가 보여준다.
    if let Err(e) = native_host::ensure_installed() {
        eprintln!("[inno-creed] ⚠️ 확장 브릿지 native host 등록 실패(계속 진행): {e:#}");
    }

    // 크리덴셜 취득 실패해도 서버는 뜬다(비치명적). 실패 시 도구 호출 시점에 로그인 안내를
    // tool 응답으로 반환한다(사용자가 채팅에서 볼 수 있게). 성공하면 캐시를 seed.
    let initial = match creds::from_browser() {
        Ok(c) => {
            eprintln!(
                "[inno-creed] 크레덴셜 취득 완료 (authToken {}자). MCP 서버 시작 (stdio)",
                c.auth_token.len()
            );
            Some(c)
        }
        Err(e) => {
            eprintln!(
                "[inno-creed] ⚠️ 크레덴셜 미취득 — 서버는 시작하되 도구 호출 시 로그인 안내를 반환합니다.\n{e}"
            );
            None
        }
    };

    // 세션 정보(compSeq/deptSeq/근태 empCd 등)는 첫 도구 호출 시 gw050A02로 lazy 취득 후
    // 10분 TTL 캐시된다(ensure_session). 시작 시 선취득하지 않는다.
    let client = GwClient::new(initial);

    let service = Amaranth::new(client).serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

fn print_help() {
    println!(
        "inno-creed {} — 아마란스(gw.innogrid.com) MCP 서버\n\
         \n\
         사용법:\n\
         \x20 inno-creed                            MCP 서버로 기동(stdio). MCP 클라이언트가 실행합니다.\n\
         \x20 inno-creed doctor                     크레덴셜·설정이 어디서 막혔는지 진단합니다.\n\
         \x20 inno-creed auth set                   BIZCUBE_AT/BIZCUBE_HK를 크레덴셜 파일에 저장합니다.\n\
         \x20 inno-creed auth clear                 저장된 크레덴셜 파일을 지웁니다.\n\
         \x20 inno-creed extension [폴더]           확장 프로그램 파일을 꺼내놓고 native host를 등록합니다.\n\
         \x20 inno-creed --install-extension-host   Chrome/Edge 확장용 native host를 다시 등록합니다(기동 시 자동).\n\
         \x20 inno-creed --version                  버전을 출력합니다.\n\
         \n\
         설치가 막히면 먼저 `inno-creed doctor`를 실행하세요.",
        env!("CARGO_PKG_VERSION")
    );
}

/// 확장 설치를 한 명령으로 끝낸다 — 파일을 꺼내놓고, native host를 등록하고, 사람이 해야 할
/// 마지막 한 단계(브라우저에서 그 폴더를 로드)를 경로와 함께 알려준다.
///
/// 브라우저에 로드하는 것만은 자동화할 수 없다(확장 설치는 사용자 제스처를 요구하는 브라우저
/// 정책이다). 그 한 단계만 남기고 나머지는 전부 여기서 끝낸다.
fn extension_cmd(dest: Option<&str>) -> Result<()> {
    let dir = native_host::unpack_extension(dest.map(std::path::PathBuf::from))?;
    native_host::install(native_host::DEFAULT_EXTENSION_ID)?;
    println!(
        "\n확장 프로그램 파일을 꺼내놨습니다:\n  {}\n\n\
         남은 한 단계 — 브라우저에서 이 폴더를 로드하세요(자동으로 못 하는 부분입니다):\n\
         \x20 1. chrome://extensions (Edge는 edge://extensions) 를 엽니다.\n\
         \x20 2. 개발자 모드를 켭니다(Chrome은 우측 상단, Edge는 좌측 하단).\n\
         \x20 3. \"압축해제된 확장 프로그램 로드\"(Edge는 \"압축 풀린 파일 로드\")로 위 폴더를 선택합니다.\n\
         \x20 4. https://gw.innogrid.com 에 로그인하면 즉시 연결됩니다.\n\n\
         ⚠️ 이 폴더를 지우면 확장도 사라집니다 — 브라우저가 원본 폴더를 계속 읽습니다.\n\
         확인: inno-creed doctor",
        dir.display()
    );
    Ok(())
}

fn auth_cmd(sub: Option<&str>) -> Result<()> {
    match sub {
        Some("set") => auth_set(),
        Some("clear") => {
            if creds::clear_creds_file()? {
                println!("크레덴셜 파일을 지웠습니다.");
            } else {
                println!("지울 크레덴셜 파일이 없습니다.");
            }
            Ok(())
        }
        _ => bail!("사용법: inno-creed auth set | inno-creed auth clear"),
    }
}

/// 쿠키 값을 받아 크레덴셜 파일에 저장한다.
///
/// **stdin으로만 받는다** — 인자로 받으면 셸 히스토리(PowerShell 포함)와 프로세스 목록에
/// 세션 토큰이 그대로 남는다. 이 두 값은 그룹웨어 로그인 세션 그 자체다.
///
/// 화면에는 그대로 보인다(가림 처리는 의존성이 필요해 하지 않는다) — 어차피 DevTools에서
/// 복사해 오는 값이라 화면에 이미 떠 있었다.
fn auth_set() -> Result<()> {
    let piped = !std::io::stdin().is_terminal();
    if !piped {
        println!(
            "브라우저에서 gw.innogrid.com 접속 → F12 → Application → Cookies →\n\
             BIZCUBE_AT / BIZCUBE_HK 의 Value를 복사해 붙여넣으세요.\n"
        );
    }
    let auth_token = prompt("BIZCUBE_AT  : ", piped)?;
    let sign_key = prompt("BIZCUBE_HK  : ", piped)?;
    if auth_token.is_empty() || sign_key.is_empty() {
        bail!("두 값이 모두 필요합니다 — 하나만으로는 서명이 만들어지지 않습니다.");
    }
    let path = creds::save_creds_file(&auth_token, &sign_key)?;
    println!("\n저장했습니다: {}", path.display());
    #[cfg(not(unix))]
    println!("⚠️ Windows에서는 파일 권한을 좁히지 않습니다 — 홈 디렉토리 권한에만 의존합니다.");
    println!(
        "이 파일은 **가장 마지막** 소스입니다({}).\n\
         MCP 설정에 INNO_CREED_AUTH_TOKEN/INNO_CREED_SIGN_KEY가 남아 있으면 그쪽이 이기니 지우세요.\n\
         확인: inno-creed doctor",
        creds::source_names().join(" → ")
    );
    Ok(())
}

fn prompt(label: &str, piped: bool) -> Result<String> {
    if !piped {
        print!("{label}");
        std::io::stdout().flush()?;
    }
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}
