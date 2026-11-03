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
const langSelect        = document.getElementById('language-select');
const deviceSelect      = document.getElementById('input-device-select');
const devicesRefreshBtn = document.getElementById('devices-refresh-btn');
const silenceStopInput  = document.getElementById('silence-stop-input');
const maxDurInput       = document.getElementById('max-duration-input');
const autoCopyInput     = document.getElementById('auto-copy-input');
const autoTypeInput     = document.getElementById('auto-type-input');
const historyLimitInput = document.getElementById('history-limit-input');
const llmOptions        = document.getElementById('llm-options');
const llmModelHint      = document.getElementById('llm-model-hint');
const ppScreenshotInput = document.getElementById('postprocess-screenshot-input');
const captureSelect     = document.getElementById('screenshot-method-select');
const ppPromptInput     = document.getElementById('postprocess-prompt-input');
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

clearHistoryBtn.onclick = async () => {
  try {
    await invoke('clear_history');
    await loadHistory();
    showToast('History cleared');
  } catch (e) {
    showToast(`Clear failed: ${e}`);
  }
};

// Providers
//
// Transcription ("stt") and post-processing ("llm") each have a provider: an
// ears service (ears-stt / ears-llm) or any OpenAI-compatible API, and
// post-processing can also be off. The backend never sends saved API keys to
// the renderer (it reports only `has_*_api_key`); key fields stay blank with a
// placeholder when a key is stored, and an empty field on save means "keep
// the key".
const PRESET_URLS = {
  stt: { ears: 'http://localhost:13278/v1', openai: 'https://api.openai.com/v1' },
  llm: { ears: 'http://localhost:13279/v1', openai: 'https://api.openai.com/v1' },
};

function providerUi(role) {
  return {
    role,
    provider: document.getElementById(`${role}-provider-select`),
    url: document.getElementById(`${role}-url-input`),
    key: document.getElementById(`${role}-key-input`),
    model: document.getElementById(`${role}-model-select`),
    refresh: document.getElementById(`${role}-refresh-btn`),
    hasKey: false,
    cleanupModels: [],
  };
}
const stt = providerUi('stt');
const llm = providerUi('llm');

function renderKeyPlaceholder(ui) {
  ui.key.value = '';
  ui.key.placeholder = ui.hasKey ? '••••••••' : (ui.provider.value === 'ears' ? 'none' : 'sk-…');
}

// Check the provider and refill its model picker. Returns true on success, so
// callers can suppress a misleading "Saved" toast.
async function refreshProvider(ui, savedModel) {
  if (ui.provider.value === 'off') return true;
  ui.refresh.disabled = true;
  ui.refresh.textContent = '…';
  try {
    const status = await invoke('check_provider', {
      role: ui.role,
      kind: ui.provider.value,
      baseUrl: ui.url.value.trim(),
      apiKey: ui.key.value.trim(),
    });
    ui.cleanupModels = status.cleanup_models;
    ui.model.replaceChildren(...status.models.map(m => {
      const opt = document.createElement('option');
      opt.value = m;
      opt.textContent = ui.cleanupModels.includes(m) ? `${m} (dictation only)` : m;
      return opt;
    }));
    const keep = savedModel && status.models.includes(savedModel);
    // Fresh pick: ears-stt's default whisper for transcription, ears-llm's
    // default model for post-processing.
    const preferred = ui.role === 'stt' ? 'medium.en' : 'granite-4.2-3b';
    ui.model.value = keep ? savedModel : (status.models.find(m => m.includes(preferred)) ?? status.models[0] ?? '');
    return true;
  } catch (e) {
    // Keep the saved model selectable so a server that's down doesn't wipe it.
    ui.model.replaceChildren();
    if (savedModel) {
      const opt = document.createElement('option');
      opt.value = savedModel;
      opt.textContent = savedModel;
      ui.model.appendChild(opt);
    }
    showToast(`${ui.role === 'stt' ? 'Transcription' : 'Post-processing'}: ${e}`);
    return false;
  } finally {
    ui.refresh.disabled = false;
    ui.refresh.textContent = '↻';
    updateCleanupControls();
  }
}

function updateCleanupControls() {
  llmOptions.hidden = llm.provider.value === 'off';
  const tuned = llm.cleanupModels.includes(llm.model.value);
  llmModelHint.textContent = tuned
    ? 'Trained only to fix dictation: uses its own prompt, no screenshot'
    : 'Language model that fixes the transcript';
  ppScreenshotInput.disabled = tuned;
  captureSelect.disabled = tuned || !ppScreenshotInput.checked;
  ppPromptInput.disabled = tuned;
}

async function loadDevices(saved) {
  devicesRefreshBtn.disabled = true;
  try {
    // [{id, label}], empty when only the system default can be offered.
    const mics = await invoke('list_input_devices');
    const options = [['', 'System default'], ...mics.map(m => [m.id, m.label])];
    if (saved && !mics.some(m => m.id === saved)) {
      options.push([saved, 'Saved microphone (not connected)']);
    }
    deviceSelect.replaceChildren(...options.map(([value, label]) => {
      const opt = document.createElement('option');
      opt.value = value;
      opt.textContent = label;
      return opt;
    }));
    deviceSelect.value = saved ?? '';
  } catch (e) {
    showToast(`Devices: ${e}`);
  } finally {
    devicesRefreshBtn.disabled = false;
  }
}

