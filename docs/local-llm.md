# Local LLM Support

Moltis can run LLM inference locally on your machine without requiring an API
key or internet connection. This enables fully offline operation and keeps your
conversations private.

## Backends

Moltis supports two backends for local inference:

| Backend | Format | Platform | GPU Acceleration |
|---------|--------|----------|------------------|
| **GGUF** (llama.cpp) | `.gguf` files | macOS, Linux, Windows | Metal (macOS), CUDA (NVIDIA) |
| **MLX** | MLX model repos | macOS (Apple Silicon only) | Apple Silicon neural engine |

### GGUF (llama.cpp)

GGUF is the primary backend, powered by [llama.cpp](https://github.com/ggerganov/llama.cpp).
It supports quantized models in the GGUF format, which significantly reduces
memory requirements while maintaining good quality.

**Advantages:**
- Cross-platform (macOS, Linux, Windows)
- Wide model compatibility (any GGUF model)
- GPU acceleration on both NVIDIA (CUDA) and Apple Silicon (Metal)
- Mature and well-tested

### MLX

MLX is Apple's machine learning framework optimized for Apple Silicon. Models
from the [mlx-community](https://huggingface.co/mlx-community) on HuggingFace
are specifically optimized for M1/M2/M3/M4 chips.

**Advantages:**
- Native Apple Silicon performance
- Efficient unified memory usage
- Lower latency on Macs

**Requirements:**
- macOS with Apple Silicon (M1/M2/M3/M4)

## Memory Requirements

Models are organized by memory tiers based on your system RAM:

| Tier | RAM | Recommended Models |
|------|-----|-------------------|
| **Tiny** | 4GB | Qwen 2.5 Coder 1.5B, Llama 3.2 1B |
| **Small** | 8GB | Qwen 2.5 Coder 3B, Llama 3.2 3B |
| **Medium** | 16GB | Qwen 2.5 Coder 7B, Llama 3.1 8B |
| **Large** | 32GB+ | Qwen 2.5 Coder 14B, DeepSeek Coder V2 Lite |

Moltis automatically detects your system memory and suggests appropriate models
in the UI.

## Configuration

### Via Web UI (Recommended)

1. Navigate to **Providers** in the sidebar
2. Click **Add Provider**
3. Select **Local LLM**
4. Choose a model from the registry or search HuggingFace
5. Click **Configure** — the model will download automatically

### Via Configuration File

Add to `~/.moltis/moltis.toml`:

```toml
[providers.local]
model = "qwen2.5-coder-7b-q4_k_m"
```

For custom GGUF files:

```toml
[providers.local]
model = "my-custom-model"
model_path = "/path/to/model.gguf"
```

## Model Storage

Downloaded models are cached in `~/.cache/moltis/models/` by default. This
directory can grow large (several GB per model).

To change the cache location:

```toml
[providers.local]
cache_dir = "/custom/models/path"
```

## HuggingFace Integration

You can search and download models directly from HuggingFace:

1. In the Add Provider dialog, click "Search HuggingFace"
2. Enter a search term (e.g., "qwen coder")
3. Select GGUF or MLX backend
4. Choose a model from the results
5. The model will be downloaded on first use

### Finding GGUF Models

Look for repositories with "GGUF" in the name on HuggingFace:
- [TheBloke](https://huggingface.co/TheBloke) — large collection of quantized models
- [bartowski](https://huggingface.co/bartowski) — Llama 3.x GGUF models
- [Qwen](https://huggingface.co/Qwen) — official Qwen GGUF models

### Finding MLX Models

MLX models are available from [mlx-community](https://huggingface.co/mlx-community):
- Pre-converted models optimized for Apple Silicon
- Look for models ending in `-4bit` or `-8bit` for quantized versions

## GPU Acceleration

### Metal (macOS)

Metal acceleration is enabled by default on macOS. The number of GPU layers
can be configured:

```toml
[providers.local]
gpu_layers = 99  # Offload all layers to GPU
```

### CUDA (NVIDIA)

Requires building with the `local-llm-cuda` feature:

```bash
cargo build --release --features local-llm-cuda
```

## Limitations

Local LLM models have some limitations compared to cloud providers:

1. **No tool calling** — Local models don't support function/tool calling.
   When using a local model, features like file operations, shell commands,
   and memory search are disabled.

2. **Slower inference** — Depending on your hardware, local inference may be
   significantly slower than cloud APIs.

3. **Quality varies** — Smaller quantized models may produce lower quality
   responses than larger cloud models.

4. **Context window** — Local models typically have smaller context windows
   (8K-32K tokens vs 128K+ for cloud models).

## Chat Templates

Different model families use different chat formatting. Moltis automatically
detects the correct template for registered models:

- **ChatML** — Qwen, many instruction-tuned models
- **Llama 3** — Meta's Llama 3.x family
- **DeepSeek** — DeepSeek Coder models

For custom models, the template is auto-detected from the model metadata when
possible.

## Troubleshooting

### Model fails to load

- Check you have enough RAM (see memory tier table above)
- Verify the GGUF file isn't corrupted (re-download if needed)
- Ensure the model file matches the expected architecture

### Slow inference

- Enable GPU acceleration (Metal on macOS, CUDA on Linux)
- Try a smaller/more quantized model
- Reduce context size in config

### Out of memory

- Choose a model from a lower memory tier
- Close other applications to free RAM
- Use a more aggressively quantized model (Q4_K_M vs Q8_0)

## Feature Flag

Local LLM support requires the `local-llm` feature flag at compile time:

```bash
cargo build --release --features local-llm
```

This is enabled by default in release builds.
