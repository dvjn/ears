const { listen } = window.__TAURI__.event;
const { invoke } = window.__TAURI__.core;

const $ = (id) => document.getElementById(id);
const strip = $('strip');
const stopBtn = $('stop');
const micBtn = $('mic');
const pop = $('pop');

// Stop: the same toggle the record button and hotkey use. It finalizes the
// dictation (copy / type / history) and hides the overlay.
stopBtn.addEventListener('click', () => {
  invoke('cmd_toggle_recording').catch(() => {});
});

// Microphone picker: a grid above the strip, in place of the captions.
function closeMics() {
  pop.hidden = true;
  micBtn.classList.remove('open');
  micBtn.setAttribute('aria-expanded', 'false');
  render();
}

micBtn.addEventListener('click', async () => {
  if (!pop.hidden) {
    closeMics();
    return;
  }
  const [mics, settings] = await Promise.all([
    invoke('list_input_devices'),
    invoke('get_settings'),
  ]);
  const current = settings.input_device ?? '';
  const options = [['', 'System default'], ...mics.map((m) => [m.id, m.label])];
  pop.replaceChildren(...options.map(([id, label]) => {
    const btn = document.createElement('button');
    btn.textContent = label;
    btn.title = label;
    if (id === current) btn.className = 'cur';
    btn.addEventListener('click', () => {
      closeMics();
      invoke('switch_mic', { id: id || null }).catch(() => {});
    });
    return btn;
  }));
  pop.hidden = false;
  micBtn.classList.add('open');
  micBtn.setAttribute('aria-expanded', 'true');
  render();
});

document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape' && !pop.hidden) closeMics();
});

// Captions: the committed transcript, then the volatile tail in a lighter
// colour; the box keeps the latest two lines.
let committed = '';
let partial = '';

function render() {
  const tail = partial === '…' ? '' : partial;
  const has = (committed + tail).trim().length > 0;
  $('cap').hidden = !has || !pop.hidden;
  $('cap-c').textContent = committed;
  $('cap-p').textContent = (committed && tail ? ' ' : '') + tail;
}

listen('live-commit', (e) => { committed = e.payload?.text ?? ''; render(); });
listen('live-partial', (e) => { partial = e.payload?.text ?? ''; render(); });

// Waveform history: one bar per sample of the loudest level, scrolling left.
const BARS = 80;
const SAMPLE_MS = 120;
const hist = $('hist');
const bars = Array.from({ length: BARS }, () => document.createElement('span'));
hist.replaceChildren(...bars);
const levels = new Array(BARS).fill(0);
let peak = 0;

// RMS to 0..1 on a log scale: -55 dB (room noise) to -15 dB (loud speech).
function toLevel(rms) {
  const db = 20 * Math.log10(Math.max(rms, 1e-6));
  return Math.min(1, Math.max(0, (db + 55) / 40));
}

listen('mic-level', (e) => { peak = Math.max(peak, toLevel(e.payload)); });

let started = Date.now();
let busy = false;

function setBusy(on) {
  busy = on;
  strip.classList.toggle('busy', on);
  stopBtn.disabled = on;
  if (on) closeMics();
}

function tick() {
  if (!busy) {
    levels.shift();
    levels.push(peak);
    peak = 0;
    bars.forEach((b, i) => { b.style.height = `${Math.round(3 + levels[i] * 29)}px`; });
    const secs = Math.floor((Date.now() - started) / 1000);
    $('tm').textContent = `${Math.floor(secs / 60)}:${String(secs % 60).padStart(2, '0')}`;
  }
}
setInterval(tick, SAMPLE_MS);

// A new session: clear everything and restart the clock.
listen('live-reset', () => {
  committed = '';
  partial = '';
  levels.fill(0);
  peak = 0;
  started = Date.now();
  setBusy(false);
  closeMics();
});

listen('recording-state-changed', (e) => {
  if (e.payload === 'recording') setBusy(false);
  else if (e.payload === 'transcribing' || e.payload === 'cleaning') setBusy(true);
});

// Some Wayland compositors (mango) show only the damaged part of this
// transparent window, so the rest flickers away. Touching the whole root
// every frame makes each frame a full repaint.
const root = $('root');
let flip = false;
function repaint() {
  flip = !flip;
  root.style.opacity = flip ? '0.999' : '1';
  requestAnimationFrame(repaint);
}
requestAnimationFrame(repaint);
