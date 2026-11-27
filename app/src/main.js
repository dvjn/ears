const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

document.addEventListener('contextmenu', (e) => {
  // Keep the menu for text the user can edit or select (copy, paste).
  if (!e.target.closest('input, textarea, .tx, .h-tx')) e.preventDefault();
});

const $ = (id) => document.getElementById(id);

// ---- Toast ------------------------------------------------------------------

let toastTimer;
function showToast(msg) {
  const el = $('toast');
  el.textContent = msg;
  el.classList.add('show');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => el.classList.remove('show'), 1800);
}

async function copyText(text) {
  try {
    await invoke('copy_text', { text });
    showToast('Copied');
  } catch (e) {
    showToast(`Copy failed: ${e}`);
  }
}

// ---- Tabs -------------------------------------------------------------------

let tab = 'dictate';
function showTab(name) {
  tab = name;
  document.querySelectorAll('.seg button').forEach((b) => {
    const on = b.dataset.tab === name;
    b.classList.toggle('on', on);
    b.setAttribute('aria-pressed', on);
  });
  for (const t of ['dictate', 'history', 'settings']) $(`tab-${t}`).hidden = t !== name;
  setClearConfirm(false);
  if (name === 'history') loadHistory();
}
document.querySelectorAll('.seg button').forEach((b) => b.addEventListener('click', () => showTab(b.dataset.tab)));

let pane = 'stt';
function showPane(name) {
  pane = name;
  document.querySelectorAll('.sub button').forEach((b) => {
    const on = b.dataset.pane === name;
    b.classList.toggle('on', on);
    b.setAttribute('aria-pressed', on);
  });
  for (const p of ['stt', 'llm', 'rec', 'out']) $(`pane-${p}`).hidden = p !== name;
}
document.querySelectorAll('.sub button').forEach((b) => b.addEventListener('click', () => showPane(b.dataset.pane)));

function openSettings(name) {
  showTab('settings');
  showPane(name);
}

$('hide-btn').addEventListener('click', () => {
  window.__TAURI__.window.getCurrentWindow().hide();
});

// ---- Dictate ------------------------------------------------------------------

// phase: idle | recording | transcribing | cleaning | done
//        | err-nospeech | err-mic | err-server | err-key
const dict = {
  phase: 'idle',
  committed: '',
  partial: '',
  last: '',
  started: 0,
  countdown: null,
  error: '',
  level: 0,
};

const BARS = [8, 12, 10, 16, 14, 22, 18, 28, 24, 34, 40];
function makeBars(el, heights) {
  el.replaceChildren(...heights.map((h) => {
    const b = document.createElement('span');
    b.className = 'bar';
    b.style.height = `${h}px`;
    return b;
  }));
}
makeBars($('wave-l'), BARS);
makeBars($('wave-r'), BARS.slice().reverse());

