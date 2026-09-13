# kb_core

知识库主进程的可执行入口。

依据 `dev-notes.md` §13.1 的分工：`kb_svc_salvo` 是**纯库**（提供 HTTP / WebSocket
路由、会话逻辑与监听器组装），**进程的启动与编排由本 crate 负责**。现阶段两者视为
一体，将来再拆分。

> **当前状态：阶段 3.5**
> 已有：仿 DSH 观感的聊天界面、浏览器 ↔ 插件两段 WebSocket 通道、
> LLM 服务与 API key 的配置（文件 + 界面）。
> 还没有：`kb_rig_llm` 插件进程（因此现在提问会提示「插件未连接」）。

---

## 1. 它做了什么

启动后**同时**监听两个服务，二者共用同一份路由表：

| 监听 | 位置 | 给谁用 |
| :--- | :--- | :--- |
| TCP HTTP / WebSocket | `127.0.0.1:8788`（默认） | 浏览器：界面、设置接口、`/ws/chat` |
| Unix domain socket | `$XDG_RUNTIME_DIR/llm_kb/kb-<日期>-<uuid>.sock` | `kb_rig_llm` 插件子进程：`/ws/plugin` |

## 2. 依赖的外部变量与约定

### 2.1 环境变量

| 变量 | 是否必需 | 作用 |
| :--- | :--- | :--- |
| `XDG_CONFIG_HOME` / `HOME` | 否 | 决定默认配置文件位置 `$XDG_CONFIG_HOME/llm_kb/config.toml`（回退 `~/.config/...`）。 |
| `XDG_RUNTIME_DIR` | 否 | 决定插件 socket 的父目录 `$XDG_RUNTIME_DIR/llm_kb`；未设置时回退系统临时目录。 |
| `RUST_LOG` | 否 | 日志过滤，走 `env_logger`。默认 `info`；排查问题可设 `RUST_LOG=debug`（会打印 WebSocket 收发与资源覆盖情况）。 |
| `LLM_KB_PLUGIN_SOCKET` | 否（**尚未使用**） | 预留：启动 `kb_rig_llm` 子进程时，父进程用它把 socket 路径传给子进程。 |

### 2.2 配置文件

默认位置 `$XDG_CONFIG_HOME/llm_kb/config.toml`（可用 `--config` 覆盖）。
**不存在时会自动生成**一份带注释的模板，因此首次启动不需要手工创建：

```toml
[services.deepseek]
provider = "deepseek"
model = "deepseek-chat"
base_url = "https://api.deepseek.com"
api_key = ""
```

- `services.<id>` 的 `<id>` 由用户自取，会作为界面上「服务」下拉框里的名字；
- 通过界面修改时**逐键写回**，用户写在文件里的注释会被保留；
- ⚠️ **API key 是明文保存的**（当前阶段的安全取舍），不要把该文件提交到版本库。

### 2.3 socket 文件名

socket **文件名不接受命令行参数或环境变量指定**，每次启动按「日期 + UUID」生成：

```text
$XDG_RUNTIME_DIR/llm_kb/kb-20260913-f9c628f955594c1fa73965bcac42f091.sock
                    └─ 日期 ─┘└──────── UUID v4 ────────┘
```

目录权限 `0700`、文件权限 `0600`；带 UUID 保证每次启动互不冲突，历史残留文件不会
导致启动失败。进程被 `SIGKILL` 强杀时文件会残留（无法拦截），可手工清理。

### 2.4 命令行参数

```text
kb_core [--config <file>] [--runtime-dir <dir>] [--assets-dir <dir>] [tcp_addr]
```

| 参数 | 默认值 | 说明 |
| :--- | :--- | :--- |
| `tcp_addr` | `127.0.0.1:8788` | 用户侧监听地址；端口写 `0` 表示由系统分配。 |
| `--config <file>` | `$XDG_CONFIG_HOME/llm_kb/config.toml` | 用户配置文件；不存在时自动创建。 |
| `--runtime-dir <dir>` | `$XDG_RUNTIME_DIR/llm_kb` | 只覆盖 socket 的**目录**。 |
| `--assets-dir <dir>` | 无（用内嵌资源） | 前端资源覆盖目录，调试页面时用；只在启动时读取一次。 |

