# Docker

Run unistar-coworker in a container without installing Rust or Node locally.

**Image:** `ghcr.io/unistar-ai/unistar-coworker`

> **Note:** Docker images are built with `--no-default-features` (no bundled Chromium).
> The `web-browser` chat tool is unavailable in the container. Use `web_fetch` or host a browser outside the container.

## Pull

```bash
docker pull ghcr.io/unistar-ai/unistar-coworker:latest
# Or pin a release: docker pull ghcr.io/unistar-ai/unistar-coworker:2.2.0
```

## Quick run

Map the Web UI to **localhost only** (recommended):

```bash
docker run --rm -it \
  -p 127.0.0.1:8787:8787 \
  -v "$(pwd)/config:/config" \
  -v "$(pwd)/data:/data" \
  -e DEEPSEEK_API_KEY \
  ghcr.io/unistar-ai/unistar-coworker:latest \
  serve --config /config/coworker.yaml
```

Open [http://127.0.0.1:8787](http://127.0.0.1:8787).

## Volumes

| Mount | Purpose |
|-------|---------|
| `/config/coworker.yaml` | Config file (pass `--config /config/coworker.yaml`) |
| `/data` | Store path (`storage.path` in config) — chat sessions, audit, digests |
| `/workspace` | Optional `chat.workspace` bind (read-only or read-write) |

### Example `coworker.yaml` for Docker

Save as `./config/coworker.yaml` before first run:

```yaml
llm_profile: deepseek

llm:
  deepseek:
    base_url: https://api.deepseek.com/v1
    model: deepseek-chat
    context_limit: 128000
    api_key: ${DEEPSEEK_API_KEY}

github:
  gh_command: gh
  env: {}

repos:
  - owner/repo

storage:
  path: /data

web:
  bind: 0.0.0.0:8787   # listen inside the container; map -p 127.0.0.1:8787:8787 on the host
```

Create config and data dirs:

```bash
mkdir -p config data
# edit config/coworker.yaml (see template above or copy from template/coworker.yaml in the image)
```

## Environment variables

| Variable | Purpose |
|----------|---------|
| `DEEPSEEK_API_KEY` | Expands `${DEEPSEEK_API_KEY}` in `coworker.yaml` |
| `GH_TOKEN` | GitHub API token when not using mounted `gh` config |

Pass secrets with `-e` or an env file (`--env-file .env`). Never commit keys into the image or config repo.

## GitHub CLI (`gh`) authentication

The runtime image includes `git` but **not** `gh`. Choose one:

1. **`GH_TOKEN`** — simplest for containers:

   ```bash
   export GH_TOKEN=ghp_...   # or github_pat_...
   docker run ... -e GH_TOKEN ghcr.io/unistar-ai/unistar-coworker:latest serve --config /config/coworker.yaml
   ```

2. **Mount host `gh` config** (after `gh auth login` on the host):

   ```bash
   docker run ... \
     -v "$HOME/.config/gh:/root/.config/gh:ro" \
     ghcr.io/unistar-ai/unistar-coworker:latest serve --config /config/coworker.yaml
   ```

   Install `gh` in a custom image if you need the binary inside the container.

## First-time setup inside the container

```bash
docker run --rm -it \
  -v "$(pwd)/config:/config" \
  -v "$(pwd)/data:/data" \
  -e DEEPSEEK_API_KEY \
  ghcr.io/unistar-ai/unistar-coworker:latest \
  init --interactive --path /config/coworker.yaml

docker run --rm \
  -v "$(pwd)/config:/config" \
  -v "$(pwd)/data:/data" \
  -e DEEPSEEK_API_KEY \
  ghcr.io/unistar-ai/unistar-coworker:latest \
  doctor --config /config/coworker.yaml
```

## Build locally

```bash
docker build -t unistar-coworker:local .
docker run --rm -p 127.0.0.1:8787:8787 unistar-coworker:local serve
```

See also [QUICKSTART.md](../QUICKSTART.md) and [packaging/README.md](../packaging/README.md).