function fmtTime(ms) {
  const secs = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(secs / 60)}:${String(secs % 60).padStart(2, '0')}`;
}

const words = (t) => (t.trim() ? t.trim().split(/\s+/).length : 0);

function micLabel() {
  const id = settings?.input_device ?? '';
  return mics.find((m) => m.id === id)?.label ?? (id ? 'Saved microphone' : 'System default');
}

function renderDictate() {
  const ph = dict.phase;
  const recording = ph === 'recording';
  const busy = ph === 'transcribing' || ph === 'cleaning';

  $('brand-dot').classList.toggle('on', recording);

  const rec = $('rec-btn');
  rec.className = `rec${recording ? ' on' : busy ? ' busy' : ''}`;
  rec.setAttribute('aria-label', recording ? 'Stop recording' : busy ? 'Working' : 'Start recording');
  $('ico-mic').style.display = !recording && !busy ? '' : 'none';
  $('ico-stop').hidden = !recording;
  $('ico-spin').hidden = !busy;
  for (const w of [$('wave-l'), $('wave-r')]) {
    w.classList.toggle('on', recording);
    w.classList.toggle('calm', recording && dict.countdown != null);
  }

  // Status line.
  const autoCopy = settings?.auto_copy ?? true;
  const autoType = settings?.auto_type ?? false;
  let title = 'Ready';
  let cls = '';
  if (recording) {
    title = `${fmtTime(Date.now() - dict.started)} · ${dict.countdown != null ? `stopping in ${dict.countdown} s` : micLabel()}`;
  } else if (ph === 'transcribing') {
    title = 'Transcribing…';
  } else if (ph === 'cleaning') {
    title = 'Cleaning up…';
  } else if (ph === 'done') {
    title = autoCopy && autoType ? 'Copied and typed' : autoType ? 'Typed' : autoCopy ? 'Copied' : 'Done';
    cls = 'ok';
  } else if (ph === 'err-nospeech') {
    title = 'No speech detected'; cls = 'warn';
  } else if (ph === 'err-mic') {
    title = 'Microphone unavailable'; cls = 'err';
  } else if (ph === 'err-server') {
    title = 'Transcription failed'; cls = 'err';
  } else if (ph === 'err-key') {
    title = 'API key needed'; cls = 'err';
  }
  const t = $('st-title');
  t.textContent = title;
  t.className = `st-title ${cls}`;

  // Listen → Transcribe → Clean up → Copy.
  const order = ['recording', 'transcribing', 'cleaning', 'done'];
  const at = order.indexOf(ph);
  const steps = $('steps');
  steps.hidden = at < 0;
  if (at >= 0) {
    const defs = [['recording', 'Listen'], ['transcribing', 'Transcribe']];
    if (settings?.cleanup_enabled) defs.push(['cleaning', 'Clean up']);
    defs.push(['done', autoType && !autoCopy ? 'Type' : 'Copy']);
    steps.replaceChildren(...defs.map(([key, label]) => {
      const i = order.indexOf(key);
      const s = document.createElement('span');
      s.className = `step ${ph === 'done' || i < at ? 'done' : i === at ? 'now' : ''}`;
      s.append(document.createElement('i'), label);
      return s;
    }));
  }

  // Card.
  let committed = dict.last;
  let partial = '';
  let label = 'Last transcription';
  let soft = false;
  let meta = '';
  if (recording) {
    committed = dict.committed;
    partial = dict.partial === '…' ? '' : dict.partial;
    if (committed && partial) partial = ` ${partial}`;
    label = 'Live';
  } else if (busy) {
    committed = dict.committed;
    label = 'As heard';
    soft = true;
  } else if (ph === 'done') {
    label = 'Result';
  }
  if (!recording && !busy && committed) meta = `${words(committed)} words`;
  const has = (committed + partial).trim().length > 0;
  $('card-label').textContent = label;
  $('live-dot').hidden = !recording;
  $('tx').hidden = !has;
  $('tx').classList.toggle('soft', soft);
  $('tx-c').textContent = committed;
  $('tx-p').textContent = partial;
  $('tx-ph').hidden = has;
  $('tx-ph').textContent = recording ? 'Listening…' : busy ? 'Finishing…' : 'Nothing transcribed yet.';
  $('meta').textContent = meta;
  $('copy-btn').disabled = !has || recording || busy;
  if (recording) $('card-body').scrollTop = $('card-body').scrollHeight;

  renderBanner();
}

function renderBanner() {
  const ph = dict.phase;
  const banner = $('banner');
  const a = $('banner-a');
  const b = $('banner-b');
  const sel = $('banner-mic');
  banner.hidden = !ph.startsWith('err-');
  if (banner.hidden) return;
  banner.className = `banner${ph === 'err-nospeech' ? '' : ' err'}`;
  sel.hidden = ph !== 'err-mic';
  b.hidden = false;
  let text = '';
  if (ph === 'err-nospeech') {
    text = 'ears heard only silence, so nothing was copied. Check that the right microphone is selected and not muted.';
    a.textContent = 'Try again'; a.onclick = toggleRecording;
    b.textContent = 'Microphone settings'; b.onclick = () => openSettings('rec');
  } else if (ph === 'err-mic') {
    text = dict.error || "Couldn't open the microphone. Pick another input.";
    a.textContent = 'Try again'; a.onclick = toggleRecording;
    b.hidden = true;
  } else if (ph === 'err-server') {
    const srv = selectedServer('stt');
    text = `${srv?.name ?? 'The server'}: ${dict.error || 'the recording could not be sent.'}`;
    a.textContent = 'Check connection';
    a.onclick = () => { openSettings('stt'); check('stt'); };
    b.textContent = 'Try again'; b.onclick = toggleRecording;
  } else if (ph === 'err-key') {
    text = 'The transcription server needs an API key. Add it in Settings.';
    a.textContent = 'Open settings'; a.onclick = () => openSettings('stt');
    b.hidden = true;
  }
  $('banner-text').textContent = text;
}

async function toggleRecording() {
  try {
    await invoke('cmd_toggle_recording');
  } catch (e) {
    showToast(String(e));
  }
}
$('rec-btn').addEventListener('click', () => {
  if (dict.phase === 'transcribing' || dict.phase === 'cleaning') return;
  toggleRecording();
});
$('copy-btn').addEventListener('click', () => {
  const text = ($('tx-c').textContent + $('tx-p').textContent).trim();
  if (text) copyText(text);
});

// Waveform: the bars follow the microphone level while recording.
function toLevel(rms) {
  const db = 20 * Math.log10(Math.max(rms, 1e-6));
  return Math.min(1, Math.max(0, (db + 55) / 40));
}
const barEls = () => document.querySelectorAll('.wave .bar');
const shown = new Array(BARS.length * 2).fill(0.14);
let lastFrame = performance.now();
let lastTick = 0;
function frame(now) {
  if (dict.phase !== 'recording') {
    barEls().forEach((b) => { b.style.transform = ''; });
    return;
  }
  const dt = Math.min(0.1, (now - lastFrame) / 1000);
  lastFrame = now;
  const t = now / 1000;
  barEls().forEach((b, i) => {
    const wobble = 0.6 + 0.4 * Math.sin(t * (6 + (i % 7) * 1.3) + i * 2.1);
    const goal = 0.14 + 0.86 * dict.level * wobble;
    const rate = goal > shown[i] ? 30 : 8;
    shown[i] += (goal - shown[i]) * (1 - Math.exp(-rate * dt));
    b.style.transform = `scaleY(${shown[i].toFixed(3)})`;
  });
  // The timer in the status line.
  if (now - lastTick > 250) {
    lastTick = now;
    $('st-title').textContent = $('st-title').textContent.replace(/^\d+:\d\d/, fmtTime(Date.now() - dict.started));
  }
  requestAnimationFrame(frame);
}

function setPhase(phase) {
  const was = dict.phase;
  dict.phase = phase;
  if (phase === 'recording' && was !== 'recording') {
    dict.started = Date.now();
    dict.committed = '';
    dict.partial = '';
    dict.countdown = null;
    dict.level = 0;
    lastFrame = performance.now();
    requestAnimationFrame(frame);
  }
  renderDictate();
}

listen('recording-state-changed', (e) => {
  const s = e.payload;
  if (s === 'recording') {
    setPhase('recording');
    showTab('dictate');
  } else if (s === 'transcribing' || s === 'cleaning') {
    setPhase(s);
  } else if (s === 'idle') {
    // transcription-done comes first; errors come right after this.
    if (['recording', 'transcribing', 'cleaning'].includes(dict.phase)) setPhase('idle');
  }
});

listen('transcription-done', (e) => {
  dict.last = e.payload;
  setPhase('done');
  loadHistory();
});

listen('transcription-error', (e) => {
  const { kind, message } = e.payload ?? {};
  dict.error = message ?? '';
  setPhase(kind === 'no-speech' ? 'err-nospeech' : 'err-server');
  showTab('dictate');
});

listen('error-no-api-key', () => {
  setPhase('err-key');
  showTab('dictate');
});

listen('mic-failed', async (e) => {
  dict.error = e.payload;
  setPhase('err-mic');
  showTab('dictate');
  await loadDevices();
});

listen('live-commit', (e) => { dict.committed = e.payload?.text ?? ''; if (dict.phase === 'recording') renderDictate(); });
listen('live-partial', (e) => { dict.partial = e.payload?.text ?? ''; if (dict.phase === 'recording') renderDictate(); });
listen('live-error', (e) => showToast(`Live preview: ${e.payload}`));
listen('mic-level', (e) => { dict.level = toLevel(e.payload); });
listen('silence-countdown', (e) => {
  const left = e.payload ?? null;
  if (left !== dict.countdown) {
    dict.countdown = left;
    if (dict.phase === 'recording') renderDictate();
  }
});

// ---- History ----------------------------------------------------------------

const SVG_COPY = '<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect width="14" height="14" x="8" y="8" rx="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/></svg>';
const SVG_TRASH = '<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 6h18M8 6V4h8v2M6 6l1 14h10l1-14"/></svg>';

function iconButton(svg, label, cls = 'ib') {
  const b = document.createElement('button');
  b.className = cls;
  b.innerHTML = svg;
  b.title = label;
  b.setAttribute('aria-label', label);
  return b;
}

let history = [];
async function loadHistory() {
  try {
    history = await invoke('get_history');
  } catch (e) {
    showToast(`History: ${e}`);
  }
  renderHistory();
}

function renderHistory() {
  const n = history.length;
  const limit = settings?.history_limit ?? 10;
  $('hist-meta').textContent = `${n} ${n === 1 ? 'dictation' : 'dictations'} · keeps the last ${limit}`;
  $('h-empty').hidden = n > 0;
  $('h-list').hidden = n === 0;
  if (!n) setClearConfirm(false);
  $('clear-ask').hidden = n === 0 || clearConfirm;
  $('h-list').replaceChildren(...history.map((text, index) => {
    const item = document.createElement('div');
    item.className = 'h-item';
    const span = document.createElement('span');
    span.className = 'h-tx';
    span.textContent = text;
    span.title = text;
    const copy = iconButton(SVG_COPY, 'Copy');
    copy.onclick = () => copyText(text);
    const remove = iconButton(SVG_TRASH, 'Remove', 'ib danger');
    remove.onclick = async () => {
      try {
        await invoke('remove_history_item', { index });
      } catch (e) {
        showToast(`Remove failed: ${e}`);
      }
      loadHistory();
    };
    item.append(span, copy, remove);
    return item;
  }));
}

let clearConfirm = false;
function setClearConfirm(on) {
  clearConfirm = on;
  $('clear-confirm').hidden = !on;
  $('clear-ask').hidden = on || history.length === 0;
}
$('clear-ask').onclick = () => setClearConfirm(true);
$('clear-cancel').onclick = () => setClearConfirm(false);
$('clear-do').onclick = async () => {
  try {
    await invoke('clear_history');
    showToast('History cleared');
  } catch (e) {
    showToast(`Clear failed: ${e}`);
  }
  setClearConfirm(false);
  loadHistory();
};

// ---- Settings -----------------------------------------------------------------
//
// The backend never sends saved API keys (each server has only `has_api_key`).
// A key field stays blank with a placeholder; a typed key is sent once, and an
// empty one means "keep the saved key". Every change saves at once.

let settings = null;
let mics = [];

const PREFERRED_MODEL = { stt: 'medium.en', llm: 'granite-4.2-3b' };
// role -> { cls: idle|busy|ok|warn|err, text, models, cleanup }
const status = {
  stt: { cls: 'idle', text: 'Not checked yet', models: [], cleanup: [] },
  llm: { cls: 'idle', text: 'Not checked yet', models: [], cleanup: [] },
};

const servers = (role) => settings[`${role}_servers`];
function selectedServer(role) {
  if (!settings) return null;
  const list = servers(role);
  return list.find((s) => s.id === settings[`${role}_selected`]) ?? list[0];
}
const hostOf = (url) => String(url || '').trim().replace(/^https?:\/\//, '').replace(/\/v1\/?$/, '').replace(/\/$/, '') || 'no address yet';

let savedTimer;
async function save() {
  try {
    await invoke('save_settings', { settings });
  } catch (e) {
    showToast(`Not saved: ${e}`);
    return false;
  }
  // A typed key is stored now; never keep or resend it.
  for (const role of ['stt', 'llm']) {
    for (const srv of servers(role)) {
      if (srv.api_key) {
        srv.api_key = '';
        srv.has_api_key = true;
      }
    }
  }
  const el = $('save-text');
  el.textContent = 'Saved';
  el.className = 'ok';
  clearTimeout(savedTimer);
  savedTimer = setTimeout(() => {
    el.textContent = 'Changes save automatically';
    el.className = '';
  }, 1600);
  return true;
}

function newId() {
  return `s${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
}

