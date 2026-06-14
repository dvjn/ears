const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

document.addEventListener('contextmenu', e => e.preventDefault());

// Tabs
document.querySelectorAll('.tab-btn').forEach(btn => {
  btn.addEventListener('click', () => {
    document.querySelectorAll('.tab-btn').forEach(b => b.classList.remove('active'));
    document.querySelectorAll('.tab-panel').forEach(p => p.classList.remove('active'));
    btn.classList.add('active');
    document.getElementById(`tab-${btn.dataset.tab}`).classList.add('active');
  });
});

// DOM refs
const lastText          = document.getElementById('last-text');
const copyBtn           = document.getElementById('copy-btn');
const recordBtn         = document.getElementById('record-btn');
const recordLabel       = document.getElementById('record-label');
const historyList       = document.getElementById('history-list');
const clearHistoryBtn   = document.getElementById('clear-history-btn');
const baseUrlInput        = document.getElementById('base-url-input');
const apiKeyInput         = document.getElementById('api-key-input');
const modelSelect         = document.getElementById('model-select');
const refreshModelsBtn    = document.getElementById('refresh-models-btn');
const langSelect          = document.getElementById('language-select');
const silenceStopInput  = document.getElementById('silence-stop-input');
const maxDurInput       = document.getElementById('max-duration-input');
const autoCopyInput     = document.getElementById('auto-copy-input');
const autoTypeInput     = document.getElementById('auto-type-input');
const historyLimitInput = document.getElementById('history-limit-input');
const toast             = document.getElementById('toast');

// Toast
let toastTimer;
function showToast(msg) {
  toast.textContent = msg;
  toast.classList.add('show');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => toast.classList.remove('show'), 2500);
}

// Timer
let timerInterval = null;
let timerStart = null;

function startTimer() {
  timerStart = Date.now();
  timerInterval = setInterval(() => {
    const elapsed = Math.floor((Date.now() - timerStart) / 1000);
    const m = Math.floor(elapsed / 60);
    const s = elapsed % 60;
    recordLabel.textContent = `${m}:${String(s).padStart(2, '0')}`;
  }, 500);
}

function stopTimer() {
  clearInterval(timerInterval);
  timerInterval = null;
}

// State
function updateBadge(state) {
  recordBtn.className = `record-btn${state === 'recording' ? ' recording' : ''}`;
  recordBtn.disabled = state === 'transcribing';
  if (state === 'recording') {
    startTimer();
  } else {
    stopTimer();
    recordLabel.textContent = state === 'transcribing' ? 'Transcribing...' : 'Record';
  }
}

// Live preview: show the growing paragraph in the transcription box while
// recording. `live-commit` carries the full transcript so far; `live-partial`
// carries the volatile (not-yet-committed) tail.
let liveCommitted = '';
let livePartial = '';

function resetLivePreview() {
  liveCommitted = '';
  livePartial = '';
}

function renderLivePreview() {
  // "…" is a bare activity cue from the backend, not real transcript text — the
  // main window already shows a "Listening…" placeholder, so ignore it here.
  const partial = livePartial === '…' ? '' : livePartial;
  const text = (liveCommitted + (liveCommitted && partial ? ' ' : '') + partial).trim();
  if (!text) return;
  lastText.textContent = text;
  lastText.classList.remove('empty');
}

// History
async function loadHistory() {
  try {
    const history = await invoke('get_history');
    renderHistory(history);
  } catch (e) {
    showToast(`History: ${e}`);
  }
}

const SVG_COPY = `<svg xmlns="http://www.w3.org/2000/svg" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/></svg>`;
const SVG_REMOVE = `<svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M18 6 6 18M6 6l12 12"/></svg>`;

