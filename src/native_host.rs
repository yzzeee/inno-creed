//! 브라우저 익스텐션(Chrome/Edge, `extension/`)이 Native Messaging으로 보내주는 쿠키를
//! 받아 로컬 캐시 파일에 저장하는 쪽. `creds::from_extension_cache`가 그 파일을 읽는다.
//! **전 OS에서 이 경로가 정식이다** — 등록 자리는 `manifest_targets`가 OS별로 정하고
//! (Windows는 레지스트리+매니페스트 1개, unix는 브라우저별 디렉토리. macOS는 Chrome만),
//! 기동마다 `ensure_installed`가 다시 맞춘다.
//!
//! **왜 존재하는가**: 쿠키 DB를 직접 읽는 길은 어느 OS에서도 보장되지 않는다. Windows
//! Chrome/Edge의 app-bound(v20) 암호화는 호출자가 브라우저 자신인지 경로로 검증해
//! 제3자 프로세스를 **설계상 항상 거부**하고(`creds.rs` 모듈 문서 참고), macOS·Linux도
//! 세션 쿠키가 디스크에 없거나 키체인·키링 접근이 막히면 그대로 실패한다.
//! 반면 익스텐션은 브라우저가 공식으로 열어준
//! `chrome.cookies` API로 평문 값을 바로 받을 수 있다 — 이 값을 로컬 프로세스로 옮기는
//! 유일한 non-소켓 경로가 Native Messaging이다(익스텐션은 리스닝 소켓을 못 연다. 브라우저가
//! 이 실행파일을 스폰해서 stdio를 파이프해준다).
//!
//! **프로토콜**: 4바이트 네이티브 바이트오더 길이 + 그만큼의 UTF-8 JSON, 양방향 동일
//! (Chrome/Edge Native Messaging 스펙 그대로).
//!
//! **호출 방식**: 브라우저가 `sendNativeMessage` 한 번마다 이 프로세스를 새로 스폰하고,
//! 응답 메시지 하나를 받으면 종료시킨다 — 그래서 이 프로세스는 메시지 하나 처리하고 바로
//! 끝나는 1회성이다. MCP 서버 본체(장수 프로세스)와는 완전히 별개 실행이고, 캐시 파일
//! 하나로만 이어진다(소켓 없음).

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::PathBuf;

use crate::creds::extension_cache_path;

const HOST_NAME: &str = "com.innogrid.inno_creed";

/// `extension/manifest.json`의 `"key"`(고정 공개키)로부터 결정되는 확장 프로그램 ID.
/// 이 값이 고정돼 있어야 익스텐션을 다시 로드해도(경로가 바뀌어도) native host의
/// `allowed_origins`가 계속 맞는다 — `"key"` 없이 unpacked로 로드하면 로드 경로에 따라
/// ID가 매번 달라져 이 등록이 깨진다.
pub const DEFAULT_EXTENSION_ID: &str = "hpabcmnjaahhdenpdmfjlmkfjljdldbf";

fn read_message(stdin: &mut impl Read) -> Result<Value> {
    let mut len_buf = [0u8; 4];
    stdin
        .read_exact(&mut len_buf)
        .context("stdin에서 길이 프리픽스 읽기 실패(브라우저가 아닌 다른 곳에서 실행한 건 아닌지)")?;
    let len = u32::from_ne_bytes(len_buf) as usize;
    let mut buf = vec![0u8; len];
    stdin.read_exact(&mut buf).context("stdin 본문 읽기 실패")?;
    serde_json::from_slice(&buf).context("네이티브 메시지 JSON 파싱 실패")
}

fn write_message(stdout: &mut impl Write, value: &Value) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    stdout.write_all(&(bytes.len() as u32).to_ne_bytes())?;
    stdout.write_all(&bytes)?;
    stdout.flush()?;
    Ok(())
}

/// 메시지 하나 처리하고 종료. stdout에는 프로토콜 메시지 외 어떤 것도 써서는 안 된다
/// (브라우저가 stdout 전체를 프로토콜로 해석함) — 진단은 전부 stderr로.
pub fn run() -> Result<()> {
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();

    let msg = read_message(&mut stdin)?;

    if msg.get("clear").and_then(Value::as_bool) == Some(true) {
        let result = clear_cache();
        return write_message(&mut stdout, &ack(result));
    }

    let auth_token = msg.get("authToken").and_then(Value::as_str);
    let sign_key = msg.get("signKey").and_then(Value::as_str);
    let result = match (auth_token, sign_key) {
        (Some(at), Some(hk)) if !at.is_empty() && !hk.is_empty() => write_cache(at, hk),
        _ => Err(anyhow::anyhow!("authToken/signKey 누락 또는 빈 값")),
    };
    write_message(&mut stdout, &ack(result))
}

fn ack(result: Result<()>) -> Value {
    match result {
        Ok(()) => json!({ "ok": true }),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    }
}

