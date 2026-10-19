# ears

a voice dictation app. press a shortcut, speak, and ears types what you said.
transcription and post-processing run on your own gpu machine with ears-stt
and ears-llm, or on any openai-compatible api.

## pieces

ears comes in three parts you install independently:

- **ears** ([app/](app)): the desktop app. lives in the tray, records when you
  run `ears toggle`, shows a live preview, then types or copies the result.
- **ears-stt** ([stt/](stt)): an openai-compatible speech-to-text server built
  on the system whisper.cpp, with a streaming endpoint for the live preview.
- **ears-llm** ([llm/](llm)): an openai-compatible server built on the
  system llama.cpp, with a catalog of small models (all under 3 gb of vram)
  that clean up transcripts.

run the two servers on one gpu machine and point every other machine's app at
it, or skip them and use a hosted api.

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

on arch, from the aur: `ears-bin`, and on the gpu machine `ears-stt-bin` and
`ears-llm-bin` plus a ggml backend (`ggml-cuda` or `ggml-vulkan`). other
platforms: download the app from the
[releases](https://github.com/dvjn/ears/releases) page.

```sh
# gpu machine
install -Dm600 /usr/share/doc/ears-stt/env.example ~/.config/ears-stt/env
install -Dm600 /usr/share/doc/ears-llm/env.example ~/.config/ears-llm/env
systemctl --user enable --now ears-stt ears-llm

# every machine with the app
systemctl --user enable --now ears
```

ears-stt listens on port 13278 and ears-llm on 13279, on 127.0.0.1 by
default. to serve other machines, set `EARS_STT_HOST` / `EARS_LLM_HOST` to a
lan or tailscale address in their env files; they then require an api key
(`EARS_STT_API_KEY` / `EARS_LLM_API_KEY`).

## usage

1. open settings from the tray. under **transcription**, pick ears-stt or an
   openai-compatible api, its url, key and model. under **post-processing**,
   optionally do the same for ears-llm or another api.
2. bind `ears toggle` to a hotkey (single-instance, so a second invocation
   toggles the running app).
3. speak. when you pause long enough, your words turn into text.

## post-processing and screen context

off by default. when on, the finished transcript goes to a language model
before it is typed or copied. general models follow the editable system
prompt; models trained only to fix dictation (like `speakoflow-mini`) use
their own prompt.

**use screenshot as context** captures the screen (with grim on wlroots
compositors, otherwise the desktop's screenshot portal) and runs local ocr, so
on-screen jargon and names are spelled correctly. this sends text read from
your screen to the post-processing provider. if any step fails, ears falls
back to the raw transcript.

## microphone

ears records through pulseaudio (pipewire-pulse on pipewire desktops), so the
microphone picker shows your sources by name; without it, ears uses the
system default. if the microphone sends no audio, ears reopens it a few times
and then asks you to reconnect it or pick another one.

## develop

```sh
mise install                      # rust, node, pnpm
cd app && pnpm install && pnpm tauri dev
cargo test --workspace            # needs whisper-cpp for ears-stt
```

## license

[mit](LICENSE)
