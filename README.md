# ears

a voice dictation app. press a shortcut, speak, and ears transcribes what you
said using an openai-compatible api.

## features

- transcribes speech to text as you talk, stops on silence
- works with any openai-compatible transcription api
- triggered by a cli command (`ears toggle`) you can bind to a hotkey
- types the result or copies it to the clipboard
- keeps a local history of past transcriptions

## install

download the installer for your platform from the
[releases](https://github.com/dvjn/ears/releases) page.

## usage

1. open settings from the tray and set your transcription **base url**, **api
   key**, and **model**.
2. toggle recording with `ears toggle` (single-instance, so a second invocation
   toggles the running app). bind it to a global hotkey for push-to-dictate.
3. speak. when you pause, your words turn into text.

## license

[mit](LICENSE)
