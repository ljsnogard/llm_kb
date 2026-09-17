# kb_core

知识库**主进程**的可执行入口。

> **当前状态：IPC 已接入，工作区与会话的增删查改能整条走通**
> 已有：命令行入口、compio 异步运行时、工作区与会话的**本地文件存储**、
> **`kb_svc_servo_ipc` 服务端**（客户端连上来之后，请求经 IPC → 派发 →
> 存储 → 应答原路返回），以及一组临时的增删查改子命令（不开客户端时手工检验用）。
> 还没有：LLM 生成（`Ask` / 事件流）、设置、目录浏览那几个域——
> 它们的 trait 尚未落地，服务端会明确回 `BadRequest`。
>
> 与上一版相比：**`kb_svc_salvo` 与 `tokio` 已从本 crate 移除**，
> 随之删掉了 HTTP / WebSocket 监听、`--config`、`tcp_addr` 位置参数，
> 以及前端资源覆盖目录 `--assets-dir`（前端资源本就属于 `kb_svc_salvo`）。
> 进程间通信现在由 `kb_svc_servo_ipc` 承担。

---

## 1. 先跑起来

### 1.1 启动服务

```bash
cargo run -p kb_core
```

预期日志（时间戳与实际目录随环境变化）：

```text
[<时间戳> INFO  kb_core::serve_] kb_core v0.1.0（协议 v1）
[<时间戳> INFO  kb_core::serve_] 运行时目录: /run/user/1000/llm_kb
[<时间戳> INFO  kb_core::serve_] 存储目录: /run/user/1000/llm_kb/storage
[<时间戳> INFO  kb_core::serve_] IPC 端点文件: /run/user/1000/llm_kb/kb-20260917-<uuid>.ipc
[<时间戳> INFO  kb_core::serve_] 已登记工作区: 0 个
[<时间戳> INFO  kb_core::serve_] 等待客户端连接（Ctrl-C 退出）
```

**进程会一直在那里**，直到你按 `Ctrl-C`。客户端（今天的
`kb_svc_servo_ipc::Client`，将来的 `kb_admin_desktop`）靠 `IPC 端点文件`
里那个名字找上来：

```text
运行时目录/
└── kb-<日期>-<uuid>.ipc     本次启动专属；内容是当前可连的端点名
```

**文件名由 `kb_core` 决定**，沿用旧 `plugin_socket` 的「日期 + UUID」约定：
每次启动都独一无二，历史残留不会挡路。文件名的语义是"这个服务端实例"，
**内容**才是"当前可连的端点名"——服务端每接受一个客户端就重建端点并把新端点名
写回去，没有在等连接时写空串。

进程被强杀时名字文件会留下（内容指向已经不存在的端点），客户端会重试到超时；
重新启动 `kb_core` 时会把运行时目录里所有 `kb-*.ipc` 清掉。

不想污染用户目录时，把两个目录都指到 `/tmp`：

```bash
cargo run -p kb_core -- --runtime-dir /tmp/kb-core/run --storage-dir /tmp/kb-core/data
```

### 1.2 从零跑完一轮增删查改

有两条等价的路：**走 IPC**（真实链路）或**用临时子命令**（不开客户端）。
两者共用同一个存储层，因此看到的数据是一样的。

#### 走 IPC

```rust
// 客户端侧（`kb_svc_servo_ipc` 里已经有一份端到端测试，见 tests/round_trip.rs）
use abs_kb_svc::v1::desktop::TrWorkspaceService;
use kb_svc_servo_ipc::Client;

let client = Client::connect("/tmp/kb-core/run")?;
let workspaces = client.list_workspaces().await?;
```

#### 用临时子命令

下面这段可以直接整段复制执行（`kb` 是一个把公共参数固定下来的 shell 函数）：

```bash
cargo build -p kb_core

kb() { ./target/debug/kb-core \
  --runtime-dir /tmp/kb-core/run \
  --storage-dir /tmp/kb-core/data "$@"; }

# ── 增：新建工作区与会话，标识由 kb_core 生成 ──────────────────────
WID=$(kb workspace add --name 笔记 --path /tmp/notes | cut -f1)
SID=$(kb session add --workspace "$WID" --title 第一问 | cut -f1)

# ── 查：列表与详情 ────────────────────────────────────────────────
kb workspace list
kb session list --workspace "$WID"
kb session show  --workspace "$WID" "$SID"

# ── 改：重命名 ────────────────────────────────────────────────────
kb workspace update "$WID" --name 资料库
kb session update   --workspace "$WID" "$SID" --title 改名后

# ── 删：先删会话，再删工作区（删工作区会级联删掉它名下的会话）────────
kb session remove   --workspace "$WID" "$SID"
kb workspace remove "$WID"
```

