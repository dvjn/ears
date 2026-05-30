const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

document.addEventListener('contextmenu', e => e.preventDefault());

// DOM refs
const stateBadge     = document.getElementById('state-badge');
const lastText       = document.getElementById('last-text');
const copyBtn        = document.getElementById('copy-btn');
const recordBtn      = document.getElementById('record-btn');
const recordLabel    = document.getElementById('record-label');
const historyList    = document.getElementById('history-list');
const modelsList     = document.getElementById('models-list');
const modelSelect    = document.getElementById('model-select');
const langSelect     = document.getElementById('language-select');
const maxDurInput    = document.getElementById('max-duration-input');
const saveBtn        = document.getElementById('save-btn');
const toast          = document.getElementById('toast');

// Toast
let toastTimer;
function showToast(msg) {
  toast.textContent = msg;
  toast.classList.add('show');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => toast.classList.remove('show'), 2500);
}

// State badge
function updateBadge(state) {
  stateBadge.className = `badge ${state}`;
  stateBadge.textContent = state;
  recordLabel.textContent = state === 'recording' ? 'Stop' : state === 'transcribing' ? 'Transcribing...' : 'Record';
  recordBtn.className = `record-btn${state === 'recording' ? ' recording' : ''}`;
  recordBtn.disabled = state === 'transcribing';
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

    item.appendChild(span);
    item.appendChild(copyBtn);
    item.appendChild(removeBtn);
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
}

saveBtn.onclick = async () => {
  try {
    await invoke('save_settings', {
      settings: {
        model_name: modelSelect.value || 'base.en',
        language: langSelect.value === 'auto' ? null : langSelect.value,
        max_duration_secs: parseInt(maxDurInput.value, 10),
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
    showToast(`Error: ${e}`);
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
});

listen('transcription-error', (e) => showToast(`Error: ${e.payload}`));

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
