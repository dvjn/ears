# ears

voice dictation for linux, macos and windows. press a hotkey, speak, and what
you said is copied to the clipboard or typed at the cursor.

the speech-to-text runs where you choose: on your own gpu machine with the
bundled servers, or on any openai-compatible api.

- live preview while you talk
- one accurate pass over the whole recording for the final text
- stops by itself when you go quiet
- optional clean-up by a language model: spelling, punctuation, filler
  words, mis-heard names, optionally using text read from your screen as
  context
- separate providers for transcription and clean-up
- local history of past transcriptions

## how it fits together

ears is three programs. you always need the first; the other two are
optional.

- **ears** ([app/](app)): the desktop app, for linux, macos and windows. it
  lives in the tray, records the microphone, shows the live preview and
  delivers the text.
- **ears-stt** ([stt/](stt)): a speech-to-text server built on whisper.cpp.
  it speaks the openai api, plus a streaming endpoint for the live preview.
- **ears-llm** ([llm/](llm)): a chat server built on llama.cpp, with a
  catalog of small models (all under 3 gb of vram) that clean up
  transcripts. it speaks the openai api too.

the servers are released for linux x86_64 only, as static binaries that use
the gpu through vulkan. so there are two ways to run ears:

- **with your own servers**: run ears-stt (and ears-llm, if you want
  clean-up) on one linux gpu machine. the app on that machine finds them at
  their default addresses; apps on your other machines, whatever their os,
  reach them over the network.
- **with a hosted api**: skip the servers and give the app the url and key
  of any openai-compatible api.

## get started

### 1. install the app

packages are on the [releases](https://github.com/dvjn/ears/releases) page.

- **arch linux**: add the [`[ears]` repository](#the-ears-pacman-repository),
  then `sudo pacman -Syu ears`.
- **other linux**: the `.deb`, `.rpm` or `.AppImage`.
- **macos**: the `.dmg` for apple silicon (`aarch64`) or intel (`x64`). the
  app isn't notarized, so the first launch is blocked; allow it under system
  settings → privacy & security → open anyway.
- **windows**: the `.msi` or the `-setup.exe`. the installer isn't signed,
  so smartscreen may warn; choose more info → run anyway.

start ears. it has no window at first, only a tray icon.

### 2. choose a transcription provider

- **your own server**: [install ears-stt](#the-servers) on a linux gpu
  machine.
- **a hosted api**: have its base url and api key ready.

then choose settings in the tray menu. in the settings tab, the first
**provider** is for transcription:

| | provider | base url | api key |
|---|---|---|---|
| ears-stt on this machine | ears-stt | `http://localhost:13278/v1` (the default) | none |
| ears-stt on another machine | ears-stt | `http://<host>:13278/v1` | the server's `EARS_STT_API_KEY` |
| a hosted api | openai-compatible | the api's base url (preset: `https://api.openai.com/v1`) | your key |

then pick a **model**; the ↻ button fetches the list from the provider.

### 3. dictate

click the tray icon, or choose start recording from its menu. speak. the
live preview shows your words as they are recognised. the recording ends
when you click again, after 3 seconds of silence, or after 2 minutes; the
**stop on silence** and **max duration** settings change both limits.

the text is copied to the clipboard and shown in a notification. to have it
typed at the cursor instead of pasting it yourself, turn on **auto type**.

- **macos**: allow ears when it asks for the microphone. for auto type,
  also allow it under system settings → privacy & security → accessibility.
- **windows**: ears can't type into apps running as administrator. with
  **auto copy** on, the result is still on the clipboard.

### 4. bind a hotkey

`ears toggle` starts a recording, and stops one that is running. ears is
single-instance, so the command talks to the app already in the tray. bind
it to a key:

- **linux**: `ears toggle` in your desktop's or compositor's keyboard
  shortcuts.
- **macos**: a shortcut in the shortcuts app with a "run shell script"
  action running `/Applications/ears.app/Contents/MacOS/ears toggle`, and a
  keyboard shortcut assigned to it.
- **windows**: a desktop shortcut to `ears.exe toggle` with a shortcut key
  set in its properties. `ears.exe` is in `%LOCALAPPDATA%\ears` (setup
  installer) or `C:\Program Files\ears` (msi).

### 5. start ears at login

- **linux**: the arch, deb and rpm packages include a systemd user unit:
  `systemctl --user enable --now ears`. with the AppImage, add it to your
  desktop's autostart instead.
- **macos**: add ears under system settings → general → login items.
- **windows**: add ears under settings → apps → startup, or put a shortcut
  in the startup folder (`shell:startup`).

## the servers

ears-stt and ears-llm run on a linux x86_64 machine. they need the gpu's
vulkan driver (`nvidia-utils`, `vulkan-radeon` or `vulkan-intel`); without
one they run on the cpu.

on arch, add the [`[ears]` repository](#the-ears-pacman-repository), then:

```sh
sudo pacman -Syu ears-stt ears-llm
install -Dm600 /usr/share/doc/ears-stt/env.example ~/.config/ears-stt/env
install -Dm600 /usr/share/doc/ears-llm/env.example ~/.config/ears-llm/env
systemctl --user enable --now ears-stt ears-llm
```

on other distributions, the `ears-stt-*-x86_64.tar.gz` and
`ears-llm-*-x86_64.tar.gz` release assets hold the binary, the systemd unit
and the example env file. [stt/README.md](stt/README.md#run) and
[llm/README.md](llm/README.md#run) show where to put them.

the first start downloads the default model, so it takes a while.

ears-stt listens on port 13278 and ears-llm on 13279, on 127.0.0.1 by
default. to serve other machines, set `EARS_STT_HOST` / `EARS_LLM_HOST` to a
lan or tailscale address in their env files; they then require an api key
(`EARS_STT_API_KEY` / `EARS_LLM_API_KEY`).

## the `[ears]` pacman repository

every release updates a signed pacman repository with the `ears`, `ears-stt`
and `ears-llm` packages. to add it, trust the signing key and name the
repository in `pacman.conf`:

```sh
curl -fsSL https://raw.githubusercontent.com/dvjn/ears/main/packaging/ears.gpg | sudo pacman-key --add -
sudo pacman-key --lsign-key 4DDA82BEE1876C012B370C206EE7D0F38E85F53D
printf '\n[ears]\nServer = https://github.com/dvjn/ears/releases/latest/download\n' | sudo tee -a /etc/pacman.conf
```

## clean-up and screen context

post-processing is off by default. to turn it on, pick a second **provider**
under post-processing in the settings tab: ears-llm (port 13279) or an
openai-compatible api, set up like the transcription provider. the finished
transcript then goes to a language model before it is copied or typed.

general models follow the **system prompt**, which you can edit. models
trained only to fix dictation (like `speakoflow-mini`) use their own prompt.

**use screenshot** captures the screen and reads its text with local ocr, so
on-screen jargon and names are spelled correctly. this sends text read from
your screen to the post-processing provider.

- **linux**: grim on wlroots compositors, the desktop's screenshot portal
  elsewhere. the **screen capture** setting picks between them.
- **macos**: the primary monitor. macos asks for the screen recording
  permission.
- **windows**: the primary monitor.

the **screen capture** setting does nothing outside linux. if any step of
the clean-up fails, ears falls back to the raw transcript.

## microphone

the **microphone** setting lists your input devices by name. on linux that
needs pulseaudio (pipewire-pulse on pipewire desktops); on plain alsa, ears
offers only the system default.

if the microphone sends no audio, ears reopens it a few times and then asks
you to reconnect it or pick another one.

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
