# Reshape 配置指南

本文说明 `crates/reshape-core/src/config.rs` 中的配置结构应该如何使用，以及运行时应该在 TOML 配置文件中如何设置这些字段。

## 配置入口

`reshape-core` 的 `AppConfig` 是运行时配置的内存结构。它定义默认值、字段类型和运行时依赖装配所需的数据；实际用户配置由 `reshape-cli` 从 TOML 文件读取后合并进 `AppConfig`。

默认配置文件位置：

```text
~/.reshape/config.toml
```

也可以通过 CLI 显式指定：

```bash
cargo run -p reshape-cli -- --config /path/to/config.toml
```

如果没有传 `--config`，CLI 会自动创建 `~/.reshape/config.toml` 和默认 workspace。显式传入的配置文件必须存在，否则启动会失败。

## 推荐最小配置

本地正常运行至少需要配置 workspace 和 OpenAI API key：

```toml
workspace = "/absolute/path/to/page"
log_level = "info"

[llm]
provider = "openai"

[llm.openai]
model = "gpt-5.5"
base_url = "https://api.openai.com/v1"
api_key = "sk-..."
stream = true
timeout_secs = 120

[runtime]
max_tool_iterations = 8
max_tool_calls = 32

[server]
host = "127.0.0.1"
port = 7331
```

`workspace` 必须是已经存在的目录。运行 `reshape workspace init` 可以创建默认 workspace 页面文件。

## 合并优先级

配置按以下顺序合并，后者覆盖前者：

1. `crates/reshape-core/src/config.rs` 中的 `Default` 默认值。
2. TOML 配置文件字段。
3. CLI 参数。

当前 CLI 会覆盖的字段包括：

- `--workspace` 覆盖 `workspace`。
- `--model` 覆盖 `llm.openai.model`。
- `--host` 覆盖 `server.host`。
- `--port` 覆盖 `server.port`。
- `--log-level` 覆盖 `log_level`。

## 字段说明

### 顶层字段

| 字段 | 默认值 | 说明 |
| --- | --- | --- |
| `workspace` | `~/.reshape/workspace` | agent 允许读取和写入的工作目录。 |
| `log_level` | `"info"` | `tracing` 日志级别，例如 `debug`、`info`、`warn`。 |

`workspace` 最终会进入 `AppConfig.workspace.root`，并由 `validate_workspace()` 校验是否存在且是目录。

### LLM 配置

```toml
[llm]
provider = "openai"

[llm.openai]
model = "gpt-5.5"
base_url = "https://api.openai.com/v1"
api_key = "sk-..."
stream = true
timeout_secs = 120
organization = "org_..."
project = "proj_..."
```

| 字段 | 默认值 | 说明 |
| --- | --- | --- |
| `llm.provider` | `"openai"` | 当前只支持 `openai`。 |
| `llm.openai.model` | `"gpt-5.5"` | Chat Completions 模型名。 |
| `llm.openai.base_url` | `"https://api.openai.com/v1"` | OpenAI 兼容 API base URL。 |
| `llm.openai.api_key` | `""` | 必填；构建真实运行时时不能为空。 |
| `llm.openai.stream` | `true` | 是否使用流式响应聚合。 |
| `llm.openai.timeout_secs` | `120` | LLM 请求超时时间。 |
| `llm.openai.organization` | 未设置 | 可选 OpenAI organization header。 |
| `llm.openai.project` | 未设置 | 可选 OpenAI project header。 |

不要把真实 API key 提交到仓库。个人开发时放在 `~/.reshape/config.toml`；共享示例时保留空字符串或占位符。

### Runtime 配置

```toml
[runtime]
max_tool_iterations = 8
max_tool_calls = 32
```

| 字段 | 默认值 | 说明 |
| --- | --- | --- |
| `max_tool_iterations` | `8` | LLM -> tool -> LLM 循环的最大轮数。 |
| `max_tool_calls` | `32` | 单次 agent turn 可执行的最大工具调用数。 |

这两个限制是防止无限工具循环的安全边界。常规页面生成建议保留默认值；只有在明确需要更长工具链时再调高。

### Server 配置

```toml
[server]
host = "127.0.0.1"
port = 7331
```

| 字段 | 默认值 | 说明 |
| --- | --- | --- |
| `host` | `"127.0.0.1"` | JSON-RPC render server 绑定地址。必须是 loopback 地址。 |
| `port` | `7331` | 本地服务端口。 |

`host` 只允许 loopback 地址，例如 `127.0.0.1`、`127.0.0.2` 或 `::1`。不要配置成 `0.0.0.0`。

## 网络工具配置

网络工具默认关闭。启用后，运行时会在工具注册表中额外注册 `web_search` 或 `web_fetch`。

### Tavily Web Search

