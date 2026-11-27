# ears-llm

The clean-up server for [ears](../README.md): a language model that fixes
spelling, punctuation and mis-heard words in a dictated transcript. It speaks
the OpenAI chat API and serves a small catalog of models, loaded one at a
time.

It is released for Linux x86_64 only, as a static binary. The ears app on any
OS can use it over the network. [llama.cpp](https://github.com/ggml-org/llama.cpp)
(a git submodule in `vendor/`) is built in, so it doesn't depend on the
system's llama.cpp or ggml. The GPU is used through Vulkan, for NVIDIA, AMD
and Intel cards alike.

## run

On Arch, install `ears-llm` from the
`[ears]` repository (see [the servers](../README.md#the-servers) to add it):

```sh
sudo pacman -Syu ears-llm
install -Dm600 /usr/share/doc/ears-llm/env.example ~/.config/ears-llm/env
systemctl --user enable --now ears-llm
```

On other distributions, unpack the `ears-llm-*-x86_64.tar.gz` release asset
and, from inside it:

```sh
sudo install -Dm755 ears-llm /usr/bin/ears-llm
install -Dm644 models.toml ~/.config/ears-llm/models.toml
install -Dm644 ears-llm.service ~/.config/systemd/user/ears-llm.service
install -Dm600 env.example ~/.config/ears-llm/env
systemctl --user enable --now ears-llm
```

It needs the Vulkan loader and the GPU's Vulkan driver (`nvidia-utils`,
`vulkan-radeon` or `vulkan-intel` on Arch); without a driver it runs on the
CPU.

## configure

Settings are environment variables in `~/.config/ears-llm/env`, described in
[env.example](env.example).

- The server listens on `127.0.0.1:13279`. To serve other machines, set
  `EARS_LLM_HOST` to a LAN or Tailscale address and `EARS_LLM_API_KEY` to a
  secret of your choice (`openssl rand -hex 32`); the apps send the same key.
  The server refuses to listen on a non-loopback address without one.
- The first start downloads the default model, so it takes a while. Models
  are stored in `$XDG_DATA_HOME/ears-llm/models` (default
  `~/.local/share/ears-llm/models`).

## models

| id | kind | VRAM |
|---|---|---|
| `granite-4.2-3b` (default) | general | 2.7 GB |
| `qwen3.5-2b` | general | 1.9 GB |
| `qwen3.5-0.8b` | general | 1.2 GB |
| `speakoflow-mini` | cleanup | 1.5 GB |

VRAM measured on an RTX 3060 Ti at 4k context, including GPU runtime overhead.

A general model follows the instructions it is given. A cleanup model is
trained only to fix dictation and uses its own fixed prompt.

The catalog is [models.toml](models.toml). To change the set, copy it to
`~/.config/ears-llm/models.toml`: each model is a Hugging Face `repo` and
`file` (or a local `path`), with an optional `kind`, `system_prompt`
(required for cleanup models), `assistant_prefix` and `context`.

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

## develop

Needs cmake (from `mise install`), clang for bindgen, and the Vulkan
headers, loader, SPIR-V headers and glslc (Arch: `vulkan-headers
vulkan-icd-loader spirv-headers shaderc`). Without them, build CPU-only with
`--no-default-features`. Set `EARS_GGML_NATIVE=1` to tune the CPU code for
the build machine instead of x86-64-v3.

```sh
git submodule update --init
EARS_LLM_MODELS=llm/models.toml cargo run -p ears-llm
cargo test -p ears-llm
```