// Settings

async function loadSettings() {
  const s = await invoke('get_settings');
  for (const [ui, prefix] of [[stt, 'stt'], [llm, 'llm']]) {
    ui.provider.value = s[`${prefix}_provider`];
    ui.url.value = s[`${prefix}_base_url`];
    ui.hasKey = !!s[`has_${prefix}_api_key`];
    renderKeyPlaceholder(ui);
  }
  langSelect.value = s.language ?? 'auto';
  silenceStopInput.value = s.silence_stop_secs ?? 3;
  maxDurInput.value = s.max_duration_secs;
  autoCopyInput.checked = s.auto_copy ?? true;
  autoTypeInput.checked = s.auto_type ?? false;
  historyLimitInput.value = s.history_limit ?? 10;
  ppScreenshotInput.checked = s.postprocess_use_screenshot ?? false;
  captureSelect.value = s.screenshot_method ?? 'auto';
  ppPromptInput.value = s.postprocess_prompt ?? '';
  updateCleanupControls();
  await Promise.all([
    refreshProvider(stt, s.stt_model),
    refreshProvider(llm, s.llm_model),
    loadDevices(s.input_device),
  ]);
}

function collectSettings() {
  return {
    stt_provider: stt.provider.value,
    stt_base_url: stt.url.value.trim(),
    stt_api_key: stt.key.value.trim(),
    stt_model: stt.model.value,
    llm_provider: llm.provider.value,
    llm_base_url: llm.url.value.trim(),
    llm_api_key: llm.key.value.trim(),
    llm_model: llm.model.value,
    postprocess_use_screenshot: ppScreenshotInput.checked,
    screenshot_method: captureSelect.value,
    postprocess_prompt: ppPromptInput.value.trim(),
    input_device: deviceSelect.value || null,
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
    // A newly typed key is now stored; clear the field so it isn't resent.
    for (const ui of [stt, llm]) {
      if (ui.key.value.trim()) {
        ui.hasKey = true;
        renderKeyPlaceholder(ui);
      }
    }
    if (!silent) showToast('Saved');
  } catch (e) {
    showToast(`Error: ${e}`);
  }
}

for (const ui of [stt, llm]) {
  // Switching provider swaps in that provider's usual URL, unless the user
  // has typed a custom one.
  ui.provider.addEventListener('change', async () => {
    const presets = Object.values(PRESET_URLS[ui.role]);
    const preset = PRESET_URLS[ui.role][ui.provider.value];
    if (preset && (!ui.url.value.trim() || presets.includes(ui.url.value.trim()))) {
      ui.url.value = preset;
    }
    renderKeyPlaceholder(ui);
    updateCleanupControls();
    const ok = await refreshProvider(ui, ui.model.value);
    await saveSettings({ silent: !ok });
  });
  const onConnectionChange = async () => {
    const ok = await refreshProvider(ui, ui.model.value);
    await saveSettings({ silent: !ok });
  };
  ui.url.addEventListener('change', onConnectionChange);
  ui.key.addEventListener('change', onConnectionChange);
  ui.refresh.onclick = () => refreshProvider(ui, ui.model.value);
  ui.model.addEventListener('change', () => {
    updateCleanupControls();
    saveSettings();
  });
}

devicesRefreshBtn.onclick = () => loadDevices(deviceSelect.value || null);

[deviceSelect, langSelect, silenceStopInput, autoCopyInput, autoTypeInput, maxDurInput,
 historyLimitInput, captureSelect, ppPromptInput]
  .forEach(el => el.addEventListener('change', saveSettings));

ppScreenshotInput.addEventListener('change', () => {
  updateCleanupControls();
  saveSettings();
});

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
  showToast('No transcription API key set — go to Settings');
  document.querySelector('.tab-btn[data-tab="settings"]').click();
});

// Live dictation preview (live mode only). Reset on the same `live-reset` event
// the overlay uses, so both windows clear their captions in lockstep.
listen('live-commit', (e) => { liveCommitted = e.payload?.text ?? ''; renderLivePreview(); });
listen('live-partial', (e) => { livePartial = e.payload?.text ?? ''; renderLivePreview(); });
listen('live-reset', () => { resetLivePreview(); });
listen('live-error', (e) => showToast(`Live: ${e.payload}`));

// The live overlay switched microphones; keep this picker in step so the
// next auto-save doesn't put the old one back.
listen('input-device-changed', (e) => loadDevices(e.payload ?? null));

// The microphone wouldn't open or sent no audio even after ears reopened it:
// say so and put the user at the microphone picker.

listen('mic-failed', async (e) => {
  showToast(e.payload);
  document.querySelector('.tab-btn[data-tab="settings"]').click();
  await loadDevices(deviceSelect.value || null);
  deviceSelect.scrollIntoView({ block: 'center' });
  deviceSelect.focus();
});

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