function field(label, id, input) {
  const row = document.createElement('div');
  row.className = 'svc-f';
  const l = document.createElement('label');
  l.className = 'l';
  l.htmlFor = id;
  l.textContent = label;
  input.id = id;
  row.append(l, input);
  return row;
}

function textInput(type, value, placeholder, mono) {
  const i = document.createElement('input');
  i.className = `ctl${mono ? ' mono' : ''}`;
  i.type = type;
  i.value = value;
  i.placeholder = placeholder;
  i.spellcheck = false;
  i.autocomplete = 'off';
  return i;
}

const SVG_RECHECK = '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 12a9 9 0 1 1-3-6.7L21 8"/><path d="M21 3v5h-5"/></svg>';
const SVG_TRASH14 = SVG_TRASH.replace(/width="13" height="13"/, 'width="14" height="14"');

// The server list for a role: one row per server, the selected one expanded
// with its fields and connection status.
function renderList(role) {
  const list = servers(role);
  const sel = selectedServer(role);
  const items = list.map((srv) => {
    const on = srv.id === sel.id;
    const item = document.createElement('div');
    item.className = `sl-item${on ? ' on' : ''}`;

    const head = document.createElement('div');
    head.className = 'sl-head';
    const row = document.createElement('button');
    row.className = 'sl-row';
    row.setAttribute('aria-pressed', on);
    const name = document.createElement('span');
    name.className = 'sl-name';
    name.textContent = srv.name || 'Unnamed';
    const host = document.createElement('span');
    host.className = 'sl-host';
    host.textContent = hostOf(srv.url);
    row.append(name, host);
    row.onclick = () => {
      if (on) return;
      settings[`${role}_selected`] = srv.id;
      renderList(role);
      save();
      check(role);
    };
    head.append(row);
    if (list.length > 1) {
      const del = iconButton(SVG_TRASH14, 'Remove server', 'ib danger sl-del');
      del.onclick = () => {
        settings[`${role}_servers`] = list.filter((s) => s.id !== srv.id);
        if (on) settings[`${role}_selected`] = settings[`${role}_servers`][0].id;
        renderList(role);
        save();
        if (on) check(role);
        showToast('Server removed');
      };
      head.append(del);
    }
    item.append(head);

    if (on) {
      const body = document.createElement('div');
      body.className = 'sl-body';
      const nameIn = textInput('text', srv.name, 'Name', false);
      nameIn.onchange = () => {
        srv.name = nameIn.value.trim();
        name.textContent = srv.name || 'Unnamed';
        save();
      };
      const urlIn = textInput('text', srv.url, 'https://…/v1', true);
      urlIn.onchange = async () => {
        srv.url = urlIn.value.trim();
        srv.kind = null;
        srv.model = '';
        host.textContent = hostOf(srv.url);
        if (await save()) check(role);
      };
      const keyIn = textInput('password', '', srv.has_api_key ? '••••••••' : 'none', false);
      keyIn.onchange = async () => {
        const key = keyIn.value.trim();
        if (!key) return;
        srv.api_key = key;
        keyIn.value = '';
        if (await save()) {
          keyIn.placeholder = '••••••••';
          check(role);
        }
      };
      const foot = document.createElement('div');
      foot.className = 'svc-foot';
      const stat = document.createElement('div');
      stat.id = `${role}-stat`;
      const dot = document.createElement('i');
      const text = document.createElement('span');
      stat.append(dot, text);
      const recheck = iconButton(SVG_RECHECK, 'Check connection');
      recheck.onclick = () => check(role);
      foot.append(stat, recheck);
      body.append(
        field('Name', `${role}-f-name`, nameIn),
        field('Server URL', `${role}-f-url`, urlIn),
        field('API key', `${role}-f-key`, keyIn),
        foot,
      );
      item.append(body);
    }
    return item;
  });

  const add = document.createElement('button');
  add.className = 'sl-add';
  add.textContent = '+ Add server';
  add.onclick = () => {
    const srv = { id: newId(), name: 'New server', url: '', api_key: '', model: '', kind: null, has_api_key: false };
    servers(role).push(srv);
    settings[`${role}_selected`] = srv.id;
    status[role] = { cls: 'warn', text: 'Enter a server URL', models: [], cleanup: [] };
    renderList(role);
    save();
    document.getElementById(`${role}-f-url`)?.focus();
  };
  $(`${role}-list`).replaceChildren(...items, add);
  renderStatus(role);
}

