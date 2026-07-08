# Quick start

Get unistar-coworker running in a few minutes. Two install paths: **tar.gz** (native binary) or **Docker**.

Full docs: [README.md](README.md) · Docker details: [docs/docker.md](docs/docker.md)

---

## Path A — tar.gz (Linux x86_64 or macOS arm64)

1. Download the latest `unistar-coworker-*-*.tar.gz` from [GitHub Releases](https://github.com/unistar-ai/unistar-coworker/releases).
2. Extract: `tar -xzf unistar-coworker-*.tar.gz && cd unistar-coworker-*`
3. Make the binary executable: `chmod +x unistar-coworker`
4. Create config: `./unistar-coworker init --interactive`  
   (Non-interactive: `./unistar-coworker init --repos owner/repo --llm-url http://127.0.0.1:11434/v1`)
5. Set GitHub auth: `export GH_TOKEN=...` or run `gh auth login` on the host.
6. Check health: `./unistar-coworker doctor`
7. Start Web UI: `./unistar-coworker serve`
8. Open [http://127.0.0.1:8787](http://127.0.0.1:8787) in your browser.
9. (Optional) Run a workflow once: `./unistar-coworker run-once --workflow daily-work`
10. (Optional) Read `template/coworker.yaml` and `coworker.example.yaml` for advanced settings.

---

## Path B — Docker

1. Install [Docker](https://docs.docker.com/get-docker/).
2. Pull the image: `docker pull ghcr.io/unistar-ai/unistar-coworker:latest`
3. Create dirs: `mkdir -p config data`
4. Create config (interactive):  
   `docker run --rm -it -v "$(pwd)/config:/config" -v "$(pwd)/data:/data" ghcr.io/unistar-ai/unistar-coworker:latest init --interactive --path /config/coworker.yaml`
5. Set `storage.path: /data` and `web.bind: 0.0.0.0:8787` in `config/coworker.yaml` (see [docs/docker.md](docs/docker.md)).
6. Export secrets: `export DEEPSEEK_API_KEY=...` and/or `export GH_TOKEN=...`
7. Run checks:  
   `docker run --rm -v "$(pwd)/config:/config" -v "$(pwd)/data:/data" -e DEEPSEEK_API_KEY -e GH_TOKEN ghcr.io/unistar-ai/unistar-coworker:latest doctor --config /config/coworker.yaml`
8. Start server:  
   `docker run --rm -p 127.0.0.1:8787:8787 -v "$(pwd)/config:/config" -v "$(pwd)/data:/data" -e DEEPSEEK_API_KEY -e GH_TOKEN ghcr.io/unistar-ai/unistar-coworker:latest serve --config /config/coworker.yaml`
9. Open [http://127.0.0.1:8787](http://127.0.0.1:8787).
10. For `gh auth login` credentials from the host, mount `~/.config/gh` (read-only) — see [docs/docker.md](docs/docker.md).

> Docker images omit Chromium (`web-browser` feature). Browser automation tools are not available in the container.
