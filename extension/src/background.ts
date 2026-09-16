// inno-creed 크레덴셜 브릿지 — gw.innogrid.com의 BIZCUBE_AT/BIZCUBE_HK 쿠키를
// Native Messaging으로 로컬 inno-creed 프로세스에 전달한다.
//
// 왜 이 방식인가: 쿠키 DB를 직접 읽는 길은 어느 OS에서도 보장되지 않는다 — Windows
// Chrome/Edge의 app-bound(v20) 암호화는 호출자 프로세스 경로를 검증해 inno-creed
// (제3자 프로세스)를 설계상 항상 막고, macOS·Linux도 세션 쿠키가 디스크에 없거나
// 키체인·키링 접근이 막히면 그대로 실패한다. 그래서 이 브릿지가 전 OS 정식 경로다.
// 익스텐션은 브라우저가 공식으로 열어준 `cookies` API로 평문 값을
// 바로 받으므로 이 문제 자체가 없다. `cookies` API는 리스닝 소켓을 못 열기 때문에,
// 값 전달은 브라우저가 로컬 실행파일을 직접 스폰해 stdio로 이어주는 Native Messaging을
// 쓴다(소켓/서버 불필요).

const HOST_NAME = "com.innogrid.inno_creed";
const DOMAIN = "gw.innogrid.com";
const COOKIE_NAMES = ["BIZCUBE_AT", "BIZCUBE_HK"] as const;

let syncTimer: ReturnType<typeof setTimeout> | undefined;

function matchesDomain(domain: string): boolean {
  const bare = domain.startsWith(".") ? domain.slice(1) : domain;
  return bare === DOMAIN;
}

// `sendNativeMessage`(1회성)는 MV3 서비스워커가 native host 프로세스 스폰 도중(서명 안 된
// exe라 Defender 스캔 등으로 느려질 수 있음) 유휴 종료되면 응답 콜백이 통째로 유실되는 걸
// 실측으로 확인했다. `connectNative`(지속 포트)는 크롬이 "열린 포트"를 살아있는 작업으로
// 취급해 이 경합이 훨씬 덜하다.
//
// 우리 쪽 native host(`native_host::run`)는 메시지 하나 처리하면 바로 종료하는 1회성
// 프로세스라 포트를 재사용할 이유가 없다 — 보낼 때마다 새로 연결(=새 프로세스 스폰)한다.
// 응답을 받은 뒤 포트가 "Native host has exited."로 끊기는 건 정상 종료이지, 실패가
// 아니다. 응답 **전**에 끊기는 경우만 진짜 실패(호스트 미설치·크래시 등)로 본다.
function sendNative(message: Record<string, unknown>): void {
  const p = chrome.runtime.connectNative(HOST_NAME);
  let responded = false;
  p.onMessage.addListener((response) => {
    responded = true;
    if (!response?.ok) {
      console.warn("[inno-creed] native host 처리 실패:", response?.error);
    }
  });
  p.onDisconnect.addListener(() => {
    // lastError는 콜백 안에서 한 번이라도 읽어야 크롬이 "확인됨"으로 처리한다 — 안 읽으면
    // (응답을 이미 받아 무시하는 경로에서도) 크롬이 "Unchecked runtime.lastError"를 별도로
    // 콘솔에 찍는다. 그래서 `responded`와 무관하게 항상 읽는다.
    const err = chrome.runtime.lastError;
    if (!responded) {
      console.warn("[inno-creed] native host 연결 실패:", err?.message);
    }
  });
  try {
    p.postMessage(message);
  } catch (e) {
    console.warn("[inno-creed] native host 전송 실패:", e);
  }
}

async function syncCookies(): Promise<void> {
  // `cookies.get({url, name})`는 URL의 path까지 쿠키의 Path 속성과 맞아야 매칭된다 —
  // BIZCUBE 쿠키의 Path가 "/"가 아니면 조용히 못 찾는다. domain 기준 `getAll`은 Path를
  // 안 따지므로 이 문제가 없다.
  const cookies = await chrome.cookies.getAll({ domain: DOMAIN });
  const authTokenCookie = cookies.find((c) => c.name === "BIZCUBE_AT");
  const signKeyCookie = cookies.find((c) => c.name === "BIZCUBE_HK");
  if (!authTokenCookie || !signKeyCookie) {
    console.log(`[inno-creed] ${DOMAIN} 쿠키 ${cookies.length}개 중 BIZCUBE_AT/HK 없음 — 미로그인 상태로 보임`);
    return;
  }
  console.log("[inno-creed] BIZCUBE_AT/HK 발견, native host로 전달");
  sendNative({ authToken: authTokenCookie.value, signKey: signKeyCookie.value });
}

function scheduleSync(): void {
  // 로그인 시 두 쿠키가 짧은 시간 안에 연속으로 세팅되므로 디바운스해서 한 번만 보낸다.
  if (syncTimer) clearTimeout(syncTimer);
  syncTimer = setTimeout(syncCookies, 500);
}

chrome.cookies.onChanged.addListener((changeInfo) => {
  const c = changeInfo.cookie;
  if (!matchesDomain(c.domain) || !(COOKIE_NAMES as readonly string[]).includes(c.name)) return;
  if (changeInfo.removed) {
    // 로그아웃/만료로 쿠키가 지워지면 캐시도 지운다 — 남겨두면 만료된 값으로
    // inno-creed가 계속 "성공"해서 나중에 API 401로 더 헷갈리는 실패가 난다.
    sendNative({ clear: true });
  } else {
    scheduleSync();
  }
});

// 익스텐션이 막 로드된 시점에 이미 로그인돼 있을 수 있으니 즉시 한 번 동기화.
syncCookies();
