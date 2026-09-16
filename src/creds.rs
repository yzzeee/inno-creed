//! 크레덴셜(authToken/signKey) 취득 — 환경변수 → 익스텐션 캐시(**전 OS 정식 경로**) → Chrome →
//! Edge(Win) → Firefox(비-Windows만) → 크레덴셜 파일 순, macOS·Linux·Windows 크로스플랫폼.
//! 브라우저 쿠키 직접 읽기(Chrome/Edge/Firefox)는 **익스텐션이 아직 없을 때를 받아주는 폴백**이다 —
//! 세션 쿠키가 디스크에 없거나 키체인·키링 접근이 막히면 그대로 실패하므로 보장되는 경로가 아니다.
//! 어느 소스에서 왜 막혔는지는 `diagnose()` 한 곳에서만 만든다 — 최종 에러 문구와
//! `doctor`가 그 값 하나를 공유한다(따로 구현하면 "doctor는 OK인데 서버는 실패"가 생긴다).
//! (Edge는 Windows에서만 시도한다 — Chrome과 같은 Chromium 코드베이스라 DB 스키마·암호화
//! 방식은 동일하고 User Data 경로만 다르다.)
//! Chrome/Edge 쿠키 복호화는 OS마다 방식이 다르다:
//!  · macOS  : 키체인 `Chrome Safe Storage` → PBKDF2(SHA1,1003) → AES-128-CBC(iv=0x20×16)
//!  · Linux  : 고정 비번 "peanuts"(키링 미사용시) → PBKDF2(SHA1,1) → AES-128-CBC(iv=0x20×16)
//!  · Windows: `v10`(Local State DPAPI 키)만 취급한다. `v20`(app-bound)은 호출자 프로세스
//!    경로를 검증하므로 제3자 프로세스로는 **설계상 항상 거부**돼(`ChromeKey` 문서 참고)
//!    시도조차 안 한다 — 익스텐션(`extension/`, `native_host.rs`)을 쓴다. Windows는 이 폴백이
//!    사실상 항상 실패하는 쪽이고, 익스텐션 자체는 전 OS 공통 경로다.
//! Firefox `cookies.sqlite`는 전 OS 평문이라 프로필 경로만 OS별로 분기하지만, **Windows에서는
//! 시도하지 않는다** — `gw.innogrid.com`의 세션 쿠키를 Firefox가 브라우저 실행 중엔 그
//! 파일에 아예 쓰지 않는 걸 실측으로 확인했다(WAL 포함 라이브로 직접 읽어도 없음). DBSC와
//! 무관한 별개 이유이고, Firefox 확장 프로그램은 Mozilla AMO 서명 없이는 일반 릴리즈
//! 채널에 설치가 안 돼(Chrome/Edge처럼 "압축해제 로드"로 못 씀) 손쉬운 회피책도 없다 —
//! 지원 안 함으로 확정.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone)]
pub struct Creds {
    pub auth_token: String,
    pub sign_key: String,
}

/// 잠긴 쿠키 DB를 읽기 위한 **호출 고유** 복사본. 경로를 쥐고 있다가 `Drop`에서 지운다.
///
/// **이름을 고유화하는 이유**: 예전에는 임시 디렉토리에 브라우저별 **고정 이름 하나**를 썼다.
/// 취득이 동시에 두 번 일어나면 같은 파일 하나를 두고 `copy`→`open`→`remove_file`이 겹치는데,
/// `fs::copy`는 대상을 **먼저 truncate**하므로 다른 쪽이 그 순간 열면 잘린 DB를 읽거나,
/// 한쪽의 `remove_file` 뒤에 열어 "파일 없음"을 만난다. 이름이 겹치지 않으면 이 창이 닫힌다.
/// (동시 취득이 실제로 일어나는 장면은 관측된 적이 없다 — 코드 구조에서 유도한 방어다.)
///
/// **`Drop`으로 지우는 이유**: 고정 이름일 때는 실패 경로에서 남겨도 다음 호출이 덮어써서
/// 회수됐다. 고유 이름은 그 회수가 없으므로, 지우지 않으면 실패할 때마다 임시 디렉토리에
/// 파일이 쌓인다. 경합을 없애는 대신 누수를 만들지 않으려면 `Drop`이 필요하다.
struct TempCopy(PathBuf);

impl TempCopy {
    /// `src`를 임시 디렉토리의 고유 경로로 복사한다. `tag`는 사람이 알아보기 위한 것(ck/ff).
    ///
    /// 고유성은 **pid + 프로세스 내 단조 카운터**로 만든다. 같은 프로세스 안에서는 카운터가,
    /// 프로세스 사이에서는 pid가 갈라준다.
    ///
    /// ⚠️ **복사보다 소유권을 먼저 잡는다.** `fs::copy`는 대상 파일을 만든 뒤 실패할 수 있는데
    /// (디스크 부족·권한·원본 중도 오류), 복사 성공 후에 `Self`를 만들면 그 부분 파일에는
    /// `Drop`이 붙지 않아 그대로 남는다. 고유 이름이라 다음 호출이 덮어써 회수해주지도 않는다 —
    /// 위 주석이 경계한 "실패할 때마다 쌓인다"가 정확히 이 경로다.
    fn new(src: &Path, tag: &str) -> std::io::Result<Self> {
        let me = Self(unique_temp_path(tag));
        std::fs::copy(src, &me.0)?;
        Ok(me)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

/// `TempCopy`가 쓰는 것과 같은 규칙(pid + 프로세스 내 단조 카운터)의 고유 임시 경로.
/// VSS 폴백처럼 파일을 직접 만든 뒤 `TempCopy(path)`로 감쌀 때 재사용한다.
fn unique_temp_path(tag: &str) -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "inno_creed_{tag}_{}_{}.db",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ))
}

impl Drop for TempCopy {
    fn drop(&mut self) {
        // 지워지지 않아도 할 수 있는 게 없다(다음 실행이 다른 이름을 쓴다). 조용히 넘긴다.
        let _ = std::fs::remove_file(&self.0);
    }
}

// ─────────────────────────── 취득 진단 ───────────────────────────

/// 한 소스를 시도한 결과.
///
/// **`Absent`와 `Failed`를 가르는 것이 이 타입의 존재 이유다.** 예전에는 모든 실패를 같은
/// 무게로 나열해서, Firefox 미설치(`os error 3`)가 "고쳐야 할 두 번째 문제"처럼 보였다.
/// 처방이 없는 것은 강등해야 사용자가 진짜 문제 하나를 본다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// 취득 성공.
    Ok,
    /// 소스가 애초에 없다(브라우저 미설치·env 미설정·파일 없음). 대응 불필요.
    Absent(String),
    /// 소스는 있는데 실패했다 — 사용자가 손댈 곳이 있다. 문구에 **처방**을 담는다.
    Failed(String),
}

/// 소스 하나의 시도 기록.
#[derive(Clone, Debug)]
pub struct SourceReport {
    /// 사람이 읽는 이름("환경변수", "익스텐션 캐시", "Chrome", …).
    pub source: &'static str,
    pub outcome: Outcome,
}

/// 취득 전체의 결과. **에러 문구와 `doctor`가 이 값 하나를 공유한다** — 진단을 두 군데에
/// 따로 구현하면 "doctor는 OK인데 서버는 실패"처럼 서로 어긋나는 순간이 생긴다.
pub struct Diagnosis {
    pub creds: Option<Creds>,
    /// **시도한 순서대로**. 성공하면 그 뒤 소스는 시도하지 않으므로 목록에 없다
    /// (서버의 실제 동작과 같아야 하므로 일부러 앞질러 보지 않는다).
    pub reports: Vec<SourceReport>,
}

/// 소스 시도 실패. `absent`면 잡음(강등), 아니면 처방 대상.
#[derive(Debug)]
struct SourceFail {
    absent: bool,
    msg: String,
}

impl SourceFail {
    fn absent(msg: impl Into<String>) -> Self {
        Self {
            absent: true,
            msg: msg.into(),
        }
    }
    fn failed(msg: impl Into<String>) -> Self {
        Self {
            absent: false,
            msg: msg.into(),
        }
    }
    fn into_outcome(self) -> Outcome {
        if self.absent {
            Outcome::Absent(self.msg)
        } else {
            Outcome::Failed(self.msg)
        }
    }
}