function renderStatus(role) {
  const st = status[role];
  const el = document.getElementById(`${role}-stat`);
  if (el) {
    el.className = `stat ${st.cls === 'ok' ? '' : st.cls}`;
    el.querySelector('span').textContent = st.text;
    el.title = st.text;
  }
  const show = st.cls === 'warn' || st.cls === 'err';
  const dotOn = role === 'stt' ? show : show && settings?.cleanup_enabled;
  $(`pdot-${role}`).className = `pdot ${dotOn ? st.cls : ''}`;
  renderModels(role);
}

function renderModels(role) {
  const srv = selectedServer(role);
  const st = status[role];
  const models = st.cls === 'ok' || st.cls === 'busy' ? st.models : [];
  const sel = $(`${role}-model`);
  sel.replaceChildren(...models.map((m) => {
    const o = document.createElement('option');
    o.value = m;
    o.textContent = st.cleanup.includes(m) ? `${m} (dictation only)` : m;
    return o;
  }));
  if (srv) sel.value = srv.model;
  if (role === 'stt') $('stt-model-row').hidden = models.length < 2;
  else $('llm-model-grp').hidden = models.length < 2;
  if (role === 'llm') renderCleanup();
}

let checkSeq = { stt: 0, llm: 0 };
async function check(role) {
  const srv = selectedServer(role);
  if (!srv) return;
  const seq = ++checkSeq[role];
  if (!srv.url) {
    status[role] = { cls: 'warn', text: 'Enter a server URL', models: [], cleanup: [] };
    renderStatus(role);
    return;
  }
  status[role] = { ...status[role], cls: 'busy', text: 'Checking…' };
  renderStatus(role);
  try {
    const st = await invoke('check_provider', { role, id: srv.id });
    if (seq !== checkSeq[role]) return;
    srv.kind = st.kind;
    const text = role === 'llm'
      ? 'Connected'
      : st.kind === 'ears' && st.streaming ? 'Connected · live preview' : 'Connected · preview updates at pauses';
    status[role] = { cls: 'ok', text, models: st.models, cleanup: st.cleanup_models };
    // Keep the saved model when the server still has it, else take the usual
    // default or the first one.
    if (st.models.length && !st.models.includes(srv.model)) {
      srv.model = st.models.find((m) => m.includes(PREFERRED_MODEL[role])) ?? st.models[0];
      save();
    }
  } catch (e) {
    if (seq !== checkSeq[role]) return;
    const msg = String(e);
    const warn = /needs an API key|Enter /.test(msg);
    status[role] = { cls: warn ? 'warn' : 'err', text: msg, models: [], cleanup: [] };
  }
  renderStatus(role);
}

