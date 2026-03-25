const els = {
  apiBase: document.getElementById("api-base"),
  token: document.getElementById("token"),
  sessions: document.getElementById("sessions"),
  sessionCount: document.getElementById("session-count"),
  taps: document.getElementById("taps"),
  transcript: document.getElementById("transcript"),
  transcriptMeta: document.getElementById("transcript-meta"),
  log: document.getElementById("log"),
};

let tapState = [];

function headers() {
  const token = els.token.value.trim();
  const out = { "Content-Type": "application/json" };
  if (token) out.Authorization = `Bearer ${token}`;
  return out;
}

function baseUrl(path) {
  return `${els.apiBase.value.replace(/\/$/, "")}${path}`;
}

function setText(el, value, empty = false) {
  el.textContent = value;
  el.classList.toggle("empty", empty);
}

function log(message, data) {
  const stamp = new Date().toLocaleTimeString();
  const payload = data === undefined ? message : `${message}\n${JSON.stringify(data, null, 2)}`;
  els.log.textContent = `[${stamp}] ${payload}\n\n${els.log.textContent}`.trim();
}

async function request(path, options = {}) {
  const res = await fetch(baseUrl(path), {
    ...options,
    headers: {
      ...headers(),
      ...(options.headers || {}),
    },
  });
  const text = await res.text();
  let body = text;
  try {
    body = text ? JSON.parse(text) : null;
  } catch {}
  if (!res.ok) {
    throw new Error(typeof body === "string" ? body : JSON.stringify(body));
  }
  return body;
}

function renderSessions(data) {
  const sessions = data.sessions || [];
  els.sessionCount.textContent = `${sessions.length} sessions`;
  setText(els.sessions, JSON.stringify(sessions, null, 2), sessions.length === 0);
}

function renderTaps() {
  if (tapState.length === 0) {
    els.taps.innerHTML = "";
    els.taps.classList.add("empty");
    els.taps.textContent = "No taps yet.";
    return;
  }
  els.taps.classList.remove("empty");
  els.taps.innerHTML = "";
  for (const tap of tapState) {
    const item = document.createElement("div");
    item.className = "tap";
    item.innerHTML = `
      <div>
        <strong>${tap.app_name}</strong>
        <span class="pill">${tap.status}</span>
      </div>
      <div class="meta">pid ${tap.target_pid} · ${tap.tap_id}</div>
      <div class="row">
        <button data-action="latest" data-tap="${tap.tap_id}">Latest transcript</button>
        <button data-action="stop" data-tap="${tap.tap_id}" class="danger">Stop</button>
      </div>
    `;
    els.taps.appendChild(item);
  }
}

async function refreshStatus() {
  const body = await request("/api/condor_audio/status");
  tapState = body.active_taps || [];
  renderTaps();
  log("status", body);
}

async function refreshSessions() {
  const body = await request("/api/condor_audio/sessions");
  renderSessions(body);
  log("sessions", body);
}

async function startTap(app) {
  const body = await request("/api/condor_audio/taps", {
    method: "POST",
    body: JSON.stringify({ app }),
  });
  log(`started ${app} tap`, body);
  await refreshStatus();
}

async function stopTap(tapId) {
  const body = await request(`/api/condor_audio/taps/${tapId}`, { method: "DELETE" });
  log(`stopped ${tapId}`, body);
  await refreshStatus();
}

async function latestTranscript(tapId) {
  const body = await request(`/api/condor_audio/taps/${tapId}/latest-transcript`);
  els.transcriptMeta.textContent = tapId;
  setText(els.transcript, body.text || "(empty transcript)", !(body.text || "").trim());
  log(`latest transcript ${tapId}`, body);
}

document.getElementById("check-status").addEventListener("click", () => refreshStatus().catch((err) => log("status error", err.message)));
document.getElementById("load-sessions").addEventListener("click", () => refreshSessions().catch((err) => log("sessions error", err.message)));
document.getElementById("refresh-taps").addEventListener("click", () => refreshStatus().catch((err) => log("refresh error", err.message)));
document.getElementById("clear-log").addEventListener("click", () => setText(els.log, "Ready."));

for (const button of document.querySelectorAll(".target-btn")) {
  button.addEventListener("click", () => startTap(button.dataset.app).catch((err) => log("start error", err.message)));
}

els.taps.addEventListener("click", (event) => {
  const target = event.target.closest("button");
  if (!target) return;
  const tapId = target.dataset.tap;
  const action = target.dataset.action;
  if (action === "stop") {
    stopTap(tapId).catch((err) => log("stop error", err.message));
  } else if (action === "latest") {
    latestTranscript(tapId).catch((err) => log("latest error", err.message));
  }
});