/// `anyhow` 에러를 처방 대상 실패로. (내부 헬퍼가 `?`로 올려보내는 잡다한 io/sqlite 오류용.)
fn failed_from(e: anyhow::Error) -> SourceFail {
    SourceFail::failed(format!("{e:#}"))
}

/// 크레덴셜 취득 진입점. 순서는 `diagnose()`가 정한다.
///
/// **크레덴셜 파일이 브라우저보다 아래인 이유**: 위에 두면 만료된 `creds.json` 하나가 멀쩡한
/// 브라우저 세션을 영영 가린다(지금 환경변수가 가진 병 그대로 — `client.rs`의
/// `reacquire_creds` 주석). 아래에 두면 브라우저가 읽히는 동안은 브라우저가 이기고, 브라우저가
/// **실패할 때만** 파일이 쓰인다. 파일을 쓰는 이유가 애초에 "브라우저에서 못 가져온다"이므로
/// 이 순서로 충분하다.
pub fn from_browser() -> Result<Creds> {
    let d = diagnose();
    match d.creds {
        Some(c) => Ok(c),
        None => bail!("{}", render_failure(&d.reports)),
    }
}

/// 소스 하나를 시도하는 함수.
type TrySource = fn() -> std::result::Result<Creds, SourceFail>;

/// 시도할 소스를 **플랫폼에 맞게** 순서대로 늘어놓는다.
///
/// Edge는 Windows에서만 시도한다(Chrome과 같은 Chromium이라 다른 OS에서 따로 볼 값이 없다).
/// Firefox는 반대로 **Windows에서만 시도하지 않는다** — `gw.innogrid.com`의 세션 쿠키를
/// Firefox가 브라우저 실행 중엔 `cookies.sqlite`에 아예 쓰지 않는 걸 실측으로 확인했다
/// (모듈 문서 참고). 목록에서 통째로 빼지 않고 "해당 없음"으로 남기는 것은, 왜 안 쓰는지를
/// `doctor`가 말해줄 수 있게 하기 위해서다.
fn sources() -> Vec<(&'static str, TrySource)> {
    let mut v: Vec<(&'static str, TrySource)> = vec![
        ("환경변수", try_env),
        ("익스텐션 캐시", try_extension_cache),
        ("Chrome", try_chrome),
    ];
    #[cfg(target_os = "windows")]
    v.push(("Edge", try_edge));
    v.push(("Firefox", try_firefox));
    v.push(("크레덴셜 파일", try_file));
    v
}

/// 시도 순서에 놓인 소스 이름들. `doctor`가 "무엇을 어떤 순서로 보는지"를 안내할 때 쓴다 —
/// **순서를 문서에 따로 적지 않으려는 것이다**(적으면 코드와 어긋난다).
pub fn source_names() -> Vec<&'static str> {
    sources().into_iter().map(|(n, _)| n).collect()
}

/// 소스를 순서대로 시도하며 **기록을 남긴다.** 성공하면 즉시 멈춘다.
pub fn diagnose() -> Diagnosis {
    let mut reports = Vec::new();
    for (source, f) in sources() {
        match f() {
            Ok(c) => {
                reports.push(SourceReport {
                    source,
                    outcome: Outcome::Ok,
                });
                return Diagnosis {
                    creds: Some(c),
                    reports,
                };
            }
            Err(fail) => reports.push(SourceReport {
                source,
                outcome: fail.into_outcome(),
            }),
        }
    }
    Diagnosis {
        creds: None,
        reports,
    }
}

/// 모두 실패했을 때의 사용자용 문구. **처방이 있는 것을 앞세우고, 없는 것은 한 줄로 강등한다.**
fn render_failure(reports: &[SourceReport]) -> String {
    let mut out = String::from("크레덴셜 취득 실패 — gw.innogrid.com 세션을 찾지 못했습니다.");
    let actionable: Vec<&SourceReport> = reports
        .iter()
        .filter(|r| matches!(r.outcome, Outcome::Failed(_)))
        .collect();
    for r in &actionable {
        if let Outcome::Failed(msg) = &r.outcome {
            out.push_str(&format!("\n  ▸ [{}] {msg}", r.source));
        }
    }
    let absent: Vec<&str> = reports
        .iter()
        .filter(|r| matches!(r.outcome, Outcome::Absent(_)))
        .map(|r| r.source)
        .collect();
    if !absent.is_empty() {
        out.push_str(&format!(
            "\n  (해당 없음 — 대응 불필요: {})",
            absent.join(", ")
        ));
    }
    if actionable.is_empty() {
        // 예전에는 여기서 OS를 갈라, 비-Windows에는 "브라우저로 로그인하세요"라고만 했다.
        // 그 말을 들은 사용자는 확장 브릿지를 아예 시도하지 않은 채 직접 읽기만 되풀이했다
        // (실제 사용자 보고). 확장이 전 OS 정식 경로가 된 지금은 한 가지로 안내한다.
        out.push_str(&format!(
            "\n  ▸ 크레덴셜 소스가 하나도 없습니다. **확장 프로그램을 브라우저에 올리세요**(전 OS 정식 경로) — \
             {page}에서 개발자 모드를 켜고 extension/ 폴더를 \
             \"압축해제된 확장 프로그램 로드\"로 고른 뒤 https://gw.innogrid.com 에 로그인하면 됩니다. \
             native host 등록은 서버가 뜰 때마다 스스로 맞춥니다.",
            page = crate::native_host::extensions_page_hint()
        ));
    }
    out.push_str("\n\n무엇이 어디서 막혔는지는 `inno-creed doctor`가 한 화면으로 보여줍니다.");
    out
}

/// 수동 입력(env) — 브라우저 복호화가 불가한 환경의 확실한 우회.
/// 두 값 모두 지정돼 있어야 쓴다(authToken은 URL 인코딩 허용).
fn try_env() -> std::result::Result<Creds, SourceFail> {
    let at = env_nonempty("INNO_CREED_AUTH_TOKEN");
    let hk = env_nonempty("INNO_CREED_SIGN_KEY");
    match (at, hk) {
        (Some(at), Some(hk)) => Ok(Creds {
            auth_token: url_decode(&at),
            sign_key: hk,
        }),
        (None, None) => Err(SourceFail::absent("미설정")),
        // 한쪽만 넣은 것은 **조용히 무시하면 안 된다** — 넣었는데 왜 안 먹는지 알 길이 없다.
        (Some(_), None) => Err(SourceFail::failed(
            "INNO_CREED_AUTH_TOKEN만 설정됨 — INNO_CREED_SIGN_KEY(BIZCUBE_HK 값)도 함께 지정해야 사용됩니다.",
        )),
        (None, Some(_)) => Err(SourceFail::failed(
            "INNO_CREED_SIGN_KEY만 설정됨 — INNO_CREED_AUTH_TOKEN(BIZCUBE_AT 값)도 함께 지정해야 사용됩니다.",
        )),
    }
}

// ────────────────────────── 익스텐션 브릿지 캐시 ──────────────────────────
//
// Chrome/Edge 익스텐션(`extension/`)이 `chrome.cookies` API로 읽은 값을 Native Messaging
// 으로 `native_host::run()`에 전달하면, 거기서 이 경로의 파일에 저장한다. 쿠키 DB
// 파일이나 COM을 전혀 안 거치므로 v20 암호화·파일 잠금·세션쿠키 소실 문제가 다 없다.

/// 익스텐션 캐시 파일 경로. `INNO_CREED_EXTENSION_CACHE`로 직접 지정 가능.
/// `native_host.rs`(쓰기)와 여기(읽기)가 공유한다.
pub(crate) fn extension_cache_path() -> Result<PathBuf> {
    if let Some(p) = env_nonempty("INNO_CREED_EXTENSION_CACHE") {
        return Ok(PathBuf::from(p));
    }
    #[cfg(target_os = "windows")]
    {
        let local = std::env::var("LOCALAPPDATA")?;
        Ok(PathBuf::from(format!("{local}\\inno-creed\\ext-creds.json")))
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME")?;
        Ok(PathBuf::from(format!(
            "{home}/Library/Application Support/inno-creed/ext-creds.json"
        )))
    }
    #[cfg(target_os = "linux")]
    {
        let home = std::env::var("HOME")?;
        Ok(PathBuf::from(format!(
            "{home}/.local/share/inno-creed/ext-creds.json"
        )))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        bail!("지원하지 않는 OS")
    }
}