for (const role of ['stt', 'llm']) {
  $(`${role}-model`).addEventListener('change', (e) => {
    selectedServer(role).model = e.target.value;
    if (role === 'llm') renderCleanup();
    save();
  });
}

// Clean-up pane: the on/off card first; the rest dims in place when off. A
// cleanup-tuned model uses its own prompt and no screen text.
function renderCleanup() {
  const on = !!settings.cleanup_enabled;
  $('llm-on').checked = on;
  $('llm-rest').classList.toggle('off', !on);
  const model = selectedServer('llm')?.model;
  const tuned = status.llm.cleanup.includes(model);
  $('llm-model-hint').textContent = tuned ? 'Dictation only' : '';
  $('screen').checked = settings.postprocess_use_screenshot && !tuned;
  $('screen').disabled = tuned;
  $('capture-row').hidden = !$('screen').checked;
  $('capture').disabled = tuned;
  $('prompt').disabled = tuned;
  $('prompt-hint').textContent = tuned ? 'Not used by this model' : 'Tells the model how to clean up the transcript';
}

$('llm-on').addEventListener('change', (e) => {
  settings.cleanup_enabled = e.target.checked;
  renderCleanup();
  renderStatus('llm');
  save();
  if (settings.cleanup_enabled) check('llm');
});
$('screen').addEventListener('change', (e) => {
  settings.postprocess_use_screenshot = e.target.checked;
  renderCleanup();
  save();
});
$('capture').addEventListener('change', (e) => { settings.screenshot_method = e.target.value; save(); });
$('prompt').addEventListener('change', (e) => { settings.postprocess_prompt = e.target.value.trim(); save(); });
$('lang').addEventListener('change', (e) => { settings.language = e.target.value === 'auto' ? null : e.target.value; save(); });

