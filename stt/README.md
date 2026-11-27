# ears-stt

OpenAI-compatible speech-to-text server for [ears](../README.md). It builds
[whisper.cpp](https://github.com/ggml-org/whisper.cpp) (a git submodule in
`vendor/`) in statically, so it doesn't depend on the system's whisper.cpp or
ggml. GPU support is Vulkan, for NVIDIA, AMD and Intel cards alike.

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

## run

```sh
install -Dm600 env.example ~/.config/ears-stt/env
systemctl --user enable --now ears-stt
```

Settings are environment variables, see [env.example](env.example). The
default port is 13278. The server refuses to listen on a non-loopback address
without an api key. Models are downloaded from Hugging Face on first use.

## develop

Needs cmake (from `mise install`), clang for bindgen, and the Vulkan
headers, loader, SPIR-V headers and glslc (Arch: `vulkan-headers
vulkan-icd-loader spirv-headers shaderc`). Without them, build CPU-only with
`--no-default-features`. Set `EARS_GGML_NATIVE=1` to tune the CPU code for
the build machine instead of x86-64-v3.

```sh
cargo run -p ears-stt
cargo test -p ears-stt
cargo run -p ears-stt --example stream -- some-16k-mono.wav
```