/// 익스텐션이 떨어뜨려둔 캐시 파일에서 크레덴셜 취득.
///
/// **캐시가 없으면 전 OS에서 `Failed`다.** 확장 브릿지가 정식 설치 경로이기 때문이다.
/// macOS·Linux는 쿠키 DB 직접 읽기(아래 소스들)로도 **동작할 수는 있지만**, 세션 쿠키가
/// 디스크에 없거나(Chrome "중단한 위치에서 계속하기" 꺼짐) 키체인·키링 권한이 막히면
/// 그대로 실패한다 — 되는지 여부가 사용자 환경에 달려 **보장되지 않는다**. 그래서 가이드는
/// 전 OS 공통으로 확장을 필수로 안내하고, 진단도 그 기준에 맞춘다. 직접 읽기는 확장이
/// 아직 없을 때를 받아주는 폴백으로 남긴다.
fn try_extension_cache() -> std::result::Result<Creds, SourceFail> {
    let path = extension_cache_path().map_err(failed_from)?;
    let txt = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(SourceFail::failed(format!(
                "익스텐션 캐시 없음: {} — 확장 브릿지가 정식 경로입니다. \
                 {page}에서 개발자 모드를 켜고 extension/ 폴더를 \
                 \"압축해제된 확장 프로그램 로드\"로 올린 다음, https://gw.innogrid.com 에 \
                 로그인하세요(로그인 즉시 자동 전달). native host 등록은 서버 기동 때 자동으로 됩니다.",
                path.display(),
                page = crate::native_host::extensions_page_hint()
            )));
        }
        Err(e) => {
            return Err(SourceFail::failed(format!(
                "익스텐션 캐시를 읽지 못했습니다({}): {e}",
                path.display()
            )));
        }
    };
    // 파일이 **있는데** 못 쓰는 것은 늘 처방 대상이다 — 익스텐션이 절반만 동작한 상태다.
    let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| {
        SourceFail::failed(format!(
            "익스텐션 캐시 형식 오류({}): {e}. 익스텐션을 제거했다 다시 로드한 뒤 gw.innogrid.com을 새로고침하세요.",
            path.display()
        ))
    })?;
    let field = |k: &str| -> std::result::Result<String, SourceFail> {
        v.get(k)
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| {
                SourceFail::failed(format!(
                    "익스텐션 캐시({})에 {k}이(가) 없습니다 — gw.innogrid.com에 로그인한 뒤 탭을 새로고침하면 다시 전달됩니다.",
                    path.display()
                ))
            })
    };
    Ok(Creds {
        auth_token: url_decode(&field("authToken")?),
        sign_key: field("signKey")?,
    })
}

// ─────────────────────────────── Chrome ───────────────────────────────

/// Chrome 쿠키에서 크레덴셜 취득(OS별 복호화). 진단 문구는 `try_chrome`이 만든다.
pub fn from_chrome() -> Result<Creds> {
    try_chrome().map_err(|f| anyhow::anyhow!("{}", f.msg))
}

fn try_chrome() -> std::result::Result<Creds, SourceFail> {
    let db = match chrome_cookie_db() {
        Ok(p) => p,
        Err(e) => {
            // User Data 루트조차 없으면 Chrome 미설치다 — 처방이 없으니 강등한다.
            let root_missing = chrome_user_data_dir().map(|r| !r.exists()).unwrap_or(true);
            return Err(if root_missing {
                SourceFail::absent("Chrome 미설치(또는 프로필 없음)")
            } else {
                SourceFail::failed(format!("{e:#}"))
            });
        }
    };
    let cookies = read_cookies_db(&db, "ck", "Chrome").map_err(failed_from)?;
    finish_chromium("Chrome", &db, cookies, chrome_key)
}

/// `chrome_key()`/`edge_key()`가 돌려주는 키 타입 — OS마다 다르다(win=구조체, 그 외=바이트열).
/// `finish_chromium`이 Chrome/Edge와 OS를 가로질러 하나의 시그니처로 받으려고 이름을 붙인다.
#[cfg(target_os = "windows")]
type ChromeKeyOwned = ChromeKey;
#[cfg(not(target_os = "windows"))]
type ChromeKeyOwned = Vec<u8>;

/// Chrome/Edge 공통 마무리 — 쿠키 목록에서 두 값을 뽑고, 실패를 원인별로 가른다.
///
/// **키 취득을 클로저로 미루는 이유**: 읽을 쿠키가 있다고 확인한 뒤에 키를 가져와야 한다.
/// macOS는 그 단계에서 키체인 프롬프트가 뜨는데, 어차피 못 쓸 상황에 사용자를 놀래킬 이유가 없다.
fn finish_chromium(
    browser: &str,
    db: &Path,
    cookies: Vec<(String, Vec<u8>)>,
    key: impl FnOnce() -> Result<ChromeKeyOwned>,
) -> std::result::Result<Creds, SourceFail> {
    // gw 쿠키가 **하나도 없는 것**과 **있는데 BIZCUBE_AT만 없는 것**은 원인도 처방도 다르다.
    // 전자만 미로그인이다. 이 구분을 안 해서 세션 쿠키 사용자를 "다시 로그인 → 여전히 실패"
    // 루프로 보낸 적이 있다.
    if cookies.is_empty() {
        let env_hint = if browser == "Edge" {
            "INNO_CREED_EDGE_COOKIES"
        } else {
            "INNO_CREED_CHROME_COOKIES"
        };
        return Err(SourceFail::failed(format!(
            "쿠키 DB({})에 gw.innogrid.com 쿠키가 하나도 없습니다 — 이 프로필로 로그인한 적이 없습니다. \
             {browser}으로 https://gw.innogrid.com 에 로그인하거나, 다른 프로필을 쓴다면 {env_hint}로 지정하세요.",
            db.display()
        )));
    }

    let key = key()
        .with_context(|| format!("{browser} 복호화 키 취득 실패(mac 키체인/win DPAPI)"))
        .map_err(failed_from)?;

    let mut auth_token = None;
    let mut sign_key = None;
    let mut decrypt_failed = false; // 대상 쿠키는 있으나 복호화만 실패한 경우 구분
    for (name, enc) in cookies {
        if name != "BIZCUBE_AT" && name != "BIZCUBE_HK" {
            continue;
        }
        match decrypt_chrome(&enc, &key) {
            Ok(val) => match name.as_str() {
                "BIZCUBE_AT" => auth_token = Some(url_decode(&val)),
                "BIZCUBE_HK" => sign_key = Some(val),
                _ => {}
            },
            Err(_) => decrypt_failed = true,
        }
    }
    // 쿠키는 있는데 복호화만 실패 → 키 스킴 불일치(키링/app-bound). "없음"과 구분해 안내.
    if auth_token.is_none() && decrypt_failed {
        return Err(SourceFail::failed(
            "BIZCUBE 쿠키는 있으나 복호화 실패. Linux 키링(gnome-keyring/kwallet, v11) 사용 시 `secret-tool`(libsecret-tools)이 설치돼 있어야 합니다 — `sudo apt install libsecret-tools` 후 재시도. \
             Windows app-bound(v20)은 호출자 프로세스 경로 검증 때문에 inno-creed 같은 제3자 프로세스로는 설계상 항상 거부됩니다(버전·설정과 무관, 시도조차 안 함). \
             Windows에서 확실한 방법: `inno-creed --install-extension-host`로 Chrome/Edge 확장 프로그램을 설치하거나, `inno-creed auth set`으로 DevTools에서 복사한 쿠키 값을 직접 저장하세요."
                .to_string(),
        ));
    }
    let missing = || SourceFail::failed(missing_at_msg(&db.display().to_string()));
    Ok(Creds {
        auth_token: auth_token.ok_or_else(missing)?,
        sign_key: sign_key.ok_or_else(missing)?,
    })
}

/// gw 쿠키는 있는데 `BIZCUBE_AT`/`BIZCUBE_HK`가 없을 때의 문구.
///
/// **원인을 하나로 단정하지 않는다.** 세션 쿠키라 디스크에 안 남은 경우와, 로그아웃해서
/// 지워진 경우가 둘 다 같은 모습으로 보인다 — 단정하면 예전처럼 또 오진이 된다.
fn missing_at_msg(where_: &str) -> String {
    format!(
        "쿠키 DB({where_})에 gw.innogrid.com 쿠키는 있으나 BIZCUBE_AT/BIZCUBE_HK가 없습니다. 원인은 둘 중 하나입니다.\n\
         \x20      (1) 세션 쿠키라 디스크에 기록되지 않음 — DevTools(F12)→Application→Cookies에서 BIZCUBE_AT의 Expires가 `Session`이면 이쪽입니다.\n\
         \x20          해결: Chrome/Edge 확장 프로그램(`inno-creed --install-extension-host`)을 쓰면 쿠키 DB를 거치지 않아 이 문제 자체가 없습니다. 확장을 못 쓰는 환경이면 `inno-creed auth set`으로 두 값을 직접 저장하세요.\n\
         \x20      (2) 로그아웃 상태 — 해결: 브라우저로 https://gw.innogrid.com 에 로그인."
    )
}

