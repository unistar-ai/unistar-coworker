# Local models (reference tier)

unistar-coworker is optimized for **25B+** models running locally (Ollama or any OpenAI-compatible API). Smaller models may work; `doctor` warns below **25B** but does not block.

## Reference combinations

| Model | Ollama tag (example) | `context_limit` | `tool_mode` | Notes |
|-------|----------------------|-----------------|-------------|-------|
| **gemma 26B A4B** | `gemma4:26b-a4b-it-qat` | `64000` | `auto` or `native` | Default in `coworker.example.yaml` |
| **qwen3.6 27B** | `qwen3.6:27b` | `64000` | `auto` or `native` | Good coding + tool calling |
| **70B class** | `llama3.1:70b` (quantized) | `128000` | `auto` | Use when VRAM allows; slower first token |

**Hardware (comfortable, not required):** 24GB+ VRAM or Apple Silicon 48GB+ unified memory for 26B–27B Q4.

## Recommended `chat` settings (25B+)

```yaml
llm:
  default:
    base_url: http://localhost:11434/v1
    model: gemma4:26b-a4b-it-qat   # or qwen3.6:27b
    context_limit: 64000

chat:
  workspace: .
  tool_mode: auto    # auto | native — reference tier; use lazy only if VRAM tight
  # max_turns: 0     # 0 = unlimited LLM steps (default)
  # max_tool_calls: 0
  # max_duration_secs: 900
  # llm_step_timeout_secs: 180   # raise for slow local 25B+ first token
```

| Knob | Default | Guidance |
|------|---------|----------|
| `tool_mode` | `auto` | **25B+:** `auto` or `native` exposes full tool schemas; `lazy` defers harness tools to `tool_search` |
| `context_limit` | `64000` | Match model window; use `128000` for 70B+ long sessions |
| `max_turns` / `max_tool_calls` | `0` (unlimited) | Cap only if you need hard bounds on runaway loops |
| `llm_step_timeout_secs` | `180` | Local 25B+ may need 60–180s before first token on cold load |

## Profiles

Use named `llm:` entries and `llm_profile` (Web Config or RPC `switch_profile`):

- `gemma-26b` / `qwen-27b` — local coding (see `coworker.example.yaml` comments)
- `deepseek` or remote API — faster inference when local GPU is busy

## Skills and tone

- **`general-agent-tone`** — always-on default style (tool-grounded, concise).
- **`github-ops-tone`** — optional; `skill_load` when doing GitHub/CI ops, not for pure workspace chat.

## Verify setup

```bash
ollama pull gemma4:26b-a4b   # or qwen3.6:27b
./unistar-coworker doctor
./unistar-coworker serve
```

`doctor` reports `llm-model` / `llm-context` when below the reference tier.

See also: [QUICKSTART.md](../QUICKSTART.md), [coworker.minimal.yaml](../coworker.minimal.yaml), [troubleshooting.md](./troubleshooting.md).