function clampInt(el, min, max, fallback) {
  const v = parseInt(el.value, 10);
  const n = Number.isNaN(v) ? fallback : Math.min(max, Math.max(min, v));
  el.value = n;
  return n;
}
$('silence').addEventListener('change', (e) => { settings.silence_stop_secs = clampInt(e.target, 0, 60, 3); save(); });
$('maxdur').addEventListener('change', (e) => { settings.max_duration_secs = clampInt(e.target, 5, 600, 120); save(); });
$('histlimit').addEventListener('change', (e) => { settings.history_limit = clampInt(e.target, 1, 9999, 10); save(); renderHistory(); });

function renderOutput() {
  $('autocopy').checked = settings.auto_copy;
  $('autotype').checked = settings.auto_type;
  $('no-output').hidden = settings.auto_copy || settings.auto_type;
}
$('autocopy').addEventListener('change', (e) => { settings.auto_copy = e.target.checked; renderOutput(); save(); });
$('autotype').addEventListener('change', (e) => { settings.auto_type = e.target.checked; renderOutput(); save(); });

$('copy-cmd').addEventListener('click', () => copyText('ears toggle'));

// Microphones: the Recording pane and the error banner share one list.
async function loadDevices() {
  $('mic-refresh').disabled = true;
  try {
    mics = [{ id: '', label: 'System default' }, ...(await invoke('list_input_devices'))];
  } catch (e) {
    showToast(`Devices: ${e}`);
  } finally {
    $('mic-refresh').disabled = false;
  }
  renderMics();
}

