const LANG = {
  en: {
    statusActive: "Active",
    permText: "Twitter/X access required to display scores.",
    permBtn: "Grant Access",
    tabDisplay: "Display",
    tabScanner: "Scanner",
    toggleTS: "TweetScout",
    toggleSorsa: "Sorsa",
    toggleFlw: "Followers",
    scannerDesc: "Auto-scroll through follower list, collect scores, then export to CSV.",
    scrollDown: "Scroll Down",
    scrollUp: "Scroll Up",
    btnScan: "Scan & Export",
    btnPause: "Pause",
    btnResume: "Resume",
    btnStop: "Stop"
  },
  vi: {
    statusActive: "Đang bật",
    permText: "Cần quyền truy cập Twitter/X để hiển thị điểm.",
    permBtn: "Cấp quyền",
    tabDisplay: "Hiển thị",
    tabScanner: "Máy quét",
    toggleTS: "TweetScout",
    toggleSorsa: "Sorsa",
    toggleFlw: "Followers",
    scannerDesc: "Tự động cuộn qua danh sách người theo dõi, thu thập điểm số, xuất ra file CSV.",
    scrollDown: "Cuộn xuống",
    scrollUp: "Cuộn lên",
    btnScan: "Quét & Xuất",
    btnPause: "Tạm dừng",
    btnResume: "Tiếp tục",
    btnStop: "Dừng"
  }
};

let currentLang = "vi";

document.addEventListener("DOMContentLoaded", () => {
  const displayKeys = ["showTS", "showSorsa", "showFollowers"];

  chrome.storage.local.get(["lang", ...displayKeys], stored => {
    currentLang = stored.lang || "vi";
    applyLang(currentLang);
    displayKeys.forEach(k => {
      const el = document.getElementById(k);
      if (el) el.checked = stored[k] !== false;
    });
  });

  function applyLang(lang) {
    currentLang = lang;
    const t = LANG[lang] || LANG.vi;
    document.querySelectorAll("[data-i18n]").forEach(el => {
      const key = el.getAttribute("data-i18n");
      if (t[key]) el.textContent = t[key];
    });
    document.querySelectorAll("[data-i18n-opt]").forEach(el => {
      const key = el.getAttribute("data-i18n-opt");
      if (t[key]) el.textContent = t[key];
    });
    document.querySelectorAll(".lang-btn").forEach(btn => {
      btn.classList.toggle("active", btn.dataset.lang === lang);
    });
  }

  document.querySelectorAll(".lang-btn").forEach(btn => {
    btn.addEventListener("click", () => {
      const lang = btn.dataset.lang;
      applyLang(lang);
      chrome.storage.local.set({ lang });
    });
  });

  document.querySelectorAll(".tab").forEach(tab => {
    tab.addEventListener("click", () => {
      document.querySelectorAll(".tab").forEach(t => t.classList.remove("active"));
      document.querySelectorAll(".tab-panel").forEach(p => p.classList.remove("active"));
      tab.classList.add("active");
      document.getElementById("tab-" + tab.dataset.tab).classList.add("active");
    });
  });

  async function registerContentScript() {
    try {
      const scripts = await chrome.scripting.getRegisteredContentScripts();
      if (!scripts.some(s => s.id === "scorelens")) {
        await chrome.scripting.registerContentScripts([{
          id: "scorelens",
          js: ["content.js"],
          css: ["styles.css"],
          matches: ["*://*.twitter.com/*", "*://*.x.com/*"],
          runAt: "document_idle"
        }]);
      }
    } catch (e) { console.error("Registration failed:", e); }
  }

  const permBanner = document.getElementById("permissionBanner");
  const grantBtn = document.getElementById("grantPermissionBtn");
  if (grantBtn) {
    grantBtn.addEventListener("click", () => {
      chrome.permissions.request({
        origins: ["*://*.twitter.com/*", "*://*.x.com/*", "https://app.sorsa.io/*", "https://api.sorsa.io/*", "https://twitterscore.io/*"]
      }, granted => {
        if (granted) {
          permBanner.style.display = "none";
          registerContentScript();
          chrome.tabs.query({ url: ["*://*.twitter.com/*", "*://*.x.com/*"] }, tabs => {
            tabs.forEach(t => chrome.tabs.reload(t.id));
          });
        }
      });
    });
  }

  chrome.permissions.contains({ origins: ["*://*.twitter.com/*", "*://*.x.com/*"] }, has => {
    permBanner.style.display = has ? "none" : "flex";
    if (has) registerContentScript();
  });

  chrome.storage.local.set({ isEnabled: true });

  function refreshBadges() {
    chrome.tabs.query({ active: true, currentWindow: true }, tabs => {
      if (tabs[0]) chrome.tabs.sendMessage(tabs[0].id, { type: "REFRESH_BADGES" });
    });
  }

  displayKeys.forEach(k => {
    const el = document.getElementById(k);
    if (el) el.addEventListener("change", () => {
      chrome.storage.local.set({ [k]: el.checked }, () => refreshBadges());
    });
  });

  const startBtn = document.getElementById("startBtn");
  const pauseBtn = document.getElementById("pauseBtn");
  const resumeBtn = document.getElementById("resumeBtn");
  const stopBtn = document.getElementById("stopBtn");
  const activeControls = document.getElementById("activeControls");
  const liveStatus = document.getElementById("liveStatus");

  function setUI(state, text) {
    if (text) liveStatus.textContent = text;
    if (state === "SCROLLING") {
      startBtn.style.display = "none";
      activeControls.style.display = "flex";
      pauseBtn.style.display = "inline-block";
      resumeBtn.style.display = "none";
    } else if (state === "PAUSED") {
      startBtn.style.display = "none";
      activeControls.style.display = "flex";
      pauseBtn.style.display = "none";
      resumeBtn.style.display = "inline-block";
    } else {
      startBtn.style.display = "block";
      activeControls.style.display = "none";
      liveStatus.textContent = "";
    }
  }

  function pollStatus() {
    chrome.tabs.query({ active: true, currentWindow: true }, tabs => {
      if (tabs[0]) {
        chrome.tabs.sendMessage(tabs[0].id, { type: "GET_STATUS" }, res => {
          if (!chrome.runtime.lastError && res?.status) setUI(res.status, res.text);
        });
      }
    });
  }

  function sendCmd(type, data = {}) {
    chrome.tabs.query({ active: true, currentWindow: true }, tabs => {
      if (tabs[0]) chrome.tabs.sendMessage(tabs[0].id, { type, ...data });
    });
  }

  pollStatus();
  setInterval(pollStatus, 1000);

  if (startBtn) startBtn.addEventListener("click", () => {
    sendCmd("AUTO_SCROLL_EXPORT", {
      scrollMult: 3, waitMin: 2, waitMax: 5,
      scrollDir: parseInt(document.getElementById("scrollDir").value, 10) || 1
    });
    setUI("SCROLLING");
  });
  if (pauseBtn) pauseBtn.addEventListener("click", () => { sendCmd("PAUSE"); setUI("PAUSED"); });
  if (resumeBtn) resumeBtn.addEventListener("click", () => { sendCmd("RESUME"); setUI("SCROLLING"); });
  if (stopBtn) stopBtn.addEventListener("click", () => { sendCmd("STOP"); setUI("IDLE"); });
});