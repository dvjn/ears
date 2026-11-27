# ears-stt

The speech-to-text server for [ears](../README.md). It speaks the OpenAI
transcription API, so other clients can use it too, and adds a streaming
endpoint that gives ears its live preview.

It is released for Linux x86_64 only, as a static binary. The ears app on any
OS can use it over the network. [whisper.cpp](https://github.com/ggml-org/whisper.cpp)
(a git submodule in `vendor/`) is built in, so it doesn't depend on the
system's whisper.cpp or ggml. The GPU is used through Vulkan, for NVIDIA, AMD
and Intel cards alike.

## run

On Arch, install `ears-stt` from the
[`[ears]` repository](../README.md#the-ears-pacman-repository):

```sh
sudo pacman -Syu ears-stt
install -Dm600 /usr/share/doc/ears-stt/env.example ~/.config/ears-stt/env
systemctl --user enable --now ears-stt
```

On other distributions, unpack the `ears-stt-*-x86_64.tar.gz` release asset
and, from inside it:

```sh
sudo install -Dm755 ears-stt /usr/bin/ears-stt
install -Dm644 ears-stt.service ~/.config/systemd/user/ears-stt.service
install -Dm600 env.example ~/.config/ears-stt/env
systemctl --user enable --now ears-stt
```

It needs the Vulkan loader and the GPU's Vulkan driver (`nvidia-utils`,
`vulkan-radeon` or `vulkan-intel` on Arch); without a driver it runs on the
CPU. ffmpeg is needed only for audio that isn't 16 kHz mono wav, which is
what ears sends.

## configure

Settings are environment variables in `~/.config/ears-stt/env`, described in
[env.example](env.example).

- The server listens on `127.0.0.1:13278`. To serve other machines, set
  `EARS_STT_HOST` to a LAN or Tailscale address and set `EARS_STT_API_KEY`;
  it refuses to listen on a non-loopback address without an api key.
- `EARS_STT_MODEL` is the model loaded at startup (default
  `whisper-medium.en`). A request may name any id from `GET /v1/models`.
- Models are downloaded from Hugging Face on first use, so the first start
  takes a while. They are stored in `$XDG_DATA_HOME/ears-stt/models`
  (default `~/.local/share/ears-stt/models`).

## endpoints

- `GET /health`: service name, version, default model, streaming support
  (no auth)
- `GET /v1/models`
- `POST /v1/audio/transcriptions`: OpenAI-compatible multipart upload with
  `file`, `model`, `language`, `prompt`, `temperature` and `response_format`
  (`json` or `text`). 16 kHz mono wav is read directly; other formats go
  through ffmpeg.
- `WS /v1/audio/transcriptions/stream`: live transcription. Send 16 kHz mono
  s16le pcm as binary frames and `{"type":"close"}` to finish; receive
  `{"type":"partial"|"commit","text":...}` as text firms up and finally
  `{"type":"final"}`. Query params `model` and `language`; auth with an
  `Authorization: Bearer` header or `?api_key=`.

## develop

Needs cmake (from `mise install`), clang for bindgen, and the Vulkan
headers, loader, SPIR-V headers and glslc (Arch: `vulkan-headers
vulkan-icd-loader spirv-headers shaderc`). Without them, build CPU-only with
`--no-default-features`. Set `EARS_GGML_NATIVE=1` to tune the CPU code for
the build machine instead of x86-64-v3.

```sh
git submodule update --init
cargo run -p ears-stt
cargo test -p ears-stt
cargo run -p ears-stt --example stream -- some-16k-mono.wav
```
