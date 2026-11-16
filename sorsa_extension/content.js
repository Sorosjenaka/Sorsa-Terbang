const scoreCache = new Map();

const COLOR_KEYS = ['red_enabled', 'yellow_enabled', 'green_enabled', 'red_sorsaMin', 'red_sorsaMax', 'yellow_sorsaMin', 'yellow_sorsaMax', 'green_sorsaMin', 'green_sorsaMax'];
const COLOR_DEFAULTS = {
  red_enabled: true, yellow_enabled: true, green_enabled: true,
  red_sorsaMin: 0, red_sorsaMax: 200,
  yellow_sorsaMin: 200, yellow_sorsaMax: 600,
  green_sorsaMin: 600, green_sorsaMax: 9999
};

let showTS = true, showSorsa = true, showFollowers = true;
let colorSettings = { ...COLOR_DEFAULTS };

chrome.storage.local.get(['showTS', 'showSorsa', 'showFollowers', ...COLOR_KEYS], s => {
  showTS = s.showTS !== false;
  showSorsa = s.showSorsa !== false;
  showFollowers = s.showFollowers !== false;
  COLOR_KEYS.forEach(k => {
    if (s[k] != null) colorSettings[k] = typeof COLOR_DEFAULTS[k] === 'number' ? parseFloat(s[k]) || COLOR_DEFAULTS[k] : s[k];
  });
});

const parseCount = v => {
  if (!v) return 0;
  const b = v.toUpperCase().replace(/,/g, '').trim();
  let m = 1, n = b;
  if (b.endsWith('K')) { m = 1000; n = b.slice(0, -1); }
  else if (b.endsWith('M')) { m = 1000000; n = b.slice(0, -1); }
  return Math.floor(parseFloat(n) * m);
};

const formatFlw = v => {
  const n = parseInt(v, 10);
  if (isNaN(n) || n <= 0) return null;
  if (n >= 1000000) return (n / 1000000).toFixed(1).replace(/\.0$/, '') + 'M';
  if (n >= 1000) return (n / 1000).toFixed(1).replace(/\.0$/, '') + 'K';
  return String(n);
};

function getXFollowers(handle) {
  const links = document.querySelectorAll(`a[href="/${handle}/followers"], a[href="/${handle}/verified_followers"]`);
  for (const link of links) {
    const txt = link.textContent.trim();
    const m = txt.match(/([\d.,]+)\s*(?:Followers|Người theo dõi|followers)/i);
    if (m) {
      let raw = m[1].replace(/\./g, '').replace(/,/g, '');
      const val = parseInt(raw, 10);
      if (val > 0) return String(val);
    }
  }
  return null;
}

let processTimer = null;
function scheduleProcess() {
  if (processTimer) return;
  processTimer = setTimeout(() => { processTimer = null; processElements(); }, 300);
}

function processElements() {
  document.querySelectorAll('[data-testid="User-Name"], [data-testid="UserCell"]').forEach(el => {
    if (el.dataset.scoreProcessed) return;
    const type = el.getAttribute("data-testid") === "User-Name" ? "tweet" : "cell";
    const link = Array.from(el.querySelectorAll("a")).find(a => {
      const href = a.getAttribute("href");
      if (!href) return false;
      return href.replace("https://x.com", "").replace("https://twitter.com", "").match(/^\/[a-zA-Z0-9_]+$/);
    });
    if (link) {
      let handle = link.getAttribute("href").replace("https://x.com", "").replace("https://twitter.com", "");
      if (handle.startsWith("/")) handle = handle.substring(1);
      handle = handle.split(/[?\/]/)[0];
      if (handle && !["home","explore","notifications","messages","i","compose","search"].includes(handle)) {
        injectScore(el, handle, type);
      }
    }
  });
  const header = document.querySelector('[data-testid="UserProfileHeader_Items"]');
  if (header && !header.dataset.scoreProcessed) {
    const handle = window.location.pathname.split("/")[1];
    if (handle && !["home","explore","notifications","messages"].includes(handle)) {
      injectScore(header, handle, "profile");
    }
  }
}

