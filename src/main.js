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
const lastText       = document.getElementById('last-text');
const copyBtn        = document.getElementById('copy-btn');
const recordBtn      = document.getElementById('record-btn');
const recordLabel    = document.getElementById('record-label');
const historyList    = document.getElementById('history-list');
const modelsList     = document.getElementById('models-list');
const modelSelect        = document.getElementById('model-select');
const langSelect         = document.getElementById('language-select');
const maxDurInput        = document.getElementById('max-duration-input');
const typeAtCursorInput  = document.getElementById('type-at-cursor-input');
const historyLimitInput  = document.getElementById('history-limit-input');
const saveBtn            = document.getElementById('save-btn');
const toast          = document.getElementById('toast');

// Toast
let toastTimer;
function showToast(msg) {
  toast.textContent = msg;
  toast.classList.add('show');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => toast.classList.remove('show'), 2500);
}

// Timer (runs inside the button label)
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

// History
async function loadHistory() {
  const history = await invoke('get_history');
  renderHistory(history);
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
      await invoke('remove_history_item', { index });
      await loadHistory();
    };

    item.appendChild(copyBtn);
    item.appendChild(removeBtn);
    item.appendChild(span);
    historyList.appendChild(item);
  });
}

// Models
async function loadModels() {
  const models = await invoke('list_models');
  renderModels(models);
  renderModelSelect(models);
}

function renderModels(models) {
  modelsList.innerHTML = '';
  for (const m of models) {
    const row = document.createElement('div');
    row.className = 'model-row';
    row.innerHTML = `
      <div class="dot ${m.downloaded ? 'downloaded' : ''}"></div>
      <span class="model-name">${m.name}</span>
      <span class="model-size">${m.size_mb >= 1000 ? (m.size_mb/1024).toFixed(1)+'GB' : m.size_mb+'MB'}</span>
    `;

    if (m.downloaded) {
      const del = document.createElement('button');
      del.className = 'sm danger';
      del.textContent = 'Delete';
      del.onclick = async () => {
        await invoke('delete_model', { modelName: m.name });
        await loadModels();
        showToast(`Deleted ${m.name}`);
      };
      row.appendChild(del);
    } else {
      const dl = document.createElement('button');
      dl.className = 'sm';
      dl.id = `dl-${m.name}`;
      dl.textContent = 'Download';
      dl.onclick = () => startDownload(m.name);
      row.appendChild(dl);
    }

    modelsList.appendChild(row);

    const prog = document.createElement('progress');
    prog.id = `prog-${m.name}`;
    prog.value = 0;
    prog.max = 100;
    prog.style.display = 'none';
    modelsList.appendChild(prog);
  }
}

function renderModelSelect(models) {
  const prev = modelSelect.value;
  modelSelect.innerHTML = '';
  for (const m of models.filter(m => m.downloaded)) {
    const opt = document.createElement('option');
    opt.value = m.name;
    opt.textContent = m.name;
    modelSelect.appendChild(opt);
  }
  if (prev) modelSelect.value = prev;
}

async function startDownload(modelName) {
  const btn = document.getElementById(`dl-${modelName}`);
  const prog = document.getElementById(`prog-${modelName}`);
  if (btn) { btn.disabled = true; btn.textContent = 'Downloading...'; }
  if (prog) prog.style.display = 'block';
  try {
    await invoke('download_model', { modelName });
    showToast(`Downloaded ${modelName}`);
  } catch (e) {
    showToast(`Download failed: ${e}`);
  } finally {
    await loadModels();
    if (prog) prog.style.display = 'none';
  }
}

// Settings
async function loadSettings() {
  const s = await invoke('get_settings');
  modelSelect.value = s.model_name;
  langSelect.value = s.language ?? 'auto';
  maxDurInput.value = s.max_duration_secs;
  typeAtCursorInput.checked = s.type_at_cursor ?? false;
  historyLimitInput.value = s.history_limit ?? 10;
  renderRecordMeta(s);
}

function renderRecordMeta(s) {
  const meta = document.getElementById('record-meta');
  const langOption = langSelect.querySelector(`option[value="${s.language ?? 'auto'}"]`);
  const langLabel = langOption ? langOption.textContent : (s.language ?? 'Auto-detect');
  const chips = [s.model_name, langLabel];
  meta.innerHTML = chips.map(c => `<span class="meta-chip">${c}</span>`).join('');
}

saveBtn.onclick = async () => {
  try {
    await invoke('save_settings', {
      settings: {
        model_name: modelSelect.value || 'base.en',
        language: langSelect.value === 'auto' ? null : langSelect.value,
        max_duration_secs: parseInt(maxDurInput.value, 10),
        type_at_cursor: typeAtCursorInput.checked,
        history_limit: Math.min(9999, Math.max(1, parseInt(historyLimitInput.value, 10) || 10)),
      }
    });
    showToast('Settings saved');
  } catch (e) {
    showToast(`Error: ${e}`);
  }
};

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
listen('recording-state-changed', (e) => updateBadge(e.payload));

listen('transcription-done', (e) => {
  const text = e.payload;
  lastText.textContent = text;
  lastText.classList.remove('empty');
  copyBtn.disabled = false;
  loadHistory();
  // Switch to record tab so the result is visible
  document.querySelector('.tab-btn[data-tab="record"]').click();
});

listen('transcription-error', (e) => showToast(e.payload));

listen('error-no-model', (e) => {
  showToast(`Model "${e.payload}" not downloaded — go to Settings`);
  document.querySelector('.tab-btn[data-tab="settings"]').click();
});

listen('download-progress', (e) => {
  const { model, downloaded, total, done } = e.payload;
  const prog = document.getElementById(`prog-${model}`);
  if (prog && total > 0) prog.value = Math.round((downloaded / total) * 100);
  if (done) loadModels();
});

// Init
(async () => {
  await loadModels();
  await loadSettings();
  await loadHistory();
  const status = await invoke('get_status');
  updateBadge(status.state);
  if (status.last_result) {
    lastText.textContent = status.last_result;
    lastText.classList.remove('empty');
    copyBtn.disabled = false;
  }
})();