```toml
[tools.web_search]
enabled = true
provider = "tavily"

[tools.web_search.tavily]
api_key = ""
env_key = "TAVILY_API_KEY"
base_url = "https://api.tavily.com"
search_depth = "basic"
max_results = 5
topic = "general"
include_answer = false
include_images = false
include_favicon = true
timeout_secs = 15
```

| 字段 | 默认值 | 说明 |
| --- | --- | --- |
| `enabled` | `false` | 是否注册 `web_search` 工具。 |
| `provider` | `"tavily"` | 当前只支持 Tavily。 |
| `api_key` | `""` | Tavily API key；优先级高于环境变量。 |
| `env_key` | `"TAVILY_API_KEY"` | 当 `api_key` 为空时读取的环境变量名。 |
| `base_url` | `"https://api.tavily.com"` | Tavily API base URL。 |
| `search_depth` | `"basic"` | 默认搜索深度，可用 `basic` 或 `advanced`。 |
| `max_results` | `5` | 默认返回结果数，工具执行时会限制到最多 20。 |
| `topic` | `"general"` | 默认主题，例如 `general` 或 `news`。 |
| `include_answer` | `false` | 是否让 Tavily 返回 answer 摘要。 |
| `include_images` | `false` | 是否包含图片结果。 |
| `include_favicon` | `true` | 是否包含 favicon 元数据。 |
| `timeout_secs` | `15` | 搜索请求超时。 |

推荐把 Tavily key 放到环境变量：

```bash
export TAVILY_API_KEY="tvly-..."
```

然后保持 `api_key = ""`。

### Web Fetch

```toml
[tools.web_fetch]
enabled = true
max_bytes = 52428800
timeout_secs = 60
max_redirects = 5
download_dir = "assets/downloads"
allowed_content_types = [
  "image/png",
  "image/jpeg",
  "image/gif",
  "image/webp",
  "video/mp4",
  "video/webm",
  "audio/mpeg",
  "audio/wav",
  "application/pdf",
]
ssrf_allowlist = []
```

| 字段 | 默认值 | 说明 |
| --- | --- | --- |
| `enabled` | `false` | 是否注册 `web_fetch` 工具。 |
| `max_bytes` | `52428800` | 单次下载最大字节数，默认 50 MiB。 |
| `timeout_secs` | `60` | 下载请求超时。 |
| `max_redirects` | `5` | 手动跟随重定向的最大次数。 |
| `download_dir` | `"assets/downloads"` | 默认下载目录，必须是 workspace 相对路径。 |
| `allowed_content_types` | 见默认配置 | 允许保存的 MIME 类型。 |
| `ssrf_allowlist` | `[]` | 允许访问的私有 IP 或 CIDR。默认阻止 loopback、private、link-local 等地址。 |

`web_fetch` 只下载 HTTP/HTTPS 媒体或二进制资源，不用于抽取网页正文。保存文件仍然通过 workspace 安全边界，目标路径不能逃出 workspace。

## 完整示例

```toml
workspace = "/Users/me/reshape-page"
log_level = "info"

[llm]
provider = "openai"

[llm.openai]
model = "gpt-5.5"
base_url = "https://api.openai.com/v1"
api_key = "sk-..."
stream = true
timeout_secs = 120

[runtime]
max_tool_iterations = 8
max_tool_calls = 32

[server]
host = "127.0.0.1"
port = 7331

[tools.web_search]
enabled = true
provider = "tavily"

[tools.web_search.tavily]
api_key = ""
env_key = "TAVILY_API_KEY"
base_url = "https://api.tavily.com"
search_depth = "basic"
max_results = 5
topic = "general"
include_answer = false
include_images = false
include_favicon = true
timeout_secs = 15

[tools.web_fetch]
enabled = true
max_bytes = 52428800
timeout_secs = 60
max_redirects = 5
download_dir = "assets/downloads"
allowed_content_types = [
  "image/png",
  "image/jpeg",
  "image/gif",
  "image/webp",
  "video/mp4",
  "video/webm",
  "audio/mpeg",
  "audio/wav",
  "application/pdf",
]
ssrf_allowlist = []
```

## 修改 `config.rs` 的原则

只有当配置契约本身发生变化时才修改 `crates/reshape-core/src/config.rs`，例如新增 provider、增加 runtime limit、或给工具增加新的配置字段。普通环境差异应写入 TOML 配置文件，不应通过改源码实现。

修改 `config.rs` 时需要同步：

- 为新字段添加 `Default`。
- 在 `reshape-cli` 的文件配置结构中增加对应 `Option<T>` 字段。
- 在配置合并逻辑中只覆盖 TOML 明确设置的字段。
- 为配置解析、默认值、CLI 覆盖和运行时装配补充集成测试。
- 更新本文档和 `README.md` 中相关配置说明。
