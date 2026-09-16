// src/background.ts
var HOST_NAME = "com.innogrid.inno_creed";
var DOMAIN = "gw.innogrid.com";
var COOKIE_NAMES = ["BIZCUBE_AT", "BIZCUBE_HK"];
var syncTimer;
function matchesDomain(domain) {
  const bare = domain.startsWith(".") ? domain.slice(1) : domain;
  return bare === DOMAIN;
}
function sendNative(message) {
  return new Promise((resolve) => {
    const p = chrome.runtime.connectNative(HOST_NAME);
    let responded = false;
    p.onMessage.addListener((response) => {
      responded = true;
      if (!response?.ok) {
        console.warn("[inno-creed] native host 처리 실패:", response?.error);
      }
      resolve(response?.ok === true);
    });
    p.onDisconnect.addListener(() => {
      const err = chrome.runtime.lastError;
      if (!responded) {
        console.warn("[inno-creed] native host 연결 실패:", err?.message);
        resolve(false);
      }
    });
    try {
      p.postMessage(message);
    } catch (e) {
      console.warn("[inno-creed] native host 전송 실패:", e);
      resolve(false);
    }
  });
}
var RETRY_ALARM = "inno-creed-retry";
var RETRY_PERIOD_MINUTES = 1;
async function armRetry() {
  if (await chrome.alarms.get(RETRY_ALARM))
    return;
  await chrome.alarms.create(RETRY_ALARM, { periodInMinutes: RETRY_PERIOD_MINUTES });
  console.log(`[inno-creed] ${RETRY_PERIOD_MINUTES}분마다 다시 시도합니다.`);
}
async function disarmRetry() {
  await chrome.alarms.clear(RETRY_ALARM);
}
async function syncCookies() {
  const cookies = await chrome.cookies.getAll({ domain: DOMAIN });
  const authTokenCookie = cookies.find((c) => c.name === "BIZCUBE_AT");
  const signKeyCookie = cookies.find((c) => c.name === "BIZCUBE_HK");
  if (!authTokenCookie || !signKeyCookie) {
    console.log(`[inno-creed] ${DOMAIN} 쿠키 ${cookies.length}개 중 BIZCUBE_AT/HK 없음 — 미로그인 상태로 보임`);
    await disarmRetry();
    return;
  }
  console.log("[inno-creed] BIZCUBE_AT/HK 발견, native host로 전달");
  const ok = await sendNative({ authToken: authTokenCookie.value, signKey: signKeyCookie.value });
  if (ok) {
    await disarmRetry();
    return;
  }
  console.warn("[inno-creed] 전달에 실패했습니다 — inno-creed가 아직 설치되지 않았거나 native host 등록이 이 브라우저 자리에 없을 수 있습니다.");
  await armRetry();
}
function scheduleSync() {
  if (syncTimer)
    clearTimeout(syncTimer);
  syncTimer = setTimeout(syncCookies, 500);
}
chrome.cookies.onChanged.addListener((changeInfo) => {
  const c = changeInfo.cookie;
  if (!matchesDomain(c.domain) || !COOKIE_NAMES.includes(c.name))
    return;
  if (changeInfo.removed) {
    sendNative({ clear: true });
  } else {
    scheduleSync();
  }
});
chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === RETRY_ALARM)
    scheduleSync();
});
scheduleSync();