参数顺序不敏感；未知参数会被忽略并打印警告。

## 3. 从最简单的用例开始

### 3.1 启动

```bash
cargo run -p kb_core
```

预期日志：

```text
[<时间戳> INFO  kb_core] 用户配置: /root/.config/llm_kb/config.toml
[<时间戳> INFO  kb_core] 服务 deepseek: provider=deepseek model=deepseek-chat api_key=缺失
[<时间戳> INFO  kb_svc_salvo::server] kb_svc_salvo 已绑定: tcp=127.0.0.1:8788 uds=/run/user/1000/llm_kb/kb-<日期>-<uuid>.sock
[<时间戳> INFO  kb_core] 用户界面: http://127.0.0.1:8788
[<时间戳> INFO  kb_core] 插件通道: /run/user/1000/llm_kb/kb-<日期>-<uuid>.sock
```

首次运行会创建配置文件，因此第二行的服务名与 `api_key=缺失` 是预期的。

### 3.2 用浏览器访问

打开 <http://127.0.0.1:8788/>，会看到仿 DSH 观感的聊天界面：

- 顶部：产品名、插件在线状态、服务下拉框、主题切换、⚙ 设置；
- 中部：空态提示（还没有对话时）；
- 底部：输入框 + 发送按钮（Enter 发送、Shift+Enter 换行）。

首次打开时插件必是**离线**状态，这是正常的——`kb_rig_llm` 还没实现。
点右上角 **⚙** 就能配置 LLM 服务与 API key。

若想固定路径、避免污染用户目录：

```bash
cargo run -p kb_core -- --config /tmp/kb-demo/config.toml --runtime-dir /tmp/kb-demo/run
```

## 4. 一句命令能测出什么

下列命令假定服务端已在 `127.0.0.1:8788` 运行。

### 4.1 界面与静态资源

```bash
curl -sI http://127.0.0.1:8788/ | head -3
curl -s -o /dev/null -w "app.css: %{http_code} %{content_type}\n" http://127.0.0.1:8788/app.css
curl -s -o /dev/null -w "app.js:  %{http_code} %{content_type}\n" http://127.0.0.1:8788/app.js
```

预期：

```text
HTTP/1.1 200 OK
content-type: text/html; charset=utf-8
...
app.css: 200 text/css; charset=utf-8
app.js:  200 text/javascript; charset=utf-8
```

### 4.2 配置文件被自动创建

```bash
ls -la "${XDG_CONFIG_HOME:-$HOME/.config}/llm_kb/"
```

预期：存在 `config.toml`，内容是一份带注释的模板（含 `[services.deepseek]`）。
用 `--config /tmp/kb-demo/config.toml` 启动时到该路径下找。

### 4.3 设置接口：写入服务并遮蔽 API key

```bash
# 写入一个服务
curl -s -X POST http://127.0.0.1:8788/api/settings/services \
  -H 'content-type: application/json' \
  -d '{"id":"deepseek","provider":"deepseek","model":"deepseek-chat","base_url":"https://api.deepseek.com","api_key":"sk-demo-123"}'

# 再读回来
curl -s http://127.0.0.1:8788/api/settings
```

预期：第一条返回 `{"id":"deepseek","ok":true}`；第二条里该服务的
`api_key` 是 `••••••••`、`has_api_key` 为 `true`，且 `active_service` 已被自动设为
`deepseek`。**响应里不会出现明文 key**，也不会出现 `sk-demo-123`。

同时配置文件被写回，且**原有注释仍在**：

```bash
grep -c '^#' "${XDG_CONFIG_HOME:-$HOME/.config}/llm_kb/config.toml"   # 预期 > 0
```

### 4.4 没有插件时提问会被明确拒绝

