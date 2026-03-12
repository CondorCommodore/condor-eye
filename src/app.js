const { invoke } = window.__TAURI__.core;

// DOM elements
const captureBtn = document.getElementById('capture-btn');
const symbolSelect = document.getElementById('symbol-select');
const modeSelect = document.getElementById('mode-select');
const profileSelect = document.getElementById('profile-select');
const statusEl = document.getElementById('status');
const resultsEl = document.getElementById('results');
const badgeEl = document.getElementById('result-badge');
const summaryEl = document.getElementById('result-summary');
const detailsEl = document.getElementById('result-details');
const metaEl = document.getElementById('result-meta');
const closeBtn = document.getElementById('close-results');

// Load available profiles
async function loadProfiles() {
  try {
    const profiles = await invoke('list_profiles');
    profileSelect.textContent = '';
    for (const name of profiles) {
      const opt = document.createElement('option');
      opt.value = name;
      opt.textContent = name;
      profileSelect.appendChild(opt);
    }
  } catch (e) {
    console.error('Failed to load profiles:', e);
  }
}

// Capture handler
async function doCapture() {
  const symbol = symbolSelect.value;
  const mode = modeSelect.value;
  const profileName = profileSelect.value;

  statusEl.textContent = 'Capturing...';
  statusEl.className = 'working';
  captureBtn.disabled = true;

  try {
    if (mode === 'free') {
      const text = await invoke('capture_free', { prompt: '' });
      renderFreeResult(text);
      statusEl.textContent = 'Done';
      statusEl.className = 'success';
    } else {
      const report = await invoke('capture_and_compare', {
        symbol,
        mode,
        profileName,
      });
      renderResults(report);
      statusEl.textContent = 'Done — ' + report.api_latency_ms + 'ms';
      statusEl.className = 'success';
    }
  } catch (err) {
    statusEl.textContent = 'Error: ' + String(err);
    statusEl.className = 'error';
    resultsEl.classList.add('hidden');
  } finally {
    captureBtn.disabled = false;
  }
}

// Render free-mode AI response
function renderFreeResult(text) {
  resultsEl.classList.remove('hidden');
  badgeEl.textContent = 'AI';
  badgeEl.className = 'badge-extract';
  summaryEl.textContent = 'Free observation';
  detailsEl.textContent = '';
  const pre = document.createElement('pre');
  pre.style.whiteSpace = 'pre-wrap';
  pre.style.wordBreak = 'break-word';
  pre.textContent = text;
  detailsEl.appendChild(pre);
  metaEl.textContent = '';
}

// Helper: create a text line element
function createLine(text, className) {
  const div = document.createElement('div');
  if (className) div.className = className;
  div.textContent = text;
  return div;
}

// Render comparison results using safe DOM methods (no innerHTML)
function renderResults(report) {
  resultsEl.classList.remove('hidden');

  // Badge
  const statusMap = {
    PASS: ['PASS', 'badge-pass'],
    WARN: ['WARN', 'badge-warn'],
    FAIL: ['FAIL', 'badge-fail'],
    EXTRACT_ONLY: ['EXTRACT', 'badge-extract'],
  };
  const [label, cls] = statusMap[report.overall] || ['?', ''];
  badgeEl.textContent = label;
  badgeEl.className = cls;

  // Summary
  if (report.overall === 'EXTRACT_ONLY') {
    summaryEl.textContent = 'Extracted ' + report.extracted_bids + ' bids, ' + report.extracted_asks + ' asks';
  } else {
    summaryEl.textContent =
      report.extracted_bids + '/' + report.truth_bids + ' bids, ' +
      report.extracted_asks + '/' + report.truth_asks + ' asks';
  }

  // Details — clear and rebuild with safe DOM methods
  detailsEl.textContent = '';

  if (report.mismatches.length > 0) {
    detailsEl.appendChild(createLine('Mismatches:', 'mismatch-line'));
    for (const m of report.mismatches) {
      detailsEl.appendChild(createLine(
        '  ' + m.side + ' $' + m.price.toFixed(2) + ': extracted=' + m.extracted_volume + ', truth=' + m.truth_volume,
        'mismatch-line'
      ));
    }
  }
  if (report.missing.length > 0) {
    detailsEl.appendChild(createLine('Missing (in truth, not on screen):', 'missing-line'));
    for (const m of report.missing) {
      detailsEl.appendChild(createLine(
        '  ' + m.side + ' $' + m.price.toFixed(2) + ' x ' + m.volume,
        'missing-line'
      ));
    }
  }
  if (report.extra.length > 0) {
    detailsEl.appendChild(createLine('Extra (on screen, not in truth):', 'extra-line'));
    for (const e of report.extra) {
      detailsEl.appendChild(createLine(
        '  ' + e.side + ' $' + e.price.toFixed(2) + ' x ' + (e.volume != null ? e.volume : '?'),
        'extra-line'
      ));
    }
  }
  if (report.mismatches.length === 0 && report.missing.length === 0 && report.extra.length === 0) {
    const msg = report.overall === 'EXTRACT_ONLY'
      ? 'Extract-only mode — no comparison performed'
      : 'All levels match';
    const em = document.createElement('em');
    em.textContent = msg;
    detailsEl.appendChild(em);
  }

  // Metadata
  metaEl.textContent =
    'Latency: ' + report.api_latency_ms + 'ms | ' +
    'Cost: $' + report.estimated_cost_usd.toFixed(4) + ' | ' +
    'Time: ' + new Date(report.timestamp).toLocaleTimeString();
}

// Event listeners
captureBtn.addEventListener('click', doCapture);
closeBtn.addEventListener('click', () => resultsEl.classList.add('hidden'));

// Listen for global shortcut trigger
const { listen } = window.__TAURI__.event;
listen('trigger-capture', () => {
  doCapture();
});

// Window controls
document.getElementById('minimize-btn').addEventListener('click', () => invoke('plugin:window|minimize'));
document.getElementById('close-btn').addEventListener('click', () => invoke('plugin:window|close'));

// Window dragging via Rust command
const dragHandle = document.getElementById('drag-handle');
const toolbar = document.getElementById('toolbar');

function startDrag(e) {
  if (e.target.tagName === 'BUTTON' || e.target.tagName === 'SELECT' || e.target.tagName === 'INPUT') return;
  if (e.button !== 0) return;
  e.preventDefault();
  invoke('start_drag');
}

dragHandle.addEventListener('mousedown', startDrag);
toolbar.addEventListener('mousedown', startDrag);

// Initialize
loadProfiles();