fn write_cache(auth_token: &str, sign_key: &str) -> Result<()> {
    let path = extension_cache_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let captured_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let payload = json!({
        "authToken": auth_token,
        "signKey": sign_key,
        "capturedAtMs": captured_at_ms,
    });
    std::fs::write(&path, serde_json::to_vec_pretty(&payload)?)
        .with_context(|| format!("캐시 파일 쓰기 실패: {}", path.display()))
}

fn clear_cache() -> Result<()> {
    let path = extension_cache_path()?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("캐시 파일 삭제 실패: {}", path.display())),
    }
}

/// native host 매니페스트를 놓을 자리와, 그 자리를 읽는 브라우저 이름(진단 문구용).
/// **`install`(쓰기)과 `doctor`(확인)가 공유한다** — 경로를 두 군데 적으면 "등록했는데
/// doctor는 없다고 한다"가 생긴다.
///
/// **왜 목록인가**: Windows는 매니페스트를 한 곳에 두고 레지스트리 항목이 그것을 가리키므로
/// 파일은 하나다. unix에는 그 간접층이 없어 **브라우저가 자기 디렉토리를 직접 훑는다** — 같은
/// 내용을 브라우저 수만큼 놓는 것이 곧 등록이다. 아직 설치되지 않은 브라우저 자리에도 미리
/// 써둔다(Windows가 두 하이브를 무조건 등록하는 것과 같다 — 나중에 깔아도 그대로 동작한다).
#[cfg(target_os = "windows")]
pub fn manifest_targets() -> Vec<(&'static str, PathBuf)> {
    let Ok(local) = std::env::var("LOCALAPPDATA") else {
        return Vec::new();
    };
    vec![(
        "Chrome·Edge 공용",
        PathBuf::from(format!("{local}\\inno-creed")).join(format!("{HOST_NAME}.json")),
    )]
}

#[cfg(not(target_os = "windows"))]
pub fn manifest_targets() -> Vec<(&'static str, PathBuf)> {
    let Ok(home) = std::env::var("HOME") else {
        return Vec::new();
    };
    let home = PathBuf::from(home);
    // macOS는 **Chrome만** 본다. Edge는 지원 대상이 아니다 — 깔지도 않은 맥에
    // `~/Library/Application Support/Microsoft Edge/`가 root 소유로 남아 있어(다른
    // 설치 프로그램이 만들어 둔 잔재) 쓰기가 거부되는 사례를 실제로 만났다. 쓰지도 않을
    // 브라우저 자리 때문에 매 기동 경고가 뜨는 것이 얻는 것보다 나쁘다.
    #[cfg(target_os = "macos")]
    let dirs = [("Chrome", "Library/Application Support/Google/Chrome")];
    #[cfg(not(target_os = "macos"))]
    let dirs = [
        ("Chrome", ".config/google-chrome"),
        ("Edge", ".config/microsoft-edge"),
    ];
    dirs.iter()
        .map(|(browser, dir)| {
            (
                *browser,
                home.join(dir)
                    .join("NativeMessagingHosts")
                    .join(format!("{HOST_NAME}.json")),
            )
        })
        .collect()
}

/// 매니페스트 내용. `path`에 **지금 실행 중인 이 파일의 절대 경로**를 박으므로, 바이너리를
/// 옮기거나 지우면 등록이 조용히 끊긴다(브라우저는 스폰 실패를 확장 콘솔에만 남긴다) —
/// 그래서 `ensure_installed`가 기동마다 다시 맞춘다.
fn manifest_json(extension_id: &str) -> Result<Value> {
    let exe = std::env::current_exe().context("실행파일 경로 취득 실패")?;
    Ok(json!({
        "name": HOST_NAME,
        "description": "inno-creed 크레덴셜 브릿지 native messaging host",
        "path": exe.to_string_lossy(),
        "type": "stdio",
        "allowed_origins": [format!("chrome-extension://{extension_id}/")],
    }))
}

fn write_manifest(path: &std::path::Path, manifest: &Value) -> Result<()> {
    let dir = path.parent().context("매니페스트 부모 경로 없음")?;
    std::fs::create_dir_all(dir)
        .with_context(|| format!("매니페스트 디렉토리 생성 실패: {}", dir.display()))?;
    std::fs::write(path, serde_json::to_vec_pretty(manifest)?)
        .with_context(|| format!("매니페스트 쓰기 실패: {}", path.display()))
}

/// 등록 다음에 사람이 해야 하는 일. OS와 무관하게 같다.
const LOAD_HINT: &str = "[inno-creed] 등록 완료. Chrome/Edge에서 확장 프로그램을 로드하면(chrome://extensions →\n\
                         개발자 모드 → 압축해제된 확장 프로그램 로드 → extension/ 폴더) 로그인 즉시 크레덴셜이\n\
                         자동으로 전달됩니다.";