/// Chrome User Data 루트 디렉토리(OS별). `INNO_CREED_CHROME_USER_DATA`로 오버라이드 가능.
fn chrome_user_data_dir() -> Result<PathBuf> {
    if let Some(p) = env_nonempty("INNO_CREED_CHROME_USER_DATA") {
        return Ok(PathBuf::from(p));
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME")?;
        Ok(PathBuf::from(format!(
            "{home}/Library/Application Support/Google/Chrome"
        )))
    }
    #[cfg(target_os = "linux")]
    {
        let home = std::env::var("HOME")?;
        let gc = PathBuf::from(format!("{home}/.config/google-chrome")); // 스테이블 우선
        if gc.exists() {
            return Ok(gc);
        }
        return Ok(PathBuf::from(format!("{home}/.config/chromium"))); // Chromium 폴백
    }
    #[cfg(target_os = "windows")]
    {
        let local = std::env::var("LOCALAPPDATA")?;
        Ok(PathBuf::from(format!("{local}\\Google\\Chrome\\User Data")))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        bail!("지원하지 않는 OS")
    }
}

/// 쿠키 DB 경로. `INNO_CREED_CHROME_COOKIES`로 직접 지정 가능. 미지정 시 User Data 루트 아래
/// 신버전(Default/Network/Cookies) 우선, 없으면 구버전(Default/Cookies).
fn chrome_cookie_db() -> Result<PathBuf> {
    if let Some(p) = env_nonempty("INNO_CREED_CHROME_COOKIES") {
        return Ok(PathBuf::from(p));
    }
    let root = chrome_user_data_dir()?;
    let net = root.join("Default").join("Network").join("Cookies");
    if net.exists() {
        return Ok(net);
    }
    let old = root.join("Default").join("Cookies");
    if old.exists() {
        return Ok(old);
    }
    bail!(
        "Chrome 쿠키 DB를 찾지 못함. 확인한 경로:\n      - {}\n      - {}\n    (User Data 루트: {})\n    비표준 위치(snap/flatpak/커스텀 프로필)면 INNO_CREED_CHROME_COOKIES(파일) 또는 INNO_CREED_CHROME_USER_DATA(루트)로 지정하세요.",
        net.display(),
        old.display(),
        root.display()
    )
}

// ─────────────────────────────── Edge (Windows 전용) ───────────────────────────────
//
// Edge는 Chrome과 같은 Chromium 코드베이스라 쿠키 DB 스키마·암호화 방식이 동일하다.
// 다른 건 User Data 경로와 v20 Elevation Service의 CLSID/IID/vtable뿐이다. macOS/Linux
// Edge는 지원 범위 밖이라(이 프로젝트가 대상으로 하는 사내 환경은 Windows Edge뿐) 이
// 섹션은 전부 Windows 전용이다.

/// Edge User Data 루트 디렉토리. `INNO_CREED_EDGE_USER_DATA`로 오버라이드 가능.
#[cfg(target_os = "windows")]
fn edge_user_data_dir() -> Result<PathBuf> {
    if let Some(p) = env_nonempty("INNO_CREED_EDGE_USER_DATA") {
        return Ok(PathBuf::from(p));
    }
    let local = std::env::var("LOCALAPPDATA")?;
    Ok(PathBuf::from(format!("{local}\\Microsoft\\Edge\\User Data")))
}

/// Edge 쿠키 DB 경로. `INNO_CREED_EDGE_COOKIES`로 직접 지정 가능. 규칙은 `chrome_cookie_db`와 동일.
#[cfg(target_os = "windows")]
fn edge_cookie_db() -> Result<PathBuf> {
    if let Some(p) = env_nonempty("INNO_CREED_EDGE_COOKIES") {
        return Ok(PathBuf::from(p));
    }
    let root = edge_user_data_dir()?;
    let net = root.join("Default").join("Network").join("Cookies");
    if net.exists() {
        return Ok(net);
    }
    let old = root.join("Default").join("Cookies");
    if old.exists() {
        return Ok(old);
    }
    bail!(
        "Edge 쿠키 DB를 찾지 못함. 확인한 경로:\n      - {}\n      - {}\n    (User Data 루트: {})\n    비표준 위치면 INNO_CREED_EDGE_COOKIES(파일) 또는 INNO_CREED_EDGE_USER_DATA(루트)로 지정하세요.",
        net.display(),
        old.display(),
        root.display()
    )
}

/// Edge 쿠키에서 크레덴셜 취득. 흐름은 `from_chrome`과 동일 — 차이는 경로와 키뿐.
#[cfg(target_os = "windows")]
pub fn from_edge() -> Result<Creds> {
    try_edge().map_err(|f| anyhow::anyhow!("{}", f.msg))
}

#[cfg(target_os = "windows")]
fn try_edge() -> std::result::Result<Creds, SourceFail> {
    let db = match edge_cookie_db() {
        Ok(p) => p,
        Err(e) => {
            let root_missing = edge_user_data_dir().map(|r| !r.exists()).unwrap_or(true);
            return Err(if root_missing {
                SourceFail::absent("Edge 미설치(또는 프로필 없음)")
            } else {
                SourceFail::failed(format!("{e:#}"))
            });
        }
    };
    let cookies = read_cookies_db(&db, "eg", "Edge").map_err(failed_from)?;
    finish_chromium("Edge", &db, cookies, edge_key)
}

/// 쿠키 DB 복사 실패를 사용자용 에러로. Windows 공유 위반(32)·잠금 위반(33)은 원인이 하나뿐
/// (브라우저가 파일을 붙들고 있음)이라 처방을 단정해도 된다.
fn locked_db_error(browser: &str, db: &Path, e: std::io::Error) -> anyhow::Error {
    if matches!(e.raw_os_error(), Some(32) | Some(33)) {
        anyhow::anyhow!(
            "{browser}이(가) 쿠키 DB를 붙들고 있어 읽지 못했습니다({}). {browser}을(를) **완전히** 종료한 뒤 다시 시도하세요 \
             — 창을 닫아도 백그라운드 프로세스가 남습니다(Chrome은 설정→시스템→\"Chrome을 닫아도 백그라운드 앱 계속 실행\" 끄기, \
             또는 작업 관리자에서 프로세스 전부 종료).\n\
             \x20      종료하지 않고 쓰려면 확장 프로그램(`inno-creed --install-extension-host`)을 설치하거나 \
             `inno-creed auth set`으로 쿠키 값을 직접 저장하세요. (원문: {e})",
            db.display()
        )
    } else {
        anyhow::anyhow!("{browser} 쿠키 DB 읽기 실패: {} ({e})", db.display())
    }
}

/// Chrome/Edge 공용 — 둘 다 같은 쿠키 DB 스키마(SQLite `cookies` 테이블)를 쓴다.
/// `tag`는 `TempCopy` 임시파일 이름 구분용("ck"/"eg"), `browser`는 에러 문구용.
fn read_cookies_db(db: &Path, tag: &str, browser: &str) -> Result<Vec<(String, Vec<u8>)>> {
    // 잠금 회피: 복사본을 읽음. (Windows에서 브라우저 실행 중이면 배타 잠금이라 copy 실패 →
    // 종료 필요. 인포스틸러 대응으로 브라우저가 의도적으로 거는 잠금이라 `FileShare` 어떤
    // 조합으로도 못 뚫는다, 실측 확인함. VSS로 우회하는 시도는 해봤으나 Defender가 그 조합
    // 자체를 악성 패턴으로 오탐해 폐기함.)
    // 복사본 이름은 호출마다 고유하다 — 이유는 `TempCopy` 주석.
    //
    // ⚠️ **여기서 처방을 붙여야 한다.** 예전에는 OS 원문("다른 프로세스가 파일을 사용 중…
    // os error 32")과 경로만 올려보내서, 정작 사용자가 할 일("브라우저를 완전히 종료")은
    // 설치 문서에만 있었다. 에러를 보는 순간에 문서는 눈앞에 없다.
    let tmp = TempCopy::new(db, tag).map_err(|e| locked_db_error(browser, db, e))?;
    let conn = rusqlite::Connection::open(tmp.path())?;
    let mut stmt =
        conn.prepare("SELECT name, encrypted_value FROM cookies WHERE host_key='gw.innogrid.com'")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?)))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    // ⚠️ `conn`을 `tmp`보다 먼저 닫아야 한다 — Windows는 열린 파일을 지우지 못한다.
    // 지역변수는 선언 역순으로 drop되므로(`tmp`가 먼저 선언 = 나중에 drop) 순서는 맞지만,
    // 조기 반환 경로까지 포함해 의도를 분명히 하려고 명시적으로 닫는다.
    drop(stmt);
    drop(conn);
    Ok(out)
}

