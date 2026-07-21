# Packaging

[`scripts/package.sh`](../scripts/package.sh) builds web-ui + Rust binary and assembles one deploy tree (local workdir or GitHub Release).

| Path | Role |
|------|------|
| [`scripts/package.sh`](../scripts/package.sh) | Full package: web-ui, binary, skills, template, docs |
| [`QUICKSTART.md`](../QUICKSTART.md) / [`QUICKSTART_CN.md`](../QUICKSTART_CN.md) | First-run guide (tar.gz + Docker); copied into `.coworker/` |
| `workdir-template/` | Seed config copied into `.coworker/` (`template/` + `coworker.yaml`) |

### Output layout

Claude-style project agent directory — binary at the deploy root, everything else under `.coworker/`:

```
<output>/
├── unistar-coworker           # binary (embed-web-ui, release)
└── .coworker/                 # agent home (config / skills / data / docs)
    ├── skills/
    ├── template/              # pristine workdir-template/
    ├── coworker.yaml          # active config (from template or preserved)
    ├── AGENTS.md
    ├── coworker.example.yaml
    ├── coworker.minimal.yaml
    ├── README.md
    ├── QUICKSTART.md
    ├── QUICKSTART_CN.md
    └── data/                  # preserved across rebuilds (local runtime only)
```

Process **cwd** stays at `<output>/` so `chat.workspace: .` is the project root. Config discovery prefers `.coworker/coworker.yaml` (legacy flat `coworker.yaml` still works).

**Local** (default `../workdir` next to repo):

```bash
./scripts/package.sh
# or: START_AGENT_WORKDIR=./workdir ./scripts/package.sh
```

**GitHub Release** (also writes `dist/*.tar.gz` + `.sha256`):

```bash
PACKAGE_VERSION=2.0.0 PACKAGE_TRIPLE=x86_64-unknown-linux-gnu ./scripts/package.sh
```

Override paths with `START_AGENT_WORKDIR` and `START_AGENT_DATA_BACKUP`.

Launch after packaging: `cd <output> && ./unistar-coworker serve`, or use your deploy wrapper (e.g. parent `start-agent.sh`).

Tag push (`v*`) runs [`.github/workflows/release.yml`](../.github/workflows/release.yml), which calls `package.sh` with `PACKAGE_VERSION` / `PACKAGE_TRIPLE`.

Docker images are published separately via [`.github/workflows/docker.yml`](../.github/workflows/docker.yml) to `ghcr.io/unistar-ai/unistar-coworker` — see [docs/docker.md](../docs/docker.md).
