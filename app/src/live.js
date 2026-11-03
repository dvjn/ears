const { listen } = window.__TAURI__.event;
const { invoke } = window.__TAURI__.core;

const textEl = document.getElementById('text');

// Stop button: same toggle the Record button / hotkey uses. In live mode this
// finalizes the dictation (copy / type / history) and hides the overlay.
document.getElementById('stop').addEventListener('click', () => {
  invoke('cmd_toggle_recording').catch(() => {});
});

// Microphone picker: hidden behind the mic button, shown in place of the
// transcript.
const micBtn = document.getElementById('mic');
const micsEl = document.getElementById('mics');

function closeMics() {
  micsEl.hidden = true;
  micBtn.classList.remove('open');
}

micBtn.addEventListener('click', async () => {
  if (!micsEl.hidden) {
    closeMics();
    return;
  }
  const [mics, settings] = await Promise.all([
    invoke('list_input_devices'),
    invoke('get_settings'),
  ]);
  const current = settings.input_device ?? '';
  const options = [['', 'System default'], ...mics.map((m) => [m.id, m.label])];
  micsEl.replaceChildren(...options.map(([id, label]) => {
    const btn = document.createElement('button');
    btn.textContent = label;
    btn.title = label;
    if (id === current) btn.className = 'current';
    btn.addEventListener('click', () => {
      closeMics();
      invoke('switch_mic', { id: id || null }).catch(() => {});
    });
    return btn;
  }));
  micsEl.hidden = false;
  micBtn.classList.add('open');
});

let committed = '';
let partial = '';

function render() {
  const hasText = (committed + partial).trim().length > 0;
  if (!hasText) {
    textEl.textContent = '';
    return;
  }
  const sep = committed && partial ? ' ' : '';
  textEl.innerHTML =
    `<span class="committed">${escapeHtml(committed)}</span>` +
    `<span class="partial">${escapeHtml(sep + partial)}</span>`;
  // Keep the most recent text in view as the paragraph grows.
  textEl.scrollTop = textEl.scrollHeight;
}

function escapeHtml(s) {
  return s.replace(/[&<>]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' }[c]));
}

// `live-commit` carries the full transcript so far; `live-partial` the tail.
listen('live-commit', (e) => { committed = e.payload?.text ?? ''; render(); });
listen('live-partial', (e) => { partial = e.payload?.text ?? ''; render(); });

// Reset when a new session begins.
listen('live-reset', () => {
  closeMics();
  committed = '';
  partial = '';
  target = 0;
  heights.fill(0);
  render();
});

// Voice visualizer: a few bars that follow the current mic level, animated
// every display frame.
const wave = document.getElementById('wave');
const ctx = wave.getContext('2d');
const panel = document.getElementById('panel');
const BARS = 7;
const BAR = 4;
const GAP = 3;
// Middle bars reach higher, like a voice meter.
const PROFILE = [0.45, 0.7, 0.9, 1, 0.9, 0.7, 0.45];
let target = 0; // latest mic level, 0..1
const heights = new Array(BARS).fill(0);

// RMS to 0..1 on a log scale: -55 dB (room noise) to -15 dB (loud speech).
function toLevel(rms) {
  const db = 20 * Math.log10(Math.max(rms, 1e-6));
  return Math.min(1, Math.max(0, (db + 55) / 40));
}

listen('mic-level', (e) => { target = toLevel(e.payload); });

let last = performance.now();
let flip = false;
function draw(now) {
  const dt = Math.min(0.1, (now - last) / 1000);
  last = now;
  const dpr = window.devicePixelRatio || 1;
  const w = wave.clientWidth;
  const h = wave.clientHeight;
  if (wave.width !== w * dpr || wave.height !== h * dpr) {
    wave.width = w * dpr;
    wave.height = h * dpr;
  }
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  ctx.fillStyle = getComputedStyle(document.documentElement).getPropertyValue('--accent');
  const t = now / 1000;
  for (let i = 0; i < BARS; i++) {
    // Each bar wobbles at its own rate so speech looks alive, not uniform.
    const wobble = 0.65 + 0.35 * Math.sin(t * (7 + i * 1.7) + i * 2.1);
    const goal = target * PROFILE[i] * wobble;
    // Rise fast, fall slower; frame-rate independent.
    const rate = goal > heights[i] ? 30 : 10;
    heights[i] += (goal - heights[i]) * (1 - Math.exp(-rate * dt));
    const bh = Math.max(BAR, heights[i] * h);
    const x = i * (BAR + GAP);
    ctx.beginPath();
    ctx.roundRect(x, (h - bh) / 2, BAR, bh, BAR / 2);
    ctx.fill();
  }
  // Some Wayland compositors (mango) show only the damaged part of this
  // transparent window, so the rest of the panel flickers away. Touching the
  // whole panel every frame makes each frame a full repaint.
  flip = !flip;
  panel.style.opacity = flip ? '0.999' : '1';
  requestAnimationFrame(draw);
}
requestAnimationFrame(draw);