function injectScore(el, handle, type) {
  if (el.dataset.scoreProcessed) return;
  if (el.querySelector(".score-badge") || el.dataset.badgePending) {
    el.dataset.scoreProcessed = "true";
    return;
  }
  el.dataset.badgePending = "true";
  if (scoreCache.has(handle)) {
    const cached = scoreCache.get(handle);
    if (cached instanceof Promise) {
      cached.then(data => { if (data) appendBadge(el, data, type); else { delete el.dataset.badgePending; } });
    } else if (cached) {
      appendBadge(el, cached, type);
    } else { delete el.dataset.badgePending; }
    return;
  }
  const promise = new Promise(resolve => {
    let sorsaRes = null, tsRes = null, pending = 2;
    function done() {
      if (--pending > 0) return;
      const data = {
        handle,
        url: sorsaRes?.url || tsRes?.url || `https://x.com/${handle}`,
        sorsaScore: sorsaRes?.score || null,
        followers: sorsaRes?.followers || null,
        tsScore: tsRes?.score || null
      };
      resolve(data.sorsaScore || data.tsScore || data.followers ? data : null);
    }
    chrome.runtime.sendMessage({ type: "GET_SCORE", handle }, res => {
      if (chrome.runtime.lastError) console.warn("Sorsa:", chrome.runtime.lastError);
      sorsaRes = res;
      done();
    });
    chrome.runtime.sendMessage({ type: "GET_TWITTERSCORE", handle }, res => {
      if (chrome.runtime.lastError) console.warn("TS:", chrome.runtime.lastError);
      tsRes = res;
      done();
    });
  });
  scoreCache.set(handle, promise);
  promise.then(data => {
    scoreCache.set(handle, data);
    if (data) {
      const xFlw = getXFollowers(handle);
      if (xFlw) data.followers = xFlw;
      appendBadge(el, data, type);
    }
    else delete el.dataset.badgePending;
  });
}

function appendBadge(el, data, type) {
  if (el.querySelector(".score-badge")) {
    el.dataset.scoreProcessed = "true";
    delete el.dataset.badgePending;
    return;
  }
  const badge = createBadge(data);
  if (type === "tweet") {
    const timeEl = el.querySelector("time");
    if (timeEl?.parentElement?.parentElement) timeEl.parentElement.parentElement.appendChild(badge);
    else el.appendChild(badge);
  } else if (type === "cell") {
    const ltr = el.querySelector('[dir="ltr"]');
    if (ltr?.parentElement) ltr.parentElement.appendChild(badge);
    else el.appendChild(badge);
  } else if (type === "profile") {
    el.prepend(badge);
  }
  el.dataset.scoreProcessed = "true";
  delete el.dataset.badgePending;
}

function createBadge(data) {
  const a = document.createElement("a");
  a.href = data.url;
  a.target = "_blank";
  a.className = "score-badge";
  if (data.handle) a.dataset.handle = data.handle;

  const parts = [];
  if (showSorsa && data.sorsaScore) parts.push(`S:${data.sorsaScore}`);
  if (showTS && data.tsScore) parts.push(`T:${data.tsScore}`);
  if (showFollowers && data.followers) parts.push(`Flw:${formatFlw(data.followers)}`);
  a.textContent = parts.join(" | ");
  a.onclick = e => e.stopPropagation();

  const sorsaVal = parseFloat(data.sorsaScore) || 0;
  const hasSorsa = !!data.sorsaScore;

  function inRange(val, min, max) { return val >= min && val < max; }

  if (colorSettings.green_enabled !== false && hasSorsa && inRange(sorsaVal, colorSettings.green_sorsaMin, colorSettings.green_sorsaMax)) {
    a.classList.add("high");
  } else if (colorSettings.yellow_enabled !== false && hasSorsa && inRange(sorsaVal, colorSettings.yellow_sorsaMin, colorSettings.yellow_sorsaMax)) {
    a.classList.add("medium");
  } else if (colorSettings.red_enabled !== false && hasSorsa && inRange(sorsaVal, colorSettings.red_sorsaMin, colorSettings.red_sorsaMax)) {
    a.classList.add("low");
  } else if (data.sorsaScore || data.tsScore) {
    a.classList.add("medium");
  }

  a.style.marginLeft = "5px";
  return a;
}

const observer = new MutationObserver(() => scheduleProcess());
observer.observe(document.body, { childList: true, subtree: true });
scheduleProcess();
setInterval(processElements, 2000);

chrome.storage.onChanged.addListener((changes, area) => {
  if (area !== "local") return;
  if (changes.showTS !== undefined) showTS = changes.showTS.newValue !== false;
  if (changes.showSorsa !== undefined) showSorsa = changes.showSorsa.newValue !== false;
  if (changes.showFollowers !== undefined) showFollowers = changes.showFollowers.newValue !== false;
  COLOR_KEYS.forEach(k => {
    if (changes[k] !== undefined) {
      colorSettings[k] = typeof COLOR_DEFAULTS[k] === 'number' ? parseFloat(changes[k].newValue) || COLOR_DEFAULTS[k] : changes[k].newValue;
    }
  });
  const changed = Object.keys(changes).some(k => ["showTS", "showSorsa", "showFollowers", ...COLOR_KEYS].includes(k));
  if (changed) {
    document.querySelectorAll(".score-badge").forEach(e => e.remove());
    document.querySelectorAll("[data-score-processed]").forEach(e => { delete e.dataset.scoreProcessed; });
    scoreCache.clear();
    processElements();
  }
});

let allScrapedData = [], isScrolling = false, isPaused = false, currentIndicatorText = "";
const seenUrls = new Set();