function renderHistory(history) {
  if (!history.length) {
    historyList.innerHTML = '<div class="history-empty">No history yet.</div>';
    return;
  }
  historyList.innerHTML = '';
  history.forEach((text, index) => {
    const item = document.createElement('div');
    item.className = 'history-item';

    const span = document.createElement('span');
    span.className = 'history-text';
    span.textContent = text;
    span.title = text;

    const copyBtn = document.createElement('button');
    copyBtn.className = 'icon-btn';
    copyBtn.innerHTML = SVG_COPY;
    copyBtn.title = 'Copy';
    copyBtn.onclick = () => {
      navigator.clipboard.writeText(text).catch(() => {});
      showToast('Copied!');
    };

    const removeBtn = document.createElement('button');
    removeBtn.className = 'icon-btn danger';
    removeBtn.innerHTML = SVG_REMOVE;
    removeBtn.title = 'Remove';
    removeBtn.onclick = async () => {
      try {
        await invoke('remove_history_item', { index });
        await loadHistory();
      } catch (e) {
        showToast(`Remove failed: ${e}`);
      }
    };

    item.appendChild(copyBtn);
    item.appendChild(removeBtn);
    item.appendChild(span);
    historyList.appendChild(item);
  });
}

// Models
// Returns true on success, false if it couldn't fetch (so callers can suppress
// a misleading "Saved" toast). An empty apiKey tells the backend to use the
// stored key.
async function fetchAndRenderModels(savedModel) {
  const baseUrl = baseUrlInput.value.trim() || 'https://api.openai.com/v1';
  const apiKey = apiKeyInput.value.trim();
  if (!apiKey && !hasStoredKey) {
    modelSelect.innerHTML = '<option value="">Enter API key first</option>';
    return false;
  }
  refreshModelsBtn.disabled = true;
  refreshModelsBtn.textContent = '…';
  try {
    const models = await invoke('list_provider_models', { baseUrl, apiKey });
    modelSelect.innerHTML = '';
    for (const m of models) {
      const opt = document.createElement('option');
      opt.value = m;
      opt.textContent = m;
      modelSelect.appendChild(opt);
    }
    if (savedModel && models.includes(savedModel)) {
      modelSelect.value = savedModel;
    } else {
      const small = models.find(m => m.includes('small'));
      modelSelect.value = small ?? models[0] ?? '';
    }
    return true;
  } catch (e) {
    modelSelect.innerHTML = '<option value="">Failed to load</option>';
    showToast(`Models: ${e}`);
    return false;
  } finally {
    refreshModelsBtn.disabled = false;
    refreshModelsBtn.textContent = '↻';
  }
}

clearHistoryBtn.onclick = async () => {
  try {
    await invoke('clear_history');
    await loadHistory();
    showToast('History cleared');
  } catch (e) {
    showToast(`Clear failed: ${e}`);
  }
};

refreshModelsBtn.onclick = () => fetchAndRenderModels(modelSelect.value);

// Settings
//
// The backend never sends the saved API key to the renderer (it reports only
// `has_api_key`). The key field stays blank with a placeholder when a key is
// stored; an empty field on save means "keep the current key".
let hasStoredKey = false;

async function loadSettings() {
  const s = await invoke('get_settings');
  baseUrlInput.value = s.base_url ?? 'https://api.openai.com/v1';
  hasStoredKey = !!s.has_api_key;
  apiKeyInput.value = '';
  apiKeyInput.placeholder = hasStoredKey ? '•••••••• (saved — leave blank to keep)' : 'sk-…';
  await fetchAndRenderModels(s.model);
  langSelect.value = s.language ?? 'auto';
  silenceStopInput.value = s.silence_stop_secs ?? 3;
  maxDurInput.value = s.max_duration_secs;
  autoCopyInput.checked = s.auto_copy ?? true;
  autoTypeInput.checked = s.auto_type ?? false;
  historyLimitInput.value = s.history_limit ?? 10;
  renderRecordMeta(s);
}

function renderRecordMeta(s) {
  const meta = document.getElementById('record-meta');
  const langOption = langSelect.querySelector(`option[value="${s.language ?? 'auto'}"]`);
  const langLabel = langOption ? langOption.textContent : (s.language ?? 'Auto-detect');
  const chips = [s.model || '—', langLabel];
  // Build chips with textContent — model ids come verbatim from the provider's
  // /models response and must never be interpolated into innerHTML.
  meta.replaceChildren(...chips.map(c => {
    const span = document.createElement('span');
    span.className = 'meta-chip';
    span.textContent = c;
    return span;
  }));
}

