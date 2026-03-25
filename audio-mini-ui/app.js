const els = {
  apiBase: document.getElementById("apiBase"),
  token: document.getElementById("token"),
  channel: document.getElementById("channel"),
  sessionPid: document.getElementById("sessionPid"),
  refreshBtn: document.getElementById("refreshBtn"),
  startBtn: document.getElementById("startBtn"),
  stopBtn: document.getElementById("stopBtn"),
  copyBtn: document.getElementById("copyBtn"),
  stateBadge: document.getElementById("stateBadge"),
  tapId: document.getElementById("tapId"),
  updatedAt: document.getElementById("updatedAt"),
  statusText: document.getElementById("statusText"),
  sessionsList: document.getElementById("sessionsList"),
  sessionCount: document.getElementById("sessionCount"),
  transcriptBox: document.getElementById("transcriptBox"),
};

const storageKey = "condor-audio-mini-ui";
const state = {
  sessions: [],
  tap: null,
  pollHandle: null,
  lastTranscript: "",
};

function loadPrefs() {
  try {
    const saved = JSON.parse(localStorage.getItem(storageKey) || "{}");
    if (saved.apiBase) els.apiBase.value = saved.apiBase;
    if (saved.token) els.token.value = saved.token;
    if (saved.channel) els.channel.value = saved.channel;
  } catch (_error) {
    // Ignore invalid local state.
  }
}

function savePrefs() {
  localStorage.setItem(
    storageKey,
    JSON.stringify({
      apiBase: els.apiBase.value.trim(),
      token: els.token.value,
      channel: els.channel.value,
    }),
  );
}

function audioApiBase() {
  return els.apiBase.value.trim().replace(/\/+$/, "");
}

function authHeaders() {
  const token = els.token.value.trim();
  return token ? { Authorization: `Bearer ${token}` } : {};
}

async function api(path, options = {}) {
  const response = await fetch(`${audioApiBase()}${path}`, {
    ...options,
    headers: {
      "Content-Type": "application/json",
      ...authHeaders(),
      ...(options.headers || {}),
    },
  });

  const contentType = response.headers.get("content-type") || "";
  const body = contentType.includes("application/json")
    ? await response.json()
    : await response.text();

  if (!response.ok) {
    const message =
      typeof body === "string" ? body : body.error || body.detail || JSON.stringify(body);
    throw new Error(message || `HTTP ${response.status}`);
  }
  return body;
}

function setStatus(kind, message) {
  els.stateBadge.textContent = kind;
  els.stateBadge.className = `badge ${kind.toLowerCase()}`;
  els.statusText.textContent = message;
}

function updateTapUi() {
  els.tapId.textContent = state.tap?.tap_id || "-";
  els.updatedAt.textContent = state.tap?.last_chunk_ts || "-";
  els.stopBtn.disabled = !state.tap || state.tap.status === "stopped";
}

function renderSessions() {
  const channel = els.channel.value;
  const visible = state.sessions.filter((session) => session.matched_target === channel);
  els.sessionCount.textContent = String(visible.length);
  els.sessionPid.innerHTML = '<option value="">Auto-detect</option>';

  if (!visible.length) {
    els.sessionsList.className = "sessions-list empty";
    els.sessionsList.textContent = `No ${channel} sessions found.`;
    return;
  }

  els.sessionsList.className = "sessions-list";
  els.sessionsList.innerHTML = visible
    .map(
      (session) => `
        <div class="session-card">
          <strong>${session.display_name} · pid ${session.pid} · ${session.state}</strong>
          <span>${session.exe_path}</span>
        </div>
      `,
    )
    .join("");

  for (const session of visible) {
    const option = document.createElement("option");
    option.value = String(session.pid);
    option.textContent = `${session.display_name} (${session.pid})`;
    els.sessionPid.appendChild(option);
  }
}

async function refreshSessions() {
  savePrefs();
  setStatus("Loading", "Refreshing audio sessions.");
  try {
    const result = await api("/api/condor_audio/sessions");
    state.sessions = Array.isArray(result.sessions) ? result.sessions : [];
    renderSessions();
    setStatus("Idle", `Loaded ${state.sessions.length} session(s).`);
  } catch (error) {
    setStatus("Error", `Session refresh failed: ${error.message}`);
  }
}

async function refreshTap() {
  if (!state.tap?.tap_id) return;
  try {
    state.tap = await api(`/api/condor_audio/taps/${encodeURIComponent(state.tap.tap_id)}`);
    updateTapUi();
  } catch (error) {
    stopPolling();
    setStatus("Error", `Tap refresh failed: ${error.message}`);
  }
}

async function refreshTranscript() {
  if (!state.tap?.tap_id) return;
  try {
    const payload = await api(
      `/api/condor_audio/taps/${encodeURIComponent(state.tap.tap_id)}/latest-transcript`,
    );
    if (payload.text && payload.text !== state.lastTranscript) {
      state.lastTranscript = payload.text;
      els.transcriptBox.value = payload.text;
      setStatus("Listening", "Transcript updated.");
    }
  } catch (error) {
    if (!String(error.message).includes("has no transcript yet")) {
      setStatus("Listening", `Waiting for transcript: ${error.message}`);
    }
  }
}

function startPolling() {
  stopPolling();
  state.pollHandle = setInterval(async () => {
    await refreshTap();
    await refreshTranscript();
  }, 1500);
}

function stopPolling() {
  if (state.pollHandle) {
    clearInterval(state.pollHandle);
    state.pollHandle = null;
  }
}

async function startListening() {
  savePrefs();
  setStatus("Loading", `Starting ${els.channel.value} tap.`);
  try {
    state.tap = await api("/api/condor_audio/taps", {
      method: "POST",
      body: JSON.stringify({
        app: els.channel.value,
        pid: els.sessionPid.value ? Number(els.sessionPid.value) : undefined,
      }),
    });
    state.lastTranscript = "";
    els.transcriptBox.value = "";
    updateTapUi();
    startPolling();
    setStatus("Listening", `Tap ${state.tap.tap_id} started.`);
  } catch (error) {
    setStatus("Error", `Start failed: ${error.message}`);
  }
}

async function stopListening() {
  if (!state.tap?.tap_id) return;
  setStatus("Loading", `Stopping ${state.tap.tap_id}.`);
  try {
    await api(`/api/condor_audio/taps/${encodeURIComponent(state.tap.tap_id)}`, {
      method: "DELETE",
    });
    stopPolling();
    state.tap = null;
    updateTapUi();
    setStatus("Idle", "Tap stopped.");
  } catch (error) {
    setStatus("Error", `Stop failed: ${error.message}`);
  }
}

async function copyTranscript() {
  try {
    await navigator.clipboard.writeText(els.transcriptBox.value);
    setStatus("Idle", "Transcript copied.");
  } catch (error) {
    setStatus("Error", `Copy failed: ${error.message}`);
  }
}

els.channel.addEventListener("change", renderSessions);
els.apiBase.addEventListener("change", savePrefs);
els.token.addEventListener("change", savePrefs);
els.refreshBtn.addEventListener("click", refreshSessions);
els.startBtn.addEventListener("click", startListening);
els.stopBtn.addEventListener("click", stopListening);
els.copyBtn.addEventListener("click", copyTranscript);

loadPrefs();
renderSessions();