/// 서버가 뜰 때마다 등록을 **지금 이 실행 파일에 맞춰 놓는다**(멱등).
///
/// 등록을 사람이 기억해야 하는 별도 단계로 두면 반드시 빠진다 — 확장만 로드하고 등록을
/// 안 하면 브릿지가 **에러 없이 조용히** 안 붙어서 증상만으로는 원인을 못 찾는다. MCP
/// 클라이언트가 실행하는 경로가 곧 브라우저가 스폰해야 할 경로이므로, 여기서 쓰는 값이
/// 사람이 손으로 등록하는 것보다 정확하다(다른 사본에 대고 등록하는 실수가 없다).
///
/// 내용이 이미 같으면 아무것도 하지 않는다. 한 자리라도 어긋나 있으면 다시 등록한다 —
/// 못 쓰는 자리(아래 `install` 주석의 root 소유 디렉토리 등)가 있으면 기동마다 다시
/// 시도하게 되는데, **그게 낫다**: 사용자가 나중에 그 브라우저를 깔거나 권한을 고치면
/// 그 다음 기동에서 스스로 붙는다.
pub fn ensure_installed() -> Result<()> {
    let targets = manifest_targets();
    if targets.is_empty() {
        bail!("native host 매니페스트를 놓을 자리를 정할 수 없습니다");
    }
    let want = serde_json::to_vec_pretty(&manifest_json(DEFAULT_EXTENSION_ID)?)?;
    if targets
        .iter()
        .all(|(_, p)| std::fs::read(p).is_ok_and(|cur| cur == want))
    {
        return Ok(());
    }
    install(DEFAULT_EXTENSION_ID)
}

/// Native Messaging 호스트를 등록한다 — 매니페스트 JSON을 쓰고 Chrome/Edge 레지스트리
/// 하이브 둘 다에 걸어준다(두 브라우저가 각자 다른 하이브를 본다). `reg.exe`를 쓰는 건
/// Windows에 항상 있는 도구라 레지스트리 FFI를 새로 안 만들어도 되기 때문 — 인자를
/// `Command::args`로 넘기므로(셸을 안 거침) 경로에 공백이 있어도 별도 이스케이프가
/// 필요 없다.
#[cfg(target_os = "windows")]
pub fn install(extension_id: &str) -> Result<()> {
    let (_, manifest_path) = manifest_targets().into_iter().next().context("LOCALAPPDATA 없음")?;
    write_manifest(&manifest_path, &manifest_json(extension_id)?)?;
    eprintln!("[inno-creed] native host 매니페스트 작성: {}", manifest_path.display());

    for (browser, hive) in [
        ("Chrome", r"Software\Google\Chrome\NativeMessagingHosts"),
        ("Edge", r"Software\Microsoft\Edge\NativeMessagingHosts"),
    ] {
        let key = format!(r"HKCU\{hive}\{HOST_NAME}");
        let status = std::process::Command::new("reg")
            .args(["add", &key, "/ve", "/d", &manifest_path.to_string_lossy(), "/f"])
            .status()
            .with_context(|| format!("reg.exe 실행 실패({browser})"))?;
        if !status.success() {
            bail!("{browser} 레지스트리 등록 실패: {key}");
        }
        eprintln!("[inno-creed] {browser} native host 등록 완료: {key}");
    }
    eprintln!("{LOAD_HINT}");
    Ok(())
}

/// unix판 등록 — 브라우저마다 자기 디렉토리를 훑으므로 레지스트리 없이 **파일만** 놓으면 된다.
///
/// ⚠️ **한 자리가 실패해도 나머지는 계속 쓴다.** 안 쓰는 브라우저의 디렉토리가 우리가 손댈 수
/// 없는 상태로 남아 있는 일이 실제로 있다 — 이 저장소를 개발한 맥에서 Edge를 깔지도 않았는데
/// `~/Library/Application Support/Microsoft Edge/`가 **root 소유**로 남아 있어 쓰기가
/// `Permission denied`였다. 처음엔 첫 실패에서 멈추게 짰는데, 그러면 정작 쓰는 브라우저인
/// Chrome 등록까지 통째로 실패로 끝난다. 하나도 못 썼을 때만 에러다.
#[cfg(not(target_os = "windows"))]
pub fn install(extension_id: &str) -> Result<()> {
    let manifest = manifest_json(extension_id)?;
    let targets = manifest_targets();
    if targets.is_empty() {
        bail!("HOME이 없어 native host 매니페스트를 놓을 자리를 정할 수 없습니다");
    }
    let mut wrote = 0usize;
    for (browser, path) in &targets {
        match write_manifest(path, &manifest) {
            Ok(()) => {
                wrote += 1;
                eprintln!("[inno-creed] {browser} native host 등록 완료: {}", path.display());
            }
            // 그 브라우저를 안 쓰는 사용자에겐 아무 문제가 아니다 — 경고로만 남긴다.
            Err(e) => eprintln!("[inno-creed] ⚠️ {browser} native host 등록 건너뜀: {e:#}"),
        }
    }
    if wrote == 0 {
        bail!("어느 브라우저에도 native host를 등록하지 못했습니다(위 사유 참고)");
    }
    eprintln!("{LOAD_HINT}");
    Ok(())
}