预期输出（标识是每次运行新生成的 UUID，形状固定、取值不同）：

```text
$ kb workspace list
w-3f2b9c1d4e5a4b7c8d9e0f1a2b3c4d5e	笔记	/tmp/notes
# 共 1 个工作区

$ kb session list --workspace "$WID"
s-9a1c77e0b2d34f5a8c6e0b1d2f3a4c5b	第一问	0 条	更新于 1789659249094
# 共 1 个会话

$ kb session show --workspace "$WID" "$SID"
{
  "summary": {
    "session_id": "s-9a1c77e0b2d34f5a8c6e0b1d2f3a4c5b",
    "workspace_id": "w-3f2b9c1d4e5a4b7c8d9e0f1a2b3c4d5e",
    "title": "第一问",
    "updated_at_millis": 1789659249094,
    "turn_count": 0
  },
  "turns": []
}

$ kb workspace update "$WID" --name 资料库
w-3f2b9c1d4e5a4b7c8d9e0f1a2b3c4d5e	资料库	/tmp/notes

$ kb session update --workspace "$WID" "$SID" --title 改名后
s-9a1c77e0b2d34f5a8c6e0b1d2f3a4c5b	改名后	0 条	更新于 1789659249108

$ kb workspace remove "$WID"
已删除工作区 w-3f2b9c1d4e5a4b7c8d9e0f1a2b3c4d5e（连同它名下的会话）
```

`workspace list` / `session list` 的每行是「制表符分隔的若干字段 + 最后一行
`# 共 N 个`」，所以 `cut -f1` 就能取到标识，像上面那样串起来用。

### 1.3 自动化验证

```bash
cargo test -p kb_core
```

预期 26 项全部 `ok`，其中覆盖了存储语义、命令行解析，以及**整条 IPC 链路**：

```text
test ipc_::tests_::ipc_round_trip_reaches_the_local_store_ ... ok     # 客户端→IPC→Store→磁盘
test store_::tests_::store_operations_honor_cancellation_ ... ok     # 取消令牌真的生效且不留副作用
test ipc_::tests_::store_errors_map_to_protocol_codes_ ... ok         # 存储错误 → ErrorCode
test store_::tests_::add_workspace_writes_documented_layout_ ... ok   # 文件落在约定路径
test store_::tests_::remove_workspace_cascades_sessions_ ... ok       # 删工作区级联删会话
test store_::tests_::create_session_derives_title_and_persists_turns_ ... ok
test store_::tests_::list_sessions_is_newest_first_ ... ok            # 最近活动的会话在前
test store_::tests_::invalid_id_is_rejected_before_touching_disk_ ... ok
test store_::tests_::broken_file_reports_decode_error_with_path_ ... ok
test args_::tests_::parse_workspace_commands_ ... ok                  # 子命令解析
test args_::tests_::parse_rejects_bad_input_ ... ok                   # 坏参数被明确拒绝
...
```

`ipc_round_trip_reaches_the_local_store_` 是那个"真起服务端、真走 ipc-channel、
真落盘"的用例：它在独立线程上用 `Listener` 起服务端、用 `Client` 连上去跑完一轮
增删查改，并回到服务端的存储目录里核对 JSON 文件确实写出来了。

---

## 2. 命令行参考

```text
kb-core [--runtime-dir <目录>] [--storage-dir <目录>] [<子命令>]
```

### 2.1 全局选项

| 选项 | 默认值 | 说明 |
| :--- | :--- | :--- |
| `--runtime-dir <目录>` | `$XDG_RUNTIME_DIR/llm_kb`（回退系统临时目录下的 `llm_kb`） | 运行时目录。IPC 端点名字文件 `kb-<日期>-<uuid>.ipc` 放在这里。 |
| `--storage-dir <目录>` | `<运行时目录>/storage` | 工作区与会话的存储目录。显式给出时与运行时目录完全独立。 |
| `-h` / `--help` | — | 打印用法说明。 |

### 2.2 子命令

| 子命令 | 作用 |
| :--- | :--- |
| `workspace add --name <名字> --path <目录>` | 新建工作区，标识由 `kb_core` 生成。 |
| `workspace list` | 列出全部工作区（按标识升序）。 |
| `workspace show <工作区标识>` | 查看单个工作区。 |
| `workspace update <工作区标识> [--name <名字>] [--path <目录>]` | 改名字 / 改路径，两者至少给一个。 |
| `workspace remove <工作区标识>` | 删除工作区，并级联删除它名下的会话。 |
| `session add --workspace <工作区标识> [--title <标题>]` | 新建会话；不给标题时从首条用户消息推导。 |
| `session list --workspace <工作区标识>` | 列出该工作区下的会话（按最近活动时间降序）。 |
| `session show --workspace <工作区标识> <会话标识>` | 打印会话完整内容（协议类型的 JSON）。 |
| `session update --workspace <工作区标识> <会话标识> --title <标题>` | 改会话标题。 |
| `session remove --workspace <工作区标识> <会话标识>` | 删除会话。 |