浏览器里直接输入问题并发送，界面底部会提示「LLM 插件当前未连接」，
消息区出现一条红色错误块。等价的命令行验证由集成测试覆盖（见 §4.6）。

### 4.5 优雅退出会清理 socket 文件

```bash
rm -rf /tmp/kb-run
timeout -s TERM 3 cargo run -p kb_core -- --runtime-dir /tmp/kb-run 127.0.0.1:8788
ls -A /tmp/kb-run/    # 预期：没有任何输出
```

预期日志：

```text
[<时间戳> INFO  kb_core] 收到 SIGTERM，开始优雅退出
[<时间戳> WARN  kb_core] 服务端未在 3s 内退出，已放弃等待
[<时间戳> INFO  kb_core] 已清理 socket 文件: /tmp/kb-run/kb-<日期>-<uuid>.sock
```

交互式终端里按 `Ctrl-C`（`SIGINT`）走同一条路径。

### 4.6 自动化验证（推荐）

```bash
cargo test -p kb_svc_salvo -p kb_core
```

预期共 39 项全部 `ok`，其中与界面/配置直接相关的是：

```text
test browser_question_reaches_plugin_and_answers_stream_back ... ok   # 提问→插件→增量回浏览器
test chat_channel_greets_with_ready ... ok                           # 打开页面即收到 ready
test ask_without_plugin_returns_error_frame ... ok                   # 无插件时明确报错
test settings_api_round_trip_masks_key ... ok                        # 写入服务 + 遮蔽明文 key
test index_serves_embedded_html ... ok                               # 首页 HTML
test file_store_upsert_keeps_comments ... ok                         # 写配置时保留注释
```

## 5. 预期结果一览

| 命令 / 操作 | 验证的内容 | 预期结果 |
| :--- | :--- | :--- |
| `cargo run -p kb_core` | 启动、建配置、绑两个监听器 | 5 行 `INFO`，含界面地址与 socket 路径 |
| 浏览器打开 `http://127.0.0.1:8788/` | 界面可用 | 聊天界面，插件状态「插件离线」 |
| `curl -sI .../` | 首页与 content-type | `200` + `text/html; charset=utf-8` |
| `curl .../app.css`、`.../app.js` | 静态资源 | `200` + `text/css` / `text/javascript` |
| `POST /api/settings/services` | 写入服务与 key | `{"ok":true}`，配置文件被写回且注释保留 |
| `GET /api/settings` | 读取设置 | key 显示为 `••••••••`，含 `has_api_key` |
| ⚙ 面板里改 key / 切换服务 | 界面配置能力 | 列表与下拉框即时更新，配置落盘 |
| 无插件时发送问题 | 降级路径 | 提示「LLM 插件当前未连接」 |
| `cargo test -p kb_svc_salvo -p kb_core` | 全部契约 | 39 项 `ok` |
| `timeout -s TERM 3 cargo run ...` | 关停清理 | `已清理 socket 文件`，目录为空 |

## 6. 下一阶段（尚未实现）

按 `dev-notes.md` §11 的顺序：

1. **启动并监管 `kb_rig_llm` 子进程**：把 socket 路径通过 `--socket <path>` 与
   `LLM_KB_PLUGIN_SOCKET` 传给子进程，处理退出监测与退避重启；
2. **`kb_rig_llm` + rig 适配 crate**：把 rig 的流式输出转成 `abs_llm::v1` 并上报；
3. **`ServerHandle::stop_graceful`**：把当前「等 3 秒或 abort」换成真正的优雅关停；
4. **浏览器端到端测试**：补一条 Playwright 用例覆盖界面的流式渲染与设置面板。

## 7. 相关文档

- `dev-notes.md` §8 配置与启动约定、§13 已确认的决策与 PoC、§14 Web 界面与用户配置；
- `kb_svc/crates/kb_svc_salvo/src/web/assets/`：前端三件套（HTML / CSS / JS）；
- `kb_svc/crates/kb_svc_salvo/src/plugin_socket.rs`：socket 路径生成与清理的实现与测试。