/// OS별 Chrome 복호화 키. mac/linux=AES-128-CBC용 16B, windows=AES-256-GCM용 32B.
#[cfg(target_os = "macos")]
fn chrome_key() -> Result<Vec<u8>> {
    let pw = keychain_password().context("Chrome Safe Storage 키체인 접근 실패")?;
    Ok(pbkdf2_key(&pw, 1003, 16))
}
#[cfg(target_os = "linux")]
fn chrome_key() -> Result<Vec<u8>> {
    // 1) 키링(gnome-keyring/kwallet)에 저장된 "Chrome Safe Storage" 비밀 시도(v11 쿠키).
    // 2) 키링 미사용 Chrome은 고정 비번 "peanuts"(v10). 둘 다 PBKDF2 반복 1회.
    if let Some(secret) = linux_keyring_secret() {
        return Ok(pbkdf2_key(secret.as_bytes(), 1, 16));
    }
    Ok(pbkdf2_key(b"peanuts", 1, 16))
}

/// Secret Service(gnome-keyring/kwallet)에서 Chrome/Chromium 저장소 키 조회. `secret-tool`(libsecret-tools) 필요.
/// 없거나 조회 실패면 None → 호출부가 "peanuts"로 폴백.
#[cfg(target_os = "linux")]
fn linux_keyring_secret() -> Option<String> {
    for app in ["chrome", "chromium"] {
        let out = std::process::Command::new("secret-tool")
            .args(["lookup", "application", app])
            .output()
            .ok()?; // secret-tool 자체가 없으면 None(→ peanuts 폴백)
        if out.status.success() && !out.stdout.is_empty() {
            let s = String::from_utf8_lossy(&out.stdout)
                .trim_end_matches(['\n', '\r'])
                .to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    None
}
/// Windows Chrome/Edge 키. `v10`(DPAPI)만 취급한다 — `v20`(app-bound)은 호출자 프로세스
/// 경로를 검증하므로 inno-creed 같은 제3자 프로세스로는 COM으로 아무리 정교하게 접근해도
/// **설계상 항상 거부된다**(실측 확인: Edge에서 `IElevator::DecryptData`가
/// `hr=0x8004B016, last_error=5`로 거부 — COM 레벨이 아니라 서비스 내부 로직의 명시적
/// 거부). `gw.innogrid.com`의 `BIZCUBE_AT`/`HK`는 Windows에서 전부 `v20`이라 이 경로로는
/// 원천적으로 못 푼다 — Chrome/Edge 확장 프로그램(`extension/`, `--install-extension-host`)을 쓴다
/// (전 OS 정식 경로다. Windows는 그중 폴백이 전혀 통하지 않는 쪽일 뿐이다). 한때 COM 활성화(`Elevation` 모니커 vs 평범한
/// `CoCreateInstance`)까지 정교하게 맞춰 시도해봤으나(성공해도 위 경로검증에 막힘) 항상
/// 실패하는 코드를 유지할 이유가 없어 걷어냈다 — 그때의 기록은
/// `.claude-workspace/release-notes-v2.0.0.md`(git 미추적).
#[cfg(target_os = "windows")]
struct ChromeKey {
    v10: Option<Vec<u8>>, // os_crypt.encrypted_key(DPAPI) → 구형 v10 쿠키
}

#[cfg(target_os = "windows")]
fn chrome_key() -> Result<ChromeKey> {
    read_os_crypt_key(&chrome_user_data_dir()?)
}

#[cfg(target_os = "windows")]
fn edge_key() -> Result<ChromeKey> {
    read_os_crypt_key(&edge_user_data_dir()?)
}

/// `Local State`에서 os_crypt DPAPI 키(v10)를 읽는다. Chrome/Edge 공용 — `user_data_dir`만 다르다.
#[cfg(target_os = "windows")]
fn read_os_crypt_key(user_data_dir: &Path) -> Result<ChromeKey> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let ls = user_data_dir.join("Local State");
    let txt = std::fs::read_to_string(&ls)
        .with_context(|| format!("Local State 읽기 실패: {}", ls.display()))?;
    let v: serde_json::Value = serde_json::from_str(&txt)?;

    // v10: DPAPI 래핑 키("DPAPI" 접두 제거 → CryptUnprotectData).
    let v10 = v
        .get("os_crypt")
        .and_then(|o| o.get("encrypted_key"))
        .and_then(|k| k.as_str())
        .and_then(|b64| STANDARD.decode(b64).ok())
        .and_then(|mut raw| {
            if raw.len() >= 5 && &raw[..5] == b"DPAPI" {
                raw.drain(0..5);
            }
            dpapi_unprotect(&raw).ok()
        });

    if v10.is_none() {
        bail!("복호화 키 취득 실패 — Local State에 os_crypt.encrypted_key가 없거나 DPAPI 복호화 불가");
    }
    Ok(ChromeKey { v10 })
}

#[cfg(not(target_os = "windows"))]
fn pbkdf2_key(pw: &[u8], iters: u32, len: usize) -> Vec<u8> {
    let mut key = vec![0u8; len];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(pw, b"saltysalt", iters, &mut key);
    key
}

/// mac/linux: v10 접두(3B) 제거 → AES-128-CBC(iv=0x20×16, Pkcs7).
#[cfg(not(target_os = "windows"))]
fn decrypt_chrome(enc: &[u8], key: &[u8]) -> Result<String> {
    use aes::Aes128;
    use cbc::cipher::{block_padding::Pkcs7, BlockModeDecrypt, KeyIvInit};
    type Dec = cbc::Decryptor<Aes128>;

    if enc.len() < 3 {
        bail!("encrypted value too short");
    }
    let k: [u8; 16] = key.try_into().map_err(|_| anyhow::anyhow!("CBC 키 길이 오류"))?;
    let iv = [0x20u8; 16];
    let mut buf = enc[3..].to_vec();
    let pt = Dec::new(&k.into(), &iv.into())
        .decrypt_padded::<Pkcs7>(&mut buf)
        .map_err(|e| anyhow::anyhow!("AES-CBC 복호화 실패: {e}"))?;
    Ok(strip_domain_hash(pt.to_vec()))
}

/// windows: 접두(3B) + nonce(12B) + ciphertext + tag(16B) → AES-256-GCM. `v10`(DPAPI)만
/// 취급한다 — `v20`(app-bound)은 애초에 시도조차 안 하고 즉시 안내 에러를 낸다(이유는
/// `ChromeKey` 문서).
#[cfg(target_os = "windows")]
fn decrypt_chrome(enc: &[u8], key: &ChromeKey) -> Result<String> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Nonce};

    if enc.len() < 3 + 12 + 16 {
        bail!("gcm cookie too short");
    }
    if &enc[..3] == b"v20" {
        bail!(
            "v20(app-bound) 쿠키 — Windows Chrome/Edge의 app-bound 암호화는 제3자 프로세스로는 \
             설계상 항상 거부됩니다(호출자 프로세스 경로 검증). `inno-creed --install-extension-host`로 \
             Chrome/Edge 확장 프로그램을 쓰세요."
        );
    }
    let k = key
        .v10
        .as_deref()
        .context("v10 쿠키인데 os_crypt DPAPI 키 취득 실패")?;
    let nonce = Nonce::try_from(&enc[3..15]).map_err(|_| anyhow::anyhow!("GCM nonce 길이 오류"))?;
    let ct = &enc[15..];
    let cipher =
        Aes256Gcm::new_from_slice(k).map_err(|_| anyhow::anyhow!("GCM 키 길이 오류(32B 필요)"))?;
    let pt = cipher
        .decrypt(&nonce, ct)
        .map_err(|_| anyhow::anyhow!("AES-GCM 복호화 실패"))?;
    Ok(strip_domain_hash(pt))
}