function renderMics() {
  const saved = settings?.input_device ?? '';
  const options = mics.map((m) => [m.id, m.label]);
  if (saved && !mics.some((m) => m.id === saved)) options.push([saved, 'Saved microphone (not connected)']);
  for (const sel of [$('mic'), $('banner-mic')]) {
    sel.replaceChildren(...options.map(([id, label]) => {
      const o = document.createElement('option');
      o.value = id;
      o.textContent = label;
      return o;
    }));
    sel.value = saved;
  }
  if (dict.phase === 'recording') renderDictate();
}

function pickMic(id) {
  settings.input_device = id || null;
  renderMics();
  save();
}
$('mic').addEventListener('change', (e) => pickMic(e.target.value));
$('banner-mic').addEventListener('change', (e) => pickMic(e.target.value));
$('mic-refresh').addEventListener('click', loadDevices);

// The live overlay switched microphones; keep the pickers in step so the
// next save doesn't put the old one back.
listen('input-device-changed', (e) => {
  if (settings) settings.input_device = e.payload ?? null;
  loadDevices();
});

async function loadSettings() {
  settings = await invoke('get_settings');
  $('lang').value = settings.language ?? 'auto';
  $('silence').value = settings.silence_stop_secs;
  $('maxdur').value = settings.max_duration_secs;
  $('histlimit').value = settings.history_limit;
  $('capture').value = settings.screenshot_method ?? 'auto';
  $('prompt').value = settings.postprocess_prompt ?? '';
  renderOutput();
  renderList('stt');
  renderList('llm');
  renderCleanup();
  check('stt');
  check('llm');
  await loadDevices();
}

// ---- Init -------------------------------------------------------------------

(async () => {
  try {
    window.__TAURI__.app.getVersion().then((v) => { $('version').textContent = `ears ${v}`; }).catch(() => {});
    await loadSettings();
    await loadHistory();
    const st = await invoke('get_status');
    if (st.last_result) dict.last = st.last_result;
    setPhase(st.state === 'recording' || st.state === 'transcribing' ? st.state : 'idle');
  } catch (e) {
    showToast(`Init error: ${e}`);
    renderDictate();
  }
})();
