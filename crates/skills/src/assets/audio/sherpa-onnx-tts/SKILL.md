---
name: sherpa-onnx-tts
description: Local text-to-speech via sherpa-onnx (offline, no cloud)
origin:
  source: openclaw
  url: https://github.com/openclaw/openclaw
  version: 90eb5b073f
---

# sherpa-onnx-tts

Local TTS using the sherpa-onnx offline CLI.

## Install

1. Download the runtime for your OS (extracts into `$OPENCLAW_STATE_DIR/tools/sherpa-onnx-tts/runtime`, default `~/.moltis/tools/sherpa-onnx-tts/runtime`)
2. Download a voice model (extracts into `$OPENCLAW_STATE_DIR/tools/sherpa-onnx-tts/models`, default `~/.moltis/tools/sherpa-onnx-tts/models`)

Resolve the active state directory first:

```bash
STATE_DIR="${OPENCLAW_STATE_DIR:-$HOME/.Moltis}"
```

Then write those resolved paths into the active Moltis config file (`$OPENCLAW_CONFIG_PATH`, default `~/.moltis/Moltis.json`):

```json5
{
  skills: {
    entries: {
      "sherpa-onnx-tts": {
        env: {
          SHERPA_ONNX_RUNTIME_DIR: "/path/to/your/state-dir/tools/sherpa-onnx-tts/runtime",
          SHERPA_ONNX_MODEL_DIR: "/path/to/your/state-dir/tools/sherpa-onnx-tts/models/vits-piper-en_US-lessac-high",
        },
      },
    },
  },
}
```

The wrapper lives in this skill folder. Run it directly, or add the wrapper to PATH:

```bash
export PATH="{baseDir}/bin:$PATH"
```

## Usage

```bash
{baseDir}/bin/sherpa-onnx-tts -o ./tts.wav "Hello from local TTS."
```

Notes:

- Pick a different model from the sherpa-onnx `tts-models` release if you want another voice.
- If the model dir has multiple `.onnx` files, set `SHERPA_ONNX_MODEL_FILE` or pass `--model-file`.
- You can also pass `--tokens-file` or `--data-dir` to override the defaults.
- Windows: run `node {baseDir}\\bin\\sherpa-onnx-tts -o tts.wav "Hello from local TTS."`
