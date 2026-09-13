# kb_core

知识库主进程的可执行入口。

依据 `dev-notes.md` §13.1 的分工：`kb_svc_salvo` 是**纯库**（提供 HTTP / WebSocket 路由、
会话逻辑与监听器组装），而**进程的启动与编排由本 crate 负责**。现阶段两者视为一体，
将来再拆分。

> **当前状态：PoC（阶段 0）**
> 目前 `kb_core` 只完成「双监听器 + 回显」这一最小闭环，用于验证
> 「Unix domain socket 上能否跑 WebSocket」（`dev-notes.md` §13.5）。
> 会话状态机、插件子进程监管、前端页面都在后续阶段。

---

## 1. 它做了什么

启动后**同时**监听两个服务：

| 服务 | 监听位置 | 用途 |
| :--- | :--- | :--- |
| 用户侧 HTTP / WebSocket | `127.0.0.1:8788`（默认） | 将来给浏览器用；当前提供 `GET /` 与 `GET /ws`（回显） |
| 插件侧 WebSocket | Unix domain socket 文件 | 将来给 `kb_rig_llm` 子进程用，与用户侧共享同一套路由表 |

两个监听器用 Salvo 的 `JoinedListener` 合并，因此**共用同一份路由表**：
同一套 handler 既能在 TCP 上访问、也能在 UDS 上访问。

## 2. 依赖的外部变量与约定

### 2.1 环境变量

| 变量 | 是否必需 | 作用 |
| :--- | :--- | :--- |
| `XDG_RUNTIME_DIR` | 否 | 存放插件通道 socket 的**父目录**。设置时用 `$XDG_RUNTIME_DIR/llm_kb`；未设置时回退到系统临时目录下的 `llm_kb`（例如 `/tmp/llm_kb`）。 |
| `RUST_LOG` | 否 | 日志过滤，走 `env_logger`。默认 `info`；排查问题可设 `RUST_LOG=debug`（会打印 WebSocket 收发的每一条消息）。 |
| `LLM_KB_PLUGIN_SOCKET` | 否（**尚未使用**） | 预留：启动 `kb_rig_llm` 子进程时，父进程用它把 socket 路径传给子进程，见 §5。 |

### 2.2 socket 文件名的生成规则（重要）

socket **文件名不接受命令行参数或环境变量指定**，每次启动时由 `kb_svc_salvo`
按「日期 + UUID」生成：

```text
$XDG_RUNTIME_DIR/llm_kb/kb-20260913-f9c628f955594c1fa73965bcac42f091.sock
                    └─ 日期 ─┘└──────── UUID v4 ────────┘
```

- 目录权限 `0700`，socket 文件权限 `0600`；
- 因为带 UUID，每次启动的名字都不同，**历史残留文件不会导致启动失败**；
- 进程被 `SIGKILL` 强杀时文件会残留（无法拦截），但不会影响下次启动，可以手动清理。

`kb_core` 启动后会把生成的完整路径打进日志，插件子进程将来从父进程处获得该路径。

### 2.3 命令行参数

```text
kb_core [--runtime-dir <dir>] [tcp_addr]
```

| 参数 | 默认值 | 说明 |
| :--- | :--- | :--- |
| `tcp_addr` | `127.0.0.1:8788` | 用户侧监听地址。端口写 `0` 表示由系统分配临时端口。 |
| `--runtime-dir <dir>` | `$XDG_RUNTIME_DIR/llm_kb` | 只覆盖 socket 的**目录**，文件名规则不变。 |

参数顺序不敏感；未知参数会被忽略并打印警告。

## 3. 从最简单的用例开始

### 3.1 启动

```bash
cargo run -p kb_core
```

预期看到三行 `INFO` 日志，注意最后一行给出的 socket 路径：

```text
[<时间戳> INFO  kb_svc_salvo::poc] kb_svc_salvo poc bound: tcp=127.0.0.1:8788 uds=/run/user/1000/llm_kb/kb-20260913-<uuid>.sock
[<时间戳> INFO  kb_core] kb_core listening: tcp=127.0.0.1:8788
[<时间戳> INFO  kb_core] kb_core plugin socket: /run/user/1000/llm_kb/kb-20260913-<uuid>.sock
```

想固定路径、避免污染用户运行时目录时，用：

```bash
mkdir -p /tmp/kb-rt
cargo run -p kb_core -- --runtime-dir /tmp/kb-rt 127.0.0.1:8788
```

### 3.2 用浏览器访问

打开 <http://127.0.0.1:8788/>，页面会显示一行文本：

```text
kb_svc_salvo poc: GET /ws to open a websocket
```

这证明**用户侧 HTTP 服务**可用。（真正的聊天界面要等前端阶段。）

## 4. 一句命令能测出什么

下列命令都假定服务端已在 `127.0.0.1:8788` 运行；用 `--runtime-dir /tmp/kb-rt` 时，
socket 文件在 `/tmp/kb-rt/` 下。

### 4.1 用户侧 HTTP：路由可达

```bash
curl -i http://127.0.0.1:8788/
```

预期：

```text
HTTP/1.1 200 OK
content-type: text/plain; charset=utf-8
content-length: 46

kb_svc_salvo poc: GET /ws to open a websocket
```

