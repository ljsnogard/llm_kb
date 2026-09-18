# kb_client_config

`kb_admin_desktop` **自己**的连接配置：启动时读它，决定"怎么找到 / 启动 `kb_core`"。

`kb_core` 不知道也不需要知道这份文件——它是客户端的本地选择。

## 生命周期

```text
启动
  ├─ ClientConfig::load(path)
  │     ├─ Ok(config)         → 用 config.default_connection() 去连
  │     ├─ Err(NotFound)      → 首次运行：问用户"怎么连"
  │     │                       → 组装 ClientConfig → save(path)
  │     └─ Err(Parse/…)       → 让用户修，**不要**静默覆盖
  └─ 连接（由 kb_client_conn_mgr 负责）
```

"文件不存在"与"文件坏了"是**两件事**：前者是 `ConfigError::NotFound`（界面进
首次运行），后者是 `ConfigError::Parse`（界面提示用户去改）。

## 格式：TOML

```toml
version = 1
default = "本机"

[[connections]]
name = "本机"
kind = "local-launch"
kb_core = "/usr/local/bin/kb-core"
runtime_dir = "/run/user/1000/llm_kb"
storage_dir = "/run/user/1000/llm_kb/storage"
handshake_timeout_millis = 10000   # 可省

[[connections]]
name = "已在本机跑着的"
kind = "local-attach"
runtime_dir = "/run/user/1000/llm_kb"

[[connections]]
name = "实验室"
kind = "tcp"
address = "192.168.1.5:8788"
connect_timeout_millis = 5000      # 可省
request_timeout_millis = 15000     # 可省
```

三种 `kind` 的语义见 `kb_client_conn_mgr` 的 README；这里只负责"怎么存"。
省掉的时限用本 crate 的缺省常量（`DEFAULT_*_TIMEOUT`）。

## 路径

| 平台 | 位置 |
| :--- | :--- |
| Linux 等 | `$XDG_CONFIG_HOME/kb_admin_desktop/config.toml`（缺省 `~/.config/…`） |
| macOS | `~/Library/Application Support/kb_admin_desktop/config.toml` |
| Windows | `%APPDATA%\kb_admin_desktop\config.toml` |

显式覆盖优先：环境变量 `KB_ADMIN_DESKTOP_CONFIG`（开发、排错、或多份配置并存时用）。

## 写是原子的

`ClientConfig::save` 先写同目录下的 `.tmp` 再 `rename`，所以读到的要么是旧内容、
要么是新内容，不会是被写了一半的 TOML——与 `kb_core` 存储层的约定一致。

## 校验

`load` / `save` 都会先 `validate()`：版本必须是 `CONFIG_VERSION`、至少一条连接、
名字非空且不重复、`default` 必须指向存在的项、必填字段非空。
手改配置写错 `kind` 会在解析阶段就报出来，不会留到连接时才炸。

## 验证

```bash
cargo test -p kb_client_config
```

9 个单元测试 + 1 个文档测试：TOML 往返、显式时限保留、六种坏配置被拒、
"文件不存在"与"文件坏了"分开、原子写往返、缺省项选择、`kind` 字符串对称。

## 相关文档

- [`kb_client_conn_mgr`](../kb_client_conn_mgr/README.md)：拿这里的一条连接方式去连 `kb_core`；
- 根 `README.md` §2：各 crate 的分工。
