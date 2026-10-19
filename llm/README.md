# ears-llm

OpenAI-compatible chat server that cleans up [ears](../README.md)
transcripts. It links the system [llama.cpp](https://github.com/ggml-org/llama.cpp)
library; GPU support comes from whichever ggml backend package is installed
(`ggml-cuda`, `ggml-vulkan`, ...). Models are listed in
[models.toml](models.toml) and loaded one at a time.

## endpoints

- `GET /health`: service name, version, default and loaded model (no auth)
- `GET /v1/models`: the catalog; each entry has `ears.kind` (`general` or
  `cleanup`) and a description
- `POST /v1/chat/completions`: OpenAI-compatible, non-streaming. Besides
  plain `messages`, a request can send
  `"ears": {"transcript", "instructions", "context"}` and the server builds
  the prompt for the model: the instructions plus screen context (as reference
  only) and the fenced transcript for a general model, or the model's own
  fixed prompt and the bare transcript for a cleanup-tuned one.

## models

| id | kind | VRAM |
|---|---|---|
| `granite-4.2-3b` (default) | general | 2.7 GB |
| `qwen3.5-2b` | general | 1.9 GB |
| `qwen3.5-0.8b` | general | 1.2 GB |
| `speakoflow-mini` | cleanup | 1.5 GB |

VRAM measured on an RTX 3060 Ti at 4k context, including CUDA overhead.
To change the set, copy `models.toml` to `~/.config/ears-llm/models.toml`:
each model is a Hugging Face `repo` and `file` (or a local `path`), with an
optional `kind`, `system_prompt` (required for cleanup models),
`assistant_prefix` and `context`.

## run

```sh
install -Dm600 env.example ~/.config/ears-llm/env
systemctl --user enable --now ears-llm
```

Settings are environment variables, see [env.example](env.example). The
default port is 13279. The server refuses to listen on a non-loopback address
without an api key.

## develop

Needs `llama-cpp` (headers and library, found with pkg-config) and clang for
bindgen.

```sh
EARS_LLM_MODELS=llm/models.toml cargo run -p ears-llm
cargo test -p ears-llm
```
