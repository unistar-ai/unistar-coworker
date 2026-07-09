# 快速开始

几分钟内跑起 unistar-coworker。两种安装方式：**tar.gz**（原生二进制）或 **Docker**。

**参考档：** 本机 **25B+** 模型（如 `gemma4:26b-a4b`、`qwen3.6:27b`）。GitHub 为可选能力。

完整文档：[README_CN.md](README_CN.md) · Docker 说明：[docs/docker.md](docs/docker.md)

---

## 路径 A — tar.gz（Linux x86_64 或 macOS arm64）

### 核心 — 本地 Agent（无需 GitHub）

1. 从 [GitHub Releases](https://github.com/unistar-ai/unistar-coworker/releases) 下载最新的 `unistar-coworker-*-*.tar.gz`。
2. 解压：`tar -xzf unistar-coworker-*.tar.gz && cd unistar-coworker-*`
3. 赋予执行权限：`chmod +x unistar-coworker`
4. 启动 Ollama 并拉取 **25B+** 模型，例如 `ollama pull gemma4:26b-a4b` 或 `ollama pull qwen3.6:27b`（profile 示例见 [coworker.example.yaml](./coworker.example.yaml)）。
5. 创建配置：`./unistar-coworker init --interactive`  
   （非交互：`./unistar-coworker init --llm-url http://127.0.0.1:11434/v1`。）
6. 健康检查：`./unistar-coworker doctor`
7. 启动 Web UI：`./unistar-coworker serve`
8. 浏览器打开 [http://127.0.0.1:8787](http://127.0.0.1:8787)，在工作区内对话（配置项 `chat.workspace`）。

### 可选 — GitHub

9. 配置 GitHub 认证：`export GH_TOKEN=...` 或在宿主机执行 `gh auth login`。
10. 在对话里写明 `owner/repo` 或粘贴 PR URL — agent **不会**猜测默认仓库。
11. 试用：`./unistar-coworker chat --once "汇总 owner/repo 的 open PR"`  
    或：`./unistar-coworker chat --once "triage https://github.com/owner/repo/pull/42"`  
    CLI 报告：`./unistar-coworker report ci --repo owner/repo`
12. 阅读 `coworker.example.yaml` 或 [coworker.minimal.yaml](./coworker.minimal.yaml) 了解高级选项。

---

## 路径 B — Docker

### 核心 — 本地 Agent

1. 安装 [Docker](https://docs.docker.com/get-docker/)。
2. 拉取镜像：`docker pull ghcr.io/unistar-ai/unistar-coworker:latest`
3. 创建目录：`mkdir -p config data`
4. 交互式创建配置：  
   `docker run --rm -it -v "$(pwd)/config:/config" -v "$(pwd)/data:/data" ghcr.io/unistar-ai/unistar-coworker:latest init --interactive --path /config/coworker.yaml`
5. 在 `config/coworker.yaml` 中设置 `storage.path: /data` 与 `web.bind: 0.0.0.0:8787`（见 [docs/docker.md](docs/docker.md)）。
6. 将 `llm.base_url` 指向可访问的 API（宿主机 Ollama：Docker Desktop 可用 `http://host.docker.internal:11434/v1`）。
7. 运行检查：  
   `docker run --rm -v "$(pwd)/config:/config" -v "$(pwd)/data:/data" ghcr.io/unistar-ai/unistar-coworker:latest doctor --config /config/coworker.yaml`
8. 启动服务：  
   `docker run --rm -p 127.0.0.1:8787:8787 -v "$(pwd)/config:/config" -v "$(pwd)/data:/data" ghcr.io/unistar-ai/unistar-coworker:latest serve --config /config/coworker.yaml`
9. 浏览器打开 [http://127.0.0.1:8787](http://127.0.0.1:8787)。

### 可选 — GitHub 与密钥

10. 运行 `doctor` / `serve` 时导出 `GH_TOKEN`（及远程 `api_key` 对应环境变量）。
11. 若使用宿主机 `gh auth login`，只读挂载 `~/.config/gh` — 见 [docs/docker.md](docs/docker.md)。

> Docker 镜像未包含 Chromium（未启用 `web-browser` 特性），容器内无法使用浏览器自动化工具。