/// 최신 Chrome은 평문 앞에 32B 도메인 SHA256을 붙인다 — utf8 파싱 실패 시 앞 32B 제거.
fn strip_domain_hash(pt: Vec<u8>) -> String {
    match std::str::from_utf8(&pt) {
        Ok(s) => s.to_string(),
        Err(_) => {
            let start = 32.min(pt.len());
            String::from_utf8_lossy(&pt[start..]).into_owned()
        }
    }
}

#[cfg(target_os = "macos")]
fn keychain_password() -> Result<Vec<u8>> {
    use std::process::Command;
    let out = Command::new("security")
        .args([
            "find-generic-password",
            "-w",
            "-s",
            "Chrome Safe Storage",
            "-a",
            "Chrome",
        ])
        .output()?;
    if !out.status.success() {
        bail!("security 명령 실패");
    }
    let mut p = out.stdout;
    while p.last() == Some(&b'\n') {
        p.pop();
    }
    Ok(p)
}

/// Windows DPAPI `CryptUnprotectData` FFI(crypt32). 현재 사용자 컨텍스트로 복호화.
#[cfg(target_os = "windows")]
fn dpapi_unprotect(data: &[u8]) -> Result<Vec<u8>> {
    use core::ffi::c_void;
    #[repr(C)]
    struct DataBlob {
        cb_data: u32,
        pb_data: *mut u8,
    }
    #[link(name = "crypt32")]
    unsafe extern "system" {
        fn CryptUnprotectData(
            p_data_in: *const DataBlob,
            ppsz_desc: *mut *mut u16,
            p_entropy: *const DataBlob,
            p_reserved: *mut c_void,
            p_prompt: *mut c_void,
            dw_flags: u32,
            p_data_out: *mut DataBlob,
        ) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LocalFree(h_mem: *mut c_void) -> *mut c_void;
    }

    let in_blob = DataBlob {
        cb_data: data.len() as u32,
        pb_data: data.as_ptr() as *mut u8,
    };
    let mut out_blob = DataBlob {
        cb_data: 0,
        pb_data: std::ptr::null_mut(),
    };
    let ok = unsafe {
        CryptUnprotectData(
            &in_blob,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
            &mut out_blob,
        )
    };
    if ok == 0 {
        bail!("CryptUnprotectData 실패");
    }
    let out =
        unsafe { std::slice::from_raw_parts(out_blob.pb_data, out_blob.cb_data as usize).to_vec() };
    unsafe {
        LocalFree(out_blob.pb_data as *mut c_void);
    }
    Ok(out)
}

// ─────────────────────────────── Firefox ───────────────────────────────

/// Firefox 프로필 루트 디렉토리(OS별). `INNO_CREED_FIREFOX_DIR`로 오버라이드 가능
/// (snap `~/snap/firefox/common/.mozilla/firefox`, flatpak 등 비표준 경로 대응).
fn firefox_profiles_dir() -> Result<PathBuf> {
    if let Some(p) = env_nonempty("INNO_CREED_FIREFOX_DIR") {
        return Ok(PathBuf::from(p));
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME")?;
        Ok(PathBuf::from(format!(
            "{home}/Library/Application Support/Firefox/Profiles"
        )))
    }
    #[cfg(target_os = "linux")]
    {
        let home = std::env::var("HOME")?;
        return Ok(PathBuf::from(format!("{home}/.mozilla/firefox")));
    }
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA")?;
        Ok(PathBuf::from(format!(
            "{appdata}\\Mozilla\\Firefox\\Profiles"
        )))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        bail!("지원하지 않는 OS")
    }
}

/// Firefox 쿠키에서 크레덴셜 취득. 진단 문구는 `try_firefox`가 만든다.
pub fn from_firefox() -> Result<Creds> {
    try_firefox().map_err(|f| anyhow::anyhow!("{}", f.msg))
}

/// `cookies.sqlite`는 **평문**이라 복호화 불필요. 프로필 자동 탐색(`*.default*` 우선).
/// Chrome이 없거나 미로그인일 때의 폴백.
///
/// **Windows에서는 시도조차 하지 않는다** — 이유는 모듈 문서. 목록에서 빼는 대신 "해당 없음"을
/// 돌려주는 것은, 왜 안 쓰는지를 `doctor`가 말해줄 수 있게 하기 위해서다(에러 문구에서는
/// 이름만 남고 강등된다).
fn try_firefox() -> std::result::Result<Creds, SourceFail> {
    #[cfg(target_os = "windows")]
    return Err(SourceFail::absent(
        "Windows에서는 지원하지 않습니다 — gw.innogrid.com의 세션 쿠키를 Firefox가 브라우저 실행 중엔 \
         cookies.sqlite에 아예 쓰지 않습니다(DBSC와 무관한 별개 이유, 실측 확인). \
         Firefox 확장 프로그램은 AMO 서명 없이 릴리즈 채널에 설치가 안 돼 우회책도 없습니다.",
    ));

    #[cfg(not(target_os = "windows"))]
    {
        // 1) cookies.sqlite 직접 지정(최우선) 2) 프로필 디렉토리 스캔.
        let src = if let Some(p) = env_nonempty("INNO_CREED_FIREFOX_COOKIES") {
            let pb = PathBuf::from(&p);
            if !pb.exists() {
                // 사용자가 **직접 지정한** 경로가 틀린 것이므로 강등하지 않는다.
                return Err(SourceFail::failed(format!(
                    "INNO_CREED_FIREFOX_COOKIES가 가리키는 파일이 없습니다: {p}"
                )));
            }
            pb
        } else {
            let profiles = firefox_profiles_dir().map_err(failed_from)?;
            // 프로필 디렉토리 자체가 없으면 **Firefox 미설치**다. 예전에는 이걸 Chrome 실패와
            // 나란히 `os error 3`으로 찍어서, 대응 불필요한 것을 문제로 읽게 만들었다.
            let Ok(entries) = std::fs::read_dir(&profiles) else {
                return Err(SourceFail::absent(format!(
                    "Firefox 미설치(프로필 디렉토리 없음: {}). 쓰고 있다면 snap은 ~/snap/firefox/common/.mozilla/firefox, \
                     flatpak은 ~/.var/app/org.mozilla.firefox/.mozilla/firefox — INNO_CREED_FIREFOX_DIR로 지정하세요.",
                    profiles.display()
                )));
            };
            let mut db_path = None;
            for entry in entries {
                let dir = entry.map_err(|e| SourceFail::failed(e.to_string()))?.path();
                let ck = dir.join("cookies.sqlite");
                if ck.exists() {
                    let is_default = dir
                        .file_name()
                        .map(|n| n.to_string_lossy().contains("default"))
                        .unwrap_or(false);
                    if is_default {
                        db_path = Some(ck);
                        break;
                    }
                    db_path.get_or_insert(ck);
                }
            }
            match db_path {
                Some(p) => p,
                None => {
                    return Err(SourceFail::absent(format!(
                        "Firefox 프로필 디렉토리({})에 cookies.sqlite가 있는 프로필이 없음",
                        profiles.display()
                    )));
                }
            }
        };

        // 잠금 회피: 복사본을 읽음. 이름은 호출마다 고유하다 — 이유는 `TempCopy` 주석.
        let tmp = TempCopy::new(&src, "ff")
            .map_err(|e| failed_from(locked_db_error("Firefox", &src, e)))?;
        let (auth_token, sign_key) = read_firefox_cookies(tmp.path()).map_err(failed_from)?;

        let missing = || SourceFail::failed(missing_at_msg(&src.display().to_string()));
        Ok(Creds {
            auth_token: auth_token.ok_or_else(missing)?,
            sign_key: sign_key.ok_or_else(missing)?,
        })
    }
}

/// 복사본에서 gw 쿠키 두 개를 뽑는다. `conn`은 `tmp`(호출부)보다 먼저 닫혀야 하므로
/// (Windows는 열린 파일을 지우지 못한다) 별도 함수로 잘라 수명을 분명히 한다.
#[cfg(not(target_os = "windows"))]
fn read_firefox_cookies(db: &Path) -> Result<(Option<String>, Option<String>)> {
    let conn = rusqlite::Connection::open(db)?;
    let mut stmt =
        conn.prepare("SELECT name, value FROM moz_cookies WHERE host LIKE '%gw.innogrid.com'")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut auth_token = None;
    let mut sign_key = None;
    for r in rows {
        let (name, val) = r?;
        match name.as_str() {
            "BIZCUBE_AT" => auth_token = Some(url_decode(&val)),
            "BIZCUBE_HK" => sign_key = Some(val),
            _ => {}
        }
    }
    drop(stmt);
    drop(conn);
    Ok((auth_token, sign_key))
}

