# codeagentd

面向 **Makefile C/C++ 项目** 的代码解析 Agent 后端：单项目单例 MCP + 多用户 Mutex 排队 + DeepSeek Tool Calling。

## 文档

| 文件 | 说明 |
|------|------|
| [PLAN.md](./PLAN.md) | **项目方案与分阶段实施计划**（主文档） |
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

### G122 示例

```toml
[project]
source_root = "g122app"
compile_commands = "g122_compile_commands.json"
remote_build_prefix = "/var/lib/home/beaver/G122/USB_Dongle"
```

`g122app` 对应远程 `/var/lib/home/beaver/G122/USB_Dongle`；`g122_compile_commands.json` 来自 build 服务器。

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