不带子命令即进入常驻骨架（§1.1）。选项与位置参数**顺序不敏感**；
不支持 `--key=value` 与短选项合并。

### 2.3 退出码

| 退出码 | 含义 |
| :---: | :--- |
| `0` | 成功（常驻骨架被信号终止时由信号决定，见下）。 |
| `1` | 运行期失败：标识不合法、目标不存在、文件读写失败等，错误同时写进日志。 |
| `2` | 参数错误：未知选项/子命令、缺参数、多余的参数；用法说明会一起打到 stderr。 |

常驻骨架没有安装信号处理器，`Ctrl-C`（`SIGINT`）走操作系统的默认处置，直接结束。

---

## 3. 数据存在哪里

工作区与会话暂时用**本地文件**代替 Turso。文件内容就是
`abs_kb_svc::v1::desktop` 里协议类型的 JSON 表示——**存储格式与线上格式同源**，
不会出现"能存进去、却发不出去"的字段。

```text
<storage_dir>/
├── workspaces/
│   └── <workspace_id>.json        一个工作区（`Workspace`）
└── sessions/
    └── <workspace_id>/
        └── <session_id>.json      一个会话（`SessionDetail`：摘要 + 全部消息）
```

三条硬性约定（都有单元测试守着）：

1. **标识必须能安全地当文件名**：只允许 ASCII 字母、数字、`-`、`_`，且不超过
   128 字节。协议把标识当作不透明字符串，手工构造（如 `"w-1"`）是允许的，
   所以存储层不能假定它一定由 `generate()` 产生；含 `../` 的标识会在碰盘之前
   就被拒绝。
2. **写入先落临时文件再 `rename`**：读到的内容要么是旧的、要么是新的，
   不会是半个 JSON。进程被强杀时最坏留下一个 `*.json.tmp`，它不会被当作对象。
3. **会话属于工作区**：`create_session` / `list_sessions` 会先确认工作区存在；
   删除工作区会级联删除它的会话目录。工作区不存在时 `list_sessions`
   **返回错误而不是空列表**——否则客户端把标识写错时，会误以为"这里没有会话"。

---

## 4. 设计说明

### 4.1 为什么去掉 `kb_svc_salvo` 与 `tokio`

上一版里 `kb_core` 只是 `kb_svc_salvo::launch::launch` 的薄壳：HTTP / WebSocket
监听、会话逻辑、启动编排全在 `kb_svc_salvo` 里，本 crate 只解析参数。
团队已决定把进程间通信从 socket / HTTP 换成共享内存方向的 IPC，`kb_svc_salvo`
整体废弃，因此本 crate 对它的依赖、以及它带来的 `tokio` 依赖一并删除。

同时删掉的还有三个只为那套 HTTP 通道存在的入口：

- `tcp_addr` 位置参数与 `--config`：前者是用户侧 HTTP 监听地址，后者是
  `kb_svc_salvo` 的 LLM 服务配置文件；
- `--assets-dir`：前端资源**覆盖目录**（调试页面时用）。前端资源本身属于
  `kb_svc_salvo`，`kb_core` 不再持有任何页面。

### 4.2 为什么是 compio

进程间通信将走 `kb_svc_servo_ipc`（servo/ipc-channel）。ipc-channel 不是
异步 API，且本机实测表明：在异步任务里内联调用它的阻塞 `recv()`，
会让 `current_thread` 运行时在 300 ms 内一次定时任务都跑不到。
因此业务侧需要一个**能把阻塞收敛到别处**的运行时，而 compio 的完成式模型
正好承接这一点。

具体到本 crate：`main` 用 `#[compio::main]`；本地文件的读写走 `compio::fs`
（io_uring）；目录遍历与递归删除这两个 compio 0.19 还没提供异步版本的操作，
交给 `compio::runtime::spawn_blocking`，**不占用执行器线程**。

### 4.3 存储层为什么单独一块

存储的全部语义都在 `src/store_/` 里，对外只有 `Store` 一个类型。这样做是为了让
"换成数据库"这件事有明确的边界：下一轮 IPC 的请求处理只依赖 `Store` 的方法，
替换实现时不需要动请求/应答的代码。

存储层**没有裸 `async fn`**：每个操作都由 `gen_mcf2::gen_may_cancel_future`
展开成一对类型，因此两条调用路径都可用：

```text
store.list_workspaces().await?                      // 不可取消
store.list_workspaces().may_cancel_with(t).await?   // 可取消
```