chrome.runtime.onMessage.addListener((msg, sender, sendResponse) => {
  if (msg.type === "REFRESH_BADGES") {
    document.querySelectorAll(".score-badge").forEach(e => e.remove());
    document.querySelectorAll("[data-score-processed]").forEach(e => { delete e.dataset.scoreProcessed; });
    processElements();
    return;
  }
  if (msg.type === "AUTO_SCROLL_EXPORT") {
    allScrapedData = []; seenUrls.clear();
    startAutoScroll(msg.scrollMult, msg.waitMin, msg.waitMax, msg.scrollDir);
  } else if (msg.type === "PAUSE") {
    isPaused = true; updateIndicator("Paused.");
  } else if (msg.type === "RESUME") {
    if (isPaused) { isPaused = false; scrollStep(); }
  } else if (msg.type === "STOP") {
    isScrolling = false; isPaused = false;
    updateIndicator("Stopped. Generating CSV..."); downloadCSV();
  } else if (msg.type === "GET_STATUS") {
    let status = "IDLE";
    if (isScrolling) status = isPaused ? "PAUSED" : "SCROLLING";
    sendResponse({ status, text: currentIndicatorText });
  }
});

let scrollState = { px: 0, min: 0, max: 0, dir: 1, limitChecks: 0, lastH: 0, lastY: 0 };

function updateIndicator(text) { currentIndicatorText = text; }

function startAutoScroll(mult = 10, waitMin = 2, waitMax = 4, dir = 1) {
  if (isScrolling) return;
  isScrolling = true; isPaused = false; allScrapedData = [];
  scrollState = {
    px: 120 * mult * dir, min: waitMin, max: waitMax, dir,
    limitChecks: 0, lastH: document.body.scrollHeight, lastY: window.scrollY
  };
  updateIndicator("Starting..."); scrollStep();
}

function scrollStep() {
  if (!isScrolling || isPaused) return;
  processElements(); collectBatchData();
  window.scrollBy(0, scrollState.px);
  const wait = Math.random() * (scrollState.max - scrollState.min) + scrollState.min;
  const dirLabel = scrollState.dir === 1 ? "Down" : "Up";
  updateIndicator(`Scanning (${dirLabel}): ${allScrapedData.length} collected | ${wait.toFixed(1)}s`);
  setTimeout(() => {
    if (!isScrolling || isPaused) return;
    if (scrollState.dir === 1) {
      const h = document.body.scrollHeight, y = window.scrollY + window.innerHeight;
      if (h === scrollState.lastH && y >= h - 100) scrollState.limitChecks++;
      else { scrollState.limitChecks = 0; scrollState.lastH = h; }
    } else {
      if (window.scrollY === 0 || window.scrollY === scrollState.lastY) scrollState.limitChecks++;
      else { scrollState.limitChecks = 0; scrollState.lastY = window.scrollY; }
    }
    if (scrollState.limitChecks >= 3) {
      updateIndicator(`End reached (${dirLabel}). Downloading...`);
      isScrolling = false; downloadCSV(); return;
    }
    scrollStep();
  }, wait * 1000);
}

function collectBatchData() {
  document.querySelectorAll(".score-badge").forEach(el => {
    const url = el.href;
    if (seenUrls.has(url)) return;
    let handle = el.dataset.handle;
    if (!handle) {
      handle = url.replace("https://x.com/", "").replace("https://twitter.com/", "");
      handle = handle.split(/[\/?]/)[0];
    }
    const cached = scoreCache.get(handle);
    if (!cached) return;
    seenUrls.add(url);
    let colorStatus = "Green";
    if (el.classList.contains("low")) colorStatus = "Red";
    else if (el.classList.contains("medium")) colorStatus = "Yellow";
    allScrapedData.push({
      handle: `@${handle}`, url,
      tsScore: cached.tsScore || "N/A",
      sorsaScore: cached.sorsaScore || "N/A",
      followers: cached.followers || "N/A",
      colorStatus
    });
  });
}

function downloadCSV() {
  if (allScrapedData.length === 0) {
    updateIndicator("No data.");
    setTimeout(() => updateIndicator(""), 3000);
    return;
  }
  try {
    const esc = v => {
      if (v == null) return "";
      const s = String(v);
      return s.includes(",") || s.includes('"') || s.includes("\n") ? `"${s.replace(/"/g, '""')}"` : s;
    };
    let csv = "\ufeff" + ["Handle","URL","TS Score","Sorsa Score","Followers","Color"].join(",") + "\n";
    allScrapedData.forEach(row => {
      csv += [esc(row.handle), esc(row.url), esc(row.tsScore), esc(row.sorsaScore), esc(row.followers), esc(row.colorStatus)].join(",") + "\n";
    });
    const blob = new Blob([csv], { type: "text/csv;charset=utf-8;" });
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = `ScoreLens_${new Date().toISOString().slice(0,10)}.csv`;
    document.body.appendChild(a); a.click(); document.body.removeChild(a);
    updateIndicator(`CSV: ${allScrapedData.length} rows.`);
    setTimeout(() => updateIndicator(""), 5000);
  } catch (err) {
    console.error("CSV Error", err);
    updateIndicator("CSV Error!");
  }
}