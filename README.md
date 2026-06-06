# codeagentd

面向 **Makefile C/C++ 项目** 的代码解析 Agent 后端：单项目单例 MCP + 多用户 Mutex 排队 + DeepSeek Tool Calling。

## 文档

| 文件 | 说明 |
|------|------|
| [context-about-codeagentd.md](./context-about-codeagentd.md) | 设计讨论背景资料（2026-06） |

## 快速依赖

```bash
curl -LsSf https://astral.sh/uv/install.sh | sh
cargo install mcp-cpp-server --git https://github.com/mpsm/mcp-cpp
clangd --version
```

## 配置

项目设置通过 **`.codeagentd/settings.toml`** 指定（见 [settings.toml.example](./.codeagentd/settings.toml.example)）：

| 字段 | 必填 | 说明 |
|------|------|------|
| `project.source_root` | ✅ | **只读**源码目录，必须存在 |
| `project.compile_commands` | ✅ | compile_commands.json，必须存在且为非空 JSON 数组 |
| `listen_addr` | | 默认 `0.0.0.0:3000` |
| `work_dir` | | 默认 `.codeagentd/tmp` |
| `project.remote_build_prefix` | | 远程路径前缀；仅在 `work_dir` 副本中重写 |
| `llm.base_url` | | 默认 `https://api.deepseek.com` |
| `llm.model` | | 默认 `deepseek-v4-pro` |

可选字段若填写则校验合法性；留空或省略则用默认值。`DEEPSEEK_API_KEY` 放在环境变量（或 `[llm].api_key`）。

```bash
mkdir -p .codeagentd
cp .codeagentd/settings.toml.example .codeagentd/settings.toml   # 按项目修改
export DEEPSEEK_API_KEY=sk-...

# 守护进程（推荐）
cargo build --release
./target/release/codeagentd start    # 后台启动
./target/release/codeagentd stop     # SIGTERM → 清理 tmp/logs → 超时后杀整组进程
./target/release/codeagentd restart  # stop + start（MCP 一并重启）

# 开发时也可用 cargo run --
cargo run -- start

# 前台调试
cargo run -- run
```

运行态文件（均在 `.codeagentd/` 下，不污染项目根目录）：

| 文件 | 说明 |
|------|------|
| `settings.toml` | 服务配置 |
| `run/codeagentd.pid` | 进程 PID |
| `logs/codeagentd.log` | 标准输出/错误日志 |
| `tmp/` | 默认 compile_commands 临时副本（可在 settings 中改 `work_dir`） |

### 配置示例

本地源码与 build 服务器路径不一致时，用 `remote_build_prefix` 在 `work_dir` 副本中重写 `compile_commands.json` 里的路径前缀：

```toml
[project]
source_root = "my-project"
compile_commands = "compile_commands.json"
remote_build_prefix = "/build/ci/my-project"
```

含义：`compile_commands.json` 中若写的是 `/build/ci/my-project/...`，运行时会映射到本机 `source_root`（即 `my-project/...`）。`compile_commands.json` 通常由 build 服务器或本地 `bear -- make` / `compiledb` 生成。

## 运行

```bash
cargo run -- start
open http://127.0.0.1:3000/
```

也可用 curl：

```bash
curl http://127.0.0.1:3000/health
curl -N -X POST http://127.0.0.1:3000/api/chat \
  -H 'Content-Type: application/json' \
  -d '{"user_id":"alice","question":"项目有哪些主要类？"}'
```