// ─────────────────────────── 크레덴셜 파일 ───────────────────────────
//
// 브라우저에서도 익스텐션에서도 못 가져오는 환경의 **최후의 수단**. `inno-creed auth set`이
// 쓰고 여기서 읽는다. **MCP 도구로는 건드리지 않는다**(`config.rs`의 규약: 쓰기는 그 파일
// 담당 모듈만).

const CREDS_FILE: &str = "creds.json";

#[derive(serde::Serialize, serde::Deserialize)]
struct StoredCreds {
    auth_token: String,
    sign_key: String,
}

/// 크레덴셜 파일 경로(`~/.config/inno-creed/creds.json`).
pub fn creds_file_path() -> Option<PathBuf> {
    crate::config::file(CREDS_FILE)
}

fn try_file() -> std::result::Result<Creds, SourceFail> {
    let Some(path) = creds_file_path() else {
        return Err(SourceFail::absent("설정 디렉토리를 정할 수 없음(HOME 없음)"));
    };
    read_creds_file(&path)
}

/// 경로를 받아 읽는다. **경로 결정과 분리한 이유**는 테스트가 사용자의 진짜
/// `~/.config/inno-creed/creds.json`을 건드리지 않게 하기 위해서다.
fn read_creds_file(path: &Path) -> std::result::Result<Creds, SourceFail> {
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(SourceFail::absent(format!("없음: {}", path.display())));
        }
        Err(e) => {
            return Err(SourceFail::failed(format!(
                "크레덴셜 파일을 읽지 못했습니다({}): {e}",
                path.display()
            )));
        }
    };
    // 파일이 **있는데** 못 쓰는 것은 처방 대상이다 — 손으로 고치다 깨뜨린 경우가 대부분.
    let stored: StoredCreds = serde_json::from_str(&raw).map_err(|e| {
        SourceFail::failed(format!(
            "크레덴셜 파일 형식 오류({}): {e}. `inno-creed auth set`으로 다시 저장하세요.",
            path.display()
        ))
    })?;
    if stored.auth_token.trim().is_empty() || stored.sign_key.trim().is_empty() {
        return Err(SourceFail::failed(format!(
            "크레덴셜 파일({})의 auth_token/sign_key가 비어 있습니다. `inno-creed auth set`으로 다시 저장하세요.",
            path.display()
        )));
    }
    Ok(Creds {
        auth_token: url_decode(&stored.auth_token),
        sign_key: stored.sign_key,
    })
}

/// 크레덴셜 파일 저장. 소유자만 읽도록 권한을 좁힌다(unix).
///
/// ⚠️ **Windows에는 등가 수단이 없다** — ACL까지 다루는 것은 이 도구의 몫이 아니라고 보고
/// 하지 않는다. 그쪽에서는 평문 파일이 홈 디렉토리 권한에만 기댄다.
pub fn save_creds_file(auth_token: &str, sign_key: &str) -> Result<PathBuf> {
    let dir = crate::config::ensure_dir()?;
    write_creds_file(&dir.join(CREDS_FILE), auth_token, sign_key)
}

fn write_creds_file(path: &Path, auth_token: &str, sign_key: &str) -> Result<PathBuf> {
    let body = serde_json::to_string_pretty(&StoredCreds {
        auth_token: auth_token.trim().to_string(),
        sign_key: sign_key.trim().to_string(),
    })?;
    std::fs::write(path, body)
        .with_context(|| format!("크레덴셜 파일 쓰기 실패: {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("권한(0600) 설정 실패: {}", path.display()))?;
    }
    Ok(path.to_path_buf())
}

/// 크레덴셜 파일 삭제. 이미 없으면 `false`.
///
/// **이 명령이 있는 이유**: 파일 소스는 브라우저보다 아래라 멀쩡한 세션을 가리지는 않지만,
/// 낡은 파일이 남아 있으면 "브라우저도 실패했는데 왜 옛 토큰으로 401만 나오는지" 헷갈린다.
pub fn clear_creds_file() -> Result<bool> {
    let Some(path) = creds_file_path() else {
        return Ok(false);
    };
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).with_context(|| format!("크레덴셜 파일 삭제 실패: {}", path.display())),
    }
}

/// 환경변수가 설정되어 있고 비어있지 않으면 그 값을 반환.
fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

