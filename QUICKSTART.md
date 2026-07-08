# Quick start

Get unistar-coworker running in a few minutes. Two install paths: **tar.gz** (native binary) or **Docker**.

**Reference tier:** local **25B+** model via Ollama (e.g. `gemma4:26b-a4b`, `qwen3.6:27b`). GitHub is optional.

Full docs: [README.md](README.md) · Docker details: [docs/docker.md](docs/docker.md)

---

## Path A — tar.gz (Linux x86_64 or macOS arm64)

### Core — local agent (no GitHub required)

1. Download the latest `unistar-coworker-*-*.tar.gz` from [GitHub Releases](https://github.com/unistar-ai/unistar-coworker/releases).
2. Extract: `tar -xzf unistar-coworker-*.tar.gz && cd unistar-coworker-*`
3. Make the binary executable: `chmod +x unistar-coworker`
4. Start Ollama and pull a **25B+** model, e.g. `ollama pull gemma4:26b-a4b` or `ollama pull qwen3.6:27b` (see [coworker.example.yaml](./coworker.example.yaml)).
5. Create config: `./unistar-coworker init --interactive`  
   (Non-interactive: `./unistar-coworker init --llm-url http://127.0.0.1:11434/v1` — skip `--repos` for workspace-only.)
6. Check health: `./unistar-coworker doctor`
7. Start Web UI: `./unistar-coworker serve`
8. Open [http://127.0.0.1:8787](http://127.0.0.1:8787) and chat in the workspace (`chat.workspace` in config).

### Optional — GitHub workflows

9. Set GitHub auth: `export GH_TOKEN=...` or run `gh auth login` on the host.
10. Add `repos:` to `coworker.yaml` (or re-run `init --interactive` with repos).
11. Run a workflow once: `./unistar-coworker run-once --workflow daily-work`
12. Read `coworker.example.yaml` or [coworker.minimal.yaml](./coworker.minimal.yaml) for advanced settings.

---

## Path B — Docker

### Core — local agent

1. Install [Docker](https://docs.docker.com/get-docker/).
2. Pull the image: `docker pull ghcr.io/unistar-ai/unistar-coworker:latest`
3. Create dirs: `mkdir -p config data`
4. Create config (interactive):  
   `docker run --rm -it -v "$(pwd)/config:/config" -v "$(pwd)/data:/data" ghcr.io/unistar-ai/unistar-coworker:latest init --interactive --path /config/coworker.yaml`
5. Set `storage.path: /data` and `web.bind: 0.0.0.0:8787` in `config/coworker.yaml` (see [docs/docker.md](docs/docker.md)).
6. Point `llm.base_url` at a reachable API (host Ollama: `http://host.docker.internal:11434/v1` on Docker Desktop).
7. Run checks:  
   `docker run --rm -v "$(pwd)/config:/config" -v "$(pwd)/data:/data" ghcr.io/unistar-ai/unistar-coworker:latest doctor --config /config/coworker.yaml`
8. Start server:  
   `docker run --rm -p 127.0.0.1:8787:8787 -v "$(pwd)/config:/config" -v "$(pwd)/data:/data" ghcr.io/unistar-ai/unistar-coworker:latest serve --config /config/coworker.yaml`
9. Open [http://127.0.0.1:8787](http://127.0.0.1:8787).

### Optional — GitHub + secrets

10. Export `GH_TOKEN` (and remote `api_key` env vars if used) when running `doctor` / `serve`.
11. Mount host `~/.config/gh` read-only if using `gh auth login` — see [docs/docker.md](docs/docker.md).

> Docker images omit Chromium (`web-browser` feature). Browser automation tools are not available in the container.