function collectSettings() {
  return {
    base_url: baseUrlInput.value.trim() || 'https://api.openai.com/v1',
    api_key: apiKeyInput.value.trim(),
    model: modelSelect.value,
    language: langSelect.value === 'auto' ? null : langSelect.value,
    silence_stop_secs: Math.max(0, Math.min(60, parseInt(silenceStopInput.value, 10) || 0)),
    max_duration_secs: parseInt(maxDurInput.value, 10) || 120,
    auto_copy: autoCopyInput.checked,
    auto_type: autoTypeInput.checked,
    history_limit: Math.min(9999, Math.max(1, parseInt(historyLimitInput.value, 10) || 10)),
  };
}

// Auto-save: persist on any change instead of an explicit Save button.
async function saveSettings({ silent = false } = {}) {
  const s = collectSettings();
  try {
    await invoke('save_settings', { settings: s });
    // If the user typed a new key, it's now stored — clear the field and switch
    // to the "saved" placeholder so we don't keep resending it.
    if (s.api_key) {
      hasStoredKey = true;
      apiKeyInput.value = '';
      apiKeyInput.placeholder = '•••••••• (saved — leave blank to keep)';
    }
    renderRecordMeta(s);
    if (!silent) showToast('Saved');
  } catch (e) {
    showToast(`Error: ${e}`);
  }
}

// Connection fields also refresh the model list, then save. If the model fetch
// failed, suppress the "Saved" toast so it doesn't mask the error.
async function onConnectionChange() {
  const ok = await fetchAndRenderModels(modelSelect.value);
  await saveSettings({ silent: !ok });
}

baseUrlInput.addEventListener('change', onConnectionChange);
apiKeyInput.addEventListener('change', onConnectionChange);
[modelSelect, langSelect, silenceStopInput, autoCopyInput, autoTypeInput, maxDurInput, historyLimitInput]
  .forEach(el => el.addEventListener('change', saveSettings));

// Buttons
recordBtn.onclick = async () => {
  try {
    await invoke('cmd_toggle_recording');
  } catch (e) {
    showToast(String(e));
  }
};

copyBtn.onclick = () => {
  const text = lastText.textContent;
  if (text && !lastText.classList.contains('empty')) {
    navigator.clipboard.writeText(text).catch(() => {});
    showToast('Copied!');
  }
};

// Events
listen('recording-state-changed', (e) => {
  if (e.payload === 'recording') {
    // Starting a fresh capture: clear the live preview and show the record tab
    // so the growing paragraph is visible.
    resetLivePreview();
    lastText.textContent = 'Listening…';
    lastText.classList.add('empty');
    document.querySelector('.tab-btn[data-tab="record"]').click();
  }
  updateBadge(e.payload);
});

listen('transcription-done', (e) => {
  const text = e.payload;
  resetLivePreview();
  lastText.textContent = text;
  lastText.classList.remove('empty');
  copyBtn.disabled = false;
  loadHistory();
  document.querySelector('.tab-btn[data-tab="record"]').click();
});

listen('transcription-error', (e) => showToast(e.payload));

listen('error-no-api-key', () => {
  showToast('No API key set — go to Settings');
  document.querySelector('.tab-btn[data-tab="settings"]').click();
});

// Live dictation preview (live mode only). Reset on the same `live-reset` event
// the overlay uses, so both windows clear their captions in lockstep.
listen('live-commit', (e) => { liveCommitted = e.payload?.text ?? ''; renderLivePreview(); });
listen('live-partial', (e) => { livePartial = e.payload?.text ?? ''; renderLivePreview(); });
listen('live-reset', () => { resetLivePreview(); });
listen('live-error', (e) => showToast(`Live: ${e.payload}`));

// Init
(async () => {
  try {
    await loadSettings();
    await loadHistory();
    const status = await invoke('get_status');
    updateBadge(status.state);
    if (status.last_result) {
      lastText.textContent = status.last_result;
      lastText.classList.remove('empty');
      copyBtn.disabled = false;
    }
  } catch (e) {
    showToast(`Init error: ${e}`);
  }
})();
