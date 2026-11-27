# ears

a voice dictation app. press a shortcut, speak, and ears types what you said.
transcription and post-processing run on your own gpu machine with ears-stt
and ears-llm, or on any openai-compatible api.

## pieces

ears comes in three parts you install independently:

- **ears** ([app/](app)): the desktop app, for linux, macos and windows. lives
  in the tray, records when you click it or run `ears toggle`, shows a live
  preview, then types or copies the result.
- **ears-stt** ([stt/](stt)): an openai-compatible speech-to-text server built
  on whisper.cpp, with a streaming endpoint for the live preview.
- **ears-llm** ([llm/](llm)): an openai-compatible server built on llama.cpp,
  with a catalog of small models (all under 3 gb of vram) that clean up
  transcripts.

the servers are released for linux x86_64 only (static binaries using the gpu
through vulkan). run them on one linux gpu machine and point every other
machine's app at it, whatever its os, or skip them and use a hosted api.

## features

- live preview while you talk (streamed from ears-stt, or per pause with any
  openai-compatible api); stops on silence
- one accurate pass over the whole recording for the final text
- types the result at the cursor or copies it to the clipboard
- optional post-processing with a language model: spelling, punctuation,
  mis-heard names, optionally using text read from your screen as context
- separate providers for transcription and post-processing
- microphone picker; a silent mic is reopened, then you're asked to pick
  another
- local history of past transcriptions

## install

### the app

download it from the [releases](https://github.com/dvjn/ears/releases) page.

- **linux**: on arch, `ears-bin` from the [`[ears]` repository](#the-ears-pacman-repository)
  (or the aur); otherwise the `.deb`, `.rpm` or `.AppImage`. the arch, deb
  and rpm packages include a systemd user unit to start ears with your
  session: `systemctl --user enable --now ears`. with the AppImage, add it to
  your desktop's autostart instead.
- **macos**: the `.dmg` for apple silicon (`aarch64`) or intel (`x64`). the
  app isn't notarized, so the first launch is blocked; allow it under system
  settings → privacy & security → open anyway. to start ears at login, add it
  under system settings → general → login items.
- **windows**: the `.msi` or the `-setup.exe`. the installer isn't signed, so
  smartscreen may warn; choose more info → run anyway. to start ears at
  login, add it under settings → apps → startup, or put a shortcut in the
  startup folder (`shell:startup`).

### the servers (linux gpu machine)

on arch, `ears-stt-bin` and `ears-llm-bin` from the
[`[ears]` repository](#the-ears-pacman-repository) (or the aur). they need the
gpu's vulkan driver (`nvidia-utils`, `vulkan-radeon` or `vulkan-intel`);
without one they run on the cpu.

```sh
install -Dm600 /usr/share/doc/ears-stt/env.example ~/.config/ears-stt/env
install -Dm600 /usr/share/doc/ears-llm/env.example ~/.config/ears-llm/env
systemctl --user enable --now ears-stt ears-llm
```

on other distributions, the `ears-stt-*-x86_64.tar.gz` and
`ears-llm-*-x86_64.tar.gz` release assets hold the binary, the systemd unit
and the example env file.

ears-stt listens on port 13278 and ears-llm on 13279, on 127.0.0.1 by
default. to serve other machines, set `EARS_STT_HOST` / `EARS_LLM_HOST` to a
lan or tailscale address in their env files; they then require an api key
(`EARS_STT_API_KEY` / `EARS_LLM_API_KEY`).

### the `[ears]` pacman repository

every release updates a signed pacman repository with `ears-bin`,
`ears-stt-bin` and `ears-llm-bin`:

```sh
# trust the signing key, then add the repository
curl -fsSL https://raw.githubusercontent.com/dvjn/ears/main/packaging/ears.gpg | sudo pacman-key --add -
sudo pacman-key --lsign-key 4DDA82BEE1876C012B370C206EE7D0F38E85F53D
printf '\n[ears]\nServer = https://github.com/dvjn/ears/releases/latest/download\n' | sudo tee -a /etc/pacman.conf
sudo pacman -Sy ears-bin            # and ears-stt-bin ears-llm-bin on the gpu machine
```

## usage

1. open settings from the tray. under **transcription**, pick ears-stt or an
   openai-compatible api, its url, key and model. under **post-processing**,
   optionally do the same for ears-llm or another api.
2. start a recording by clicking the tray icon, or bind `ears toggle` to a
   hotkey (ears is single-instance, so a second invocation toggles the
   running app):
   - **linux**: `ears toggle` in your desktop's or compositor's keyboard
     shortcuts.
   - **macos**: a shortcut in the shortcuts app with a "run shell script"
     action running `/Applications/ears.app/Contents/MacOS/ears toggle`, and
     a keyboard shortcut assigned to it.
   - **windows**: a desktop shortcut to `ears.exe toggle` with a shortcut key
     set in its properties. `ears.exe` is in `%LOCALAPPDATA%\ears` (setup
     installer) or `C:\Program Files\ears` (msi).
3. speak. when you pause long enough, your words turn into text.

on macos, allow ears when it asks for the microphone, and under system
settings → privacy & security → accessibility so it can type the result.
on windows, ears can't type into apps running as administrator; with
auto-copy on, the result is still on the clipboard.

## post-processing and screen context

off by default. when on, the finished transcript goes to a language model
before it is typed or copied. general models follow the editable system
prompt; models trained only to fix dictation (like `speakoflow-mini`) use
their own prompt.

**use screenshot as context** captures the screen and runs local ocr, so
on-screen jargon and names are spelled correctly. on linux it uses grim on
wlroots compositors and the desktop's screenshot portal elsewhere (the
**screenshot method** setting picks; it does nothing on other systems). on
macos and windows it captures the primary monitor; macos asks for the screen
recording permission. this sends text read from
your screen to the post-processing provider. if any step fails, ears falls
back to the raw transcript.

## microphone

the microphone picker lists your input devices by name. on linux that needs
pulseaudio (pipewire-pulse on pipewire desktops); on plain alsa, ears offers
only the system default. if the microphone sends no audio, ears reopens it a
few times and then asks you to reconnect it or pick another one.

## develop

install tauri's [prerequisites](https://tauri.app/start/prerequisites/) for
your os, then:

```sh
mise install                      # rust, node, pnpm, cmake
cd app && pnpm install && pnpm tauri dev
cargo test -p ears                  # the app
```

the servers build on linux only:

```sh
git submodule update --init       # whisper.cpp, llama.cpp
cargo test -p ears-stt -p ears-llm   # needs the vulkan headers, see stt/README.md
```

## license

[mit](LICENSE)
