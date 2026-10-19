const { listen } = window.__TAURI__.event;
const { invoke } = window.__TAURI__.core;

const textEl = document.getElementById('text');

// Stop button: same toggle the Record button / hotkey uses. In live mode this
// finalizes the dictation (copy / type / history) and hides the overlay.
document.getElementById('stop').addEventListener('click', () => {
  invoke('cmd_toggle_recording').catch(() => {});
});

let committed = '';
let partial = '';

function render() {
  const hasText = (committed + partial).trim().length > 0;
  if (!hasText) {
    textEl.className = 'placeholder';
    textEl.textContent = 'Start speaking…';
    return;
  }
  textEl.className = '';
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
listen('live-reset', () => { committed = ''; partial = ''; render(); });