/// authToken은 `%7C`(=`|`)로 URL 인코딩되어 저장됨.
fn url_decode(s: &str) -> String {
    s.replace("%7C", "|").replace("%7c", "|")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 쿠키 DB 대역. 내용이 잘렸는지 확인할 수 있게 충분히 크게 만든다.
    /// 테스트가 **패닉해도** 남지 않도록 `Drop`으로 지운다 — 임시 디렉토리를 어지르지 않는 것이
    /// 이 파일의 주제이니 테스트도 같은 규율을 지킨다.
    struct Src(PathBuf);

    impl Src {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir()
                .join(format!("inno_creed_test_src_{}_{name}", std::process::id()));
            std::fs::write(&p, vec![b'x'; 256 * 1024]).unwrap();
            Self(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Src {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    /// 테스트 전용 임시 크레덴셜 파일 경로. **사용자의 진짜
    /// `~/.config/inno-creed/creds.json`을 절대 건드리지 않는다** — 그래서 경로 결정과
    /// 읽기/쓰기를 분리해 두었다.
    struct TmpCreds(PathBuf);

    impl TmpCreds {
        fn new(name: &str) -> Self {
            Self(std::env::temp_dir().join(format!(
                "inno_creed_test_creds_{}_{name}.json",
                std::process::id()
            )))
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TmpCreds {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn 크레덴셜_파일은_저장한_값을_그대로_돌려준다() {
        let f = TmpCreds::new("roundtrip");
        // authToken은 브라우저에 URL 인코딩(`%7C`)으로 들어 있다 — 붙여넣은 그대로 저장해도
        // 읽을 때 디코드돼야 env 경로와 결과가 같다.
        write_creds_file(f.path(), "abc%7Cdef", "hk-value").unwrap();
        let c = read_creds_file(f.path()).expect("저장한 파일을 읽지 못했다");
        assert_eq!(c.auth_token, "abc|def", "%7C가 디코드돼야 한다");
        assert_eq!(c.sign_key, "hk-value");
    }

    #[test]
    fn 크레덴셜_파일이_없으면_처방_대상이_아니다() {
        let f = TmpCreds::new("absent");
        let fail = read_creds_file(f.path())
            .err()
            .expect("없는 파일은 실패해야 한다");
        assert!(
            fail.absent,
            "파일 없음은 정상 상태다 — Failed로 올리면 최종 에러에서 진짜 문제를 가린다"
        );
    }

    #[test]
    fn 깨진_크레덴셜_파일은_처방_대상이다() {
        let f = TmpCreds::new("broken");
        std::fs::write(f.path(), "{ 이건 JSON이 아니다").unwrap();
        let fail = read_creds_file(f.path())
            .err()
            .expect("깨진 파일은 실패해야 한다");
        assert!(!fail.absent, "파일이 있는데 못 쓰는 것은 사용자가 손댈 일이다");
        assert!(fail.msg.contains("auth set"), "다시 저장하는 법을 알려줘야 한다");
    }

    #[test]
    fn 빈_값이_든_크레덴셜_파일은_처방_대상이다() {
        let f = TmpCreds::new("empty");
        std::fs::write(f.path(), r#"{"auth_token":"","sign_key":"hk"}"#).unwrap();
        let fail = read_creds_file(f.path())
            .err()
            .expect("빈 값은 실패해야 한다");
        assert!(!fail.absent);
    }

    /// 최종 에러의 요점은 **하나뿐인 진짜 문제를 보이게 하는 것**이다. 예전에는 Firefox
    /// 미설치(`os error 3`)가 Chrome 실패와 나란히 찍혀 두 번째 문제처럼 읽혔다.
    #[test]
    fn 최종_에러는_처방없는_실패를_강등한다() {
        let msg = render_failure(&[
            SourceReport {
                source: "환경변수",
                outcome: Outcome::Absent("미설정".into()),
            },
            SourceReport {
                source: "Chrome",
                outcome: Outcome::Failed("Chrome을 종료하세요".into()),
            },
            SourceReport {
                source: "Firefox",
                outcome: Outcome::Absent("Firefox 미설치".into()),
            },
        ]);
        let chrome_line = msg.find("Chrome을 종료하세요").expect("처방은 본문에 나와야 한다");
        let absent_line = msg.find("대응 불필요").expect("강등 줄이 있어야 한다");
        assert!(chrome_line < absent_line, "처방이 잡음보다 먼저 와야 한다");
        assert!(
            !msg.contains("Firefox 미설치"),
            "강등된 항목의 상세는 본문에 늘어놓지 않는다 — 이름만 남긴다"
        );
    }

    #[test]
    fn 소스가_하나도_없으면_다음_행동을_안내한다() {
        let msg = render_failure(&[
            SourceReport {
                source: "환경변수",
                outcome: Outcome::Absent("미설정".into()),
            },
            SourceReport {
                source: "Chrome",
                outcome: Outcome::Absent("Chrome 미설치".into()),
            },
        ]);
        // 처방이 하나도 없어도 다음 행동은 줘야 한다. Windows는 익스텐션 설치가, 그 외는
        // 브라우저 로그인이 그 행동이다.
        let has_next_step = msg.contains("로그인") || msg.contains("install-extension-host");
        assert!(has_next_step, "처방이 없으면 다음 행동을 줘야 한다:\n{msg}");
    }

    /// Windows 공유 위반(32)은 원인이 하나뿐이라 처방을 단정한다. 예전에는 OS 원문과 경로만
    /// 올려보내서, 정작 할 일("브라우저 완전 종료")은 설치 문서에만 있었다.
    #[test]
    fn 잠긴_쿠키db는_종료_처방을_준다() {
        let e = std::io::Error::from_raw_os_error(32);
        let msg = format!("{:#}", locked_db_error("Chrome", Path::new("/x/Cookies"), e));
        assert!(msg.contains("완전히"), "종료하라는 처방이 있어야 한다");
        assert!(
            msg.contains("install-extension-host") || msg.contains("auth set"),
            "종료하지 않고 쓰는 우회로도 알려줘야 한다"
        );
    }

    #[test]
    fn 잠금이_아닌_io오류는_원문을_보존한다() {
        let e = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "권한 없음");
        let msg = format!("{:#}", locked_db_error("Chrome", Path::new("/x/Cookies"), e));
        assert!(msg.contains("권한 없음"), "원인을 지어내지 말고 원문을 남겨야 한다");
        assert!(!msg.contains("완전히"), "잠금이 아닌데 종료 처방을 주면 오진이다");
    }

    /// 원인을 하나로 단정하면 또 오진이 된다 — 세션 쿠키와 로그아웃이 같은 모습으로 보인다.
    #[test]
    fn at누락_문구는_두_원인을_모두_적는다() {
        let msg = missing_at_msg("/x/Cookies");
        assert!(msg.contains("세션 쿠키"), "디스크에 안 남는 경우");
        assert!(msg.contains("로그아웃"), "지워진 경우");
    }

    /// 소스 순서는 **문서가 아니라 코드가 정본**이다 — `doctor` 출력과 README가 이 순서를
    /// 그대로 적으므로, 순서를 바꾸면 여기서 먼저 걸려야 한다.
    ///
    /// 특히 **크레덴셜 파일이 마지막**인 것이 핵심이다. 위로 올리면 만료된 `creds.json`
    /// 하나가 멀쩡한 브라우저 세션을 영영 가린다(`from_browser` 주석).
    #[test]
    fn 소스_순서는_환경변수부터_크레덴셜파일까지다() {
        let names: Vec<&str> = sources().into_iter().map(|(n, _)| n).collect();
        assert_eq!(names.first(), Some(&"환경변수"), "env가 늘 최우선이다");
        assert_eq!(
            names.last(),
            Some(&"크레덴셜 파일"),
            "파일은 반드시 브라우저보다 아래여야 한다"
        );
        let idx = |n: &str| names.iter().position(|x| *x == n);
        assert!(
            idx("익스텐션 캐시") < idx("Chrome"),
            "익스텐션이 쿠키 DB 직독보다 먼저다(v20·잠금·세션쿠키 문제가 없는 경로)"
        );
        #[cfg(target_os = "windows")]
        assert!(idx("Edge").is_some(), "Windows에서는 Edge도 시도한다");
        #[cfg(not(target_os = "windows"))]
        assert!(idx("Edge").is_none(), "비-Windows에서 Edge를 볼 이유가 없다");
    }

    #[test]
    fn temp_copy는_drop되면_파일을_지운다() {
        let src = Src::new("drop");
        let path = {
            let tmp = TempCopy::new(src.path(), "ck").unwrap();
            let path = tmp.path().to_path_buf();
            assert!(path.exists(), "복사 직후에는 있어야 한다");
            path
        };
        assert!(!path.exists(), "Drop이 지워야 한다 — 고유 이름은 덮어쓰기로 회수되지 않는다");
    }

    /// 실패 경로에서도 지워지는지 — `?`로 조기 반환해도 `Drop`은 돈다.
    #[test]
    fn temp_copy는_조기반환에도_남지_않는다() {
        let src = Src::new("early");
        let leaked = std::cell::RefCell::new(PathBuf::new());
        let f = || -> Result<()> {
            let tmp = TempCopy::new(src.path(), "ck")?;
            *leaked.borrow_mut() = tmp.path().to_path_buf();
            bail!("중간에 실패한 셈 치자");
        };
        assert!(f().is_err());
        assert!(!leaked.borrow().exists(), "실패해도 임시파일이 남으면 안 된다");
    }

    /// **경합 방어의 핵심 단언.** 동시에 복사본을 만들면 경로가 전부 달라야 한다.
    ///
    /// 브라우저별 고정 이름 하나이던 시절에는 N개가 같은 경로를 가리켰다 —
    /// `fs::copy`가 대상을 truncate하므로 서로의 복사 한복판을 읽거나, 남의 `remove_file`
    /// 뒤에 열어 "파일 없음"을 만났다. 경로가 전부 다르면 그 창 자체가 없다.
    /// (고정 이름으로 되돌리면 `unique.len()`이 1이 되어 이 테스트가 즉시 깨진다.)
    #[test]
    fn 동시_복사본은_서로_다른_경로를_쓰고_내용이_온전하다() {
        const N: usize = 8;
        let src = std::sync::Arc::new(Src::new("race"));
        let expected = std::fs::metadata(src.path()).unwrap().len();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(N));

        let handles: Vec<_> = (0..N)
            .map(|_| {
                let (src, barrier) = (src.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait(); // 진짜로 동시에 출발시킨다
                    let tmp = TempCopy::new(src.path(), "ck").expect("복사 실패");
                    // 남이 truncate한 것을 읽으면 길이가 어긋난다.
                    let len = std::fs::metadata(tmp.path()).unwrap().len();
                    (tmp.path().to_path_buf(), len)
                })
            })
            .collect();

        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let unique: std::collections::HashSet<_> = results.iter().map(|(p, _)| p.clone()).collect();
        assert_eq!(unique.len(), N, "동시 복사본이 같은 경로를 재사용했다 — 경합 창이 열려 있다");
        for (p, len) in &results {
            assert_eq!(*len, expected, "{p:?} 내용이 잘렸다");
            assert!(!p.exists(), "스레드 종료 시 Drop이 지웠어야 한다");
        }
    }

    /// Chrome/Firefox 복사본이 서로 다른 이름 공간을 쓰는지(둘이 겹치면 같은 경합이 돌아온다).
    #[test]
    fn 크롬과_파이어폭스_복사본은_이름이_겹치지_않는다() {
        let src = Src::new("tag");
        let ck = TempCopy::new(src.path(), "ck").unwrap();
        let ff = TempCopy::new(src.path(), "ff").unwrap();
        assert_ne!(ck.path(), ff.path());
        let name = |t: &TempCopy| t.path().file_name().unwrap().to_string_lossy().into_owned();
        assert!(name(&ck).contains("_ck_"), "{}", name(&ck));
        assert!(name(&ff).contains("_ff_"), "{}", name(&ff));
        // pid가 들어가야 다른 프로세스와도 갈린다.
        assert!(name(&ck).contains(&std::process::id().to_string()));
    }
}