取消的落点是 `store_::race_cancel_`：整个操作体与取消信号赛跑，令牌先触发就返回
`StoreError::Cancelled`，尚未完成的等待随 future 一起被丢弃。
`KbService` 把 IPC 请求带的令牌继续传给它（`.may_cancel_with(cancel)`），
所以"客户端撤销了一次调用"能一路传到最后一次文件等待。

私有辅助（`read_json_` / `write_json_` / `create_dir_all_` …）**没有各自再包一层
宏**：它们只在已经被令牌包住的操作体里被调用，等待会随外层 future 一起被丢弃，
没有第二个调用者需要独立的 future 类型。细节与理由见 `src/store_/mod.rs` 的模块文档。

### 4.4 与 `abs_kb_svc` 协议的关系

存储层直接使用 `abs_kb_svc::v1::desktop` 的 `Workspace` / `SessionSummary` /
`SessionDetail` / `Turn` 等类型，不在此处镜像第二套结构。因此：

- 文件里的 JSON 与将来 IPC 上跑的是同一批类型；
- 协议里"标识由 `kb_core` 分配"、"只发差异不整棵树"（列表只回摘要、正文按需
  拉取）这两条约定，在存储层的接口形状上已经体现出来。

临时的 CRUD 子命令（`src/cli_.rs`）只是把这些方法挂到命令行上，
**不构成对外约定**；IPC 接通后可以保留为维护命令，也可以整体删除。

### 4.5 模块地图

| 文件 | 职责 |
| :--- | :--- |
| `src/main.rs` | 入口：初始化日志 → 解析参数 → 分发 → 决定退出码 |
| `src/args_.rs` | 命令行与子命令解析；`USAGE` 是唯一的用法说明来源 |
| `src/error_.rs` | 进程级错误：把存储失败与 IPC 失败收在一处 |
| `src/serve_.rs` | 常驻服务：打开存储、`Listener::bind`、循环 accept + serve |
| `src/ipc_.rs` | `KbService`：按域 RPC trait 的服务端实现（转发给 `Store`） |
| `src/cli_.rs` | 临时的 CRUD 子命令实现 |
| `src/store_/` | 本地文件存储：`Store`、错误类型、布局与标识校验 |

### 4.6 一次请求的完整路径

```text
kb_admin_desktop / 测试客户端
    │  Client::add_workspace(request)                  ← kb_svc_servo_ipc 的代理
    │  ── 请求通道 ──►  Connection::serve              ← kb_core 的常驻循环
    │                       │  dispatch_()
    │                       ▼
    │                   KbService::add_workspace()     ← src/ipc_.rs
    │                       │
    │                       ▼
    │                   Store::add_workspace()         ← src/store_/mod.rs
    │                       │  写 workspaces/<id>.json
    │  ◄── 应答通道 ────────┘  Reply::WorkspaceAdded { local_id, workspace }
    └─ 代理把载荷取出来返回 Workspace
```

业务错误（例如"工作区不存在"）在 `KbService` 里就翻成
`RpcError::Business(ErrorReply)`，到线上是 `Reply::Error`；传输失败（对端断开）
则走客户端的 `RpcError::Transport`。两类错误的划分见
`kb_svc/crates/abs_kb_svc/README.md` §5 第 7 条。

---

## 5. 下一轮（尚未实现）

1. **其余业务域**：`Hello`（握手）、设置（`ListServices` / `UpsertService`…）、
   目录浏览（`ListDirectory`）、生成（`Ask` / `Cancel` + 事件流）。
   它们的按域 trait 还没落地，服务端现在对它们明确回 `BadRequest`；
2. **并发服务多个客户端**：当前一次只服务一个连接（上一个断开才回到 `accept`）。
   要并发就得把 `accept` 循环与 `serve` 拆到不同任务上，并处理端点的生命周期；
3. **优雅退出**：当前依赖操作系统的默认信号处置；将来要显式撤下端点名字、
   通知在连的客户端；
4. **`kb_admin_desktop` 接上**：客户端侧目前只有 `kb_svc_servo_ipc::Client`
   这个 Rust 代理，Flutter 那边还没有桥接。

---

## 6. 相关文档

- `dev-notes/kb_svc_servo_ipc-20260917-1548.md`：IPC 落地方案、
  三个未知数的实测结论、以及本 crate 与 IPC 的分工；
- `dev-notes/abs_kb_svc-20260917-1254.md`：IPC 选型（ipc-channel）与异步 RPC 设计；
- `dev-notes/kb_admin_desktop-20260917-1341.md`：客户端通信需求与协议数据来源；
- `kb_svc/crates/abs_kb_svc/README.md`：业务通信抽象层的定位、契约与接口形状；
- `kb_svc/crates/abs_kb_svc/src/v1/desktop/mod.rs`：协议 v1 数据定义的入口。
