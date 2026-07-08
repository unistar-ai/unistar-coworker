# 快速开始

几分钟内跑起 unistar-coworker。两种安装方式：**tar.gz**（原生二进制）或 **Docker**。

完整文档：[readme_cn.md](readme_cn.md) · Docker 说明：[docs/docker.md](docs/docker.md)

---

## 路径 A — tar.gz（Linux x86_64 或 macOS arm64）

1. 从 [GitHub Releases](https://github.com/unistar-ai/unistar-coworker/releases) 下载最新的 `unistar-coworker-*-*.tar.gz`。
2. 解压：`tar -xzf unistar-coworker-*.tar.gz && cd unistar-coworker-*`
3. 赋予执行权限：`chmod +x unistar-coworker`
4. 创建配置：`./unistar-coworker init --interactive`  
   （非交互：`./unistar-coworker init --repos owner/repo --llm-url http://127.0.0.1:11434/v1`）
5. 配置 GitHub 认证：`export GH_TOKEN=...` 或在宿主机执行 `gh auth login`。
6. 健康检查：`./unistar-coworker doctor`
7. 启动 Web UI：`./unistar-coworker serve`
8. 浏览器打开 [http://127.0.0.1:8787](http://127.0.0.1:8787)。
9. （可选）运行一次工作流：`./unistar-coworker run-once --workflow daily-work`
10. （可选）阅读 `template/coworker.yaml` 与 `coworker.example.yaml` 了解高级选项。

---

## 路径 B — Docker

1. 安装 [Docker](https://docs.docker.com/get-docker/)。
2. 拉取镜像：`docker pull ghcr.io/unistar-ai/unistar-coworker:latest`
3. 创建目录：`mkdir -p config data`
4. 交互式创建配置：  
   `docker run --rm -it -v "$(pwd)/config:/config" -v "$(pwd)/data:/data" ghcr.io/unistar-ai/unistar-coworker:latest init --interactive --path /config/coworker.yaml`
5. 在 `config/coworker.yaml` 中设置 `storage.path: /data` 与 `web.bind: 0.0.0.0:8787`（见 [docs/docker.md](docs/docker.md)）。
6. 导出密钥：`export DEEPSEEK_API_KEY=...` 和/或 `export GH_TOKEN=...`
7. 运行检查：  
   `docker run --rm -v "$(pwd)/config:/config" -v "$(pwd)/data:/data" -e DEEPSEEK_API_KEY -e GH_TOKEN ghcr.io/unistar-ai/unistar-coworker:latest doctor --config /config/coworker.yaml`
8. 启动服务：  
   `docker run --rm -p 127.0.0.1:8787:8787 -v "$(pwd)/config:/config" -v "$(pwd)/data:/data" -e DEEPSEEK_API_KEY -e GH_TOKEN ghcr.io/unistar-ai/unistar-coworker:latest serve --config /config/coworker.yaml`
9. 浏览器打开 [http://127.0.0.1:8787](http://127.0.0.1:8787)。
10. 若需使用宿主机 `gh auth login` 的凭据，只读挂载 `~/.config/gh` — 见 [docs/docker.md](docs/docker.md)。

> Docker 镜像未包含 Chromium（未启用 `web-browser` 特性），容器内无法使用浏览器自动化工具。
