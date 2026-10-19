# ears-stt

OpenAI-compatible speech-to-text server for [ears](../README.md). It links
the system [whisper.cpp](https://github.com/ggml-org/whisper.cpp); GPU support
comes from whichever ggml backend package is installed (`ggml-cuda`,
`ggml-vulkan`, ...), so nothing is compiled per GPU.

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

Needs `whisper-cpp` (headers and library, found with pkg-config) and clang
for bindgen.

```sh
cargo run -p ears-stt
cargo test -p ears-stt
cargo run -p ears-stt --example stream -- some-16k-mono.wav
```