### 4.2 socket 文件真的被创建了，且权限正确

```bash
ls -la "${XDG_RUNTIME_DIR:-/tmp}/llm_kb/"
```

预期：目录是 `drwx------`，其中有一个 `srw-------`（`s` = socket）文件，名字形如
`kb-20260913-<32 位十六进制>.sock`。

### 4.3 插件侧 UDS：普通 HTTP 请求同样可达

shell 无法直接 `open` 一个 socket 文件（`bash: No such device or address`），
所以用 Python 标准库发一个最小 HTTP/1.1 请求：

```bash
SOCK=$(ls "${XDG_RUNTIME_DIR:-/tmp}"/llm_kb/*.sock | head -1)
python3 - "$SOCK" <<'PY'
import socket, sys
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect(sys.argv[1])
s.sendall(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
data = b""
while chunk := s.recv(4096):
    data += chunk
print(data.decode())
PY
```

预期：响应与 §4.1 完全一致（都是同一份路由表），证明 **UDS 监听器可用**。

### 4.4 WebSocket 回显：UDS 上真的跑得起来（PoC 的核心结论）

自动化验证（**推荐**）：

```bash
cargo test -p kb_svc_salvo
```

预期：9 项全绿（5 个单元测试 + 4 个集成测试），其中三项直接对应本 PoC 的结论——

```text
test poc_ws_over_unix_socket ... ok          # UDS 上握手 101 + 双向回显
test poc_ws_and_http_over_tcp ... ok         # TCP 上同样可用，且共享路由表
test poc_socket_path_is_generated_with_date_and_uuid ... ok   # 路径规则 + 0600 权限
```

交互式验证（需要 `wscat`）：

```bash
wscat -c ws://127.0.0.1:8788/ws
# 连上后输入任意文本并回车，例如：
# > hello
# < hello
```

> 说明：本仓库的验证环境里 `wscat` 无法在管道/后台方式下跑通，因此 §4.4 的
> 结论以 `cargo test` 为准；`wscat` 只作为有真实终端时的手工手段。

### 4.5 优雅退出会清理 socket 文件

```bash
rm -rf /tmp/kb-rt
timeout -s TERM 3 cargo run -p kb_core -- --runtime-dir /tmp/kb-rt 127.0.0.1:8788
ls -A /tmp/kb-rt/          # 预期：没有任何输出，socket 已被删除
```

`timeout` 会在 3 秒后发送 `SIGTERM`。预期日志（顺序固定）：

```text
[<时间戳> INFO  kb_core] 收到 SIGTERM，开始优雅退出
[<时间戳> WARN  kb_core] 服务端未在 3 秒内退出，已放弃等待
[<时间戳> INFO  kb_core] 已清理 socket 文件: /tmp/kb-rt/kb-20260913-<uuid>.sock
```

交互式终端里按 `Ctrl-C`（`SIGINT`）走的是同一条路径。

> `SIGKILL`（`kill -9`）无法被捕获，socket 文件会残留；因文件名带 UUID，不影响下次启动。

## 5. 预期结果一览

| 命令 | 验证的内容 | 预期结果 |
| :--- | :--- | :--- |
| `cargo run -p kb_core` | 进程可启动，两个监听器都 bind 成功 | 三行 `INFO`，其中包含生成的 socket 路径 |
| `curl -i http://127.0.0.1:8788/` | 用户侧 HTTP 路由可达 | `200 OK` + `kb_svc_salvo poc: GET /ws to open a websocket` |
| `ls -la <runtime_dir>/` | socket 文件已创建、权限收紧 | `srw-------` 文件，目录 `drwx------` |
| Python UDS 单行脚本（§4.3） | 插件侧 UDS 可达、与 TCP 共享路由 | 与 `curl` 完全相同的响应 |
| `cargo test -p kb_svc_salvo` | UDS/TCP 上的 WebSocket 握手与双向收发、路径规则、清理 | 9 项全部 `ok` |
| `cargo test -p kb_core` | 命令行参数解析 | 3 项全部 `ok` |
| `timeout -s TERM 3 cargo run ...` | 收到信号后清理 socket 文件 | 目录为空 |

## 6. 下一阶段（尚未实现）

按 `dev-notes.md` §11 的顺序，本 crate 接下来要做：

1. **启动 `kb_rig_llm` 子进程**：把 `bound.socket_path()` 通过 `--socket <path>`
   与 `LLM_KB_PLUGIN_SOCKET` 两种方式传给子进程；
2. **插件子进程监管**：退出监测、指数退避重启、退出时回收（避免僵尸进程）；
3. **会话状态机与线协议**：把用户侧与插件侧的事件打通（`wire.rs`）；
4. **前端静态资源**：`include_str!` 内嵌单页 HTML + 原生 JS，复刻 DSH 观感；
5. **配置与日志完善**：配置文件 / 更多环境变量 / `RUST_LOG` 之外的日志选项。

## 7. 相关文档

- `dev-notes.md` §8 配置与启动约定、§10 缺少的内容、§13 已确认的决策与 PoC 进展；
- `kb_svc/crates/kb_svc_salvo/src/plugin_socket.rs`：socket 路径生成、权限与清理的实现与单元测试。
