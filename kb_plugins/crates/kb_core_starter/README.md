# kb_core_starter

用命令行启动一个 `kb_core` 子进程，并**异步地**等它公布自己的 **IPC 端点文件名**。

```text
调用方 ──Command::new("kb-core") --handshake-prompt=stdio──► kb_core（子进程）
       ◄──── stdout 上的一行 IpcReadyNotice（JSON）────────┘
```

它与 `kb_core_rproxy`、`kb_svc_servo_ipc` 一起放在 `kb_plugins/crates/`：
按本仓库的分组，**"插件与配套进程"**这一类装的是"围着一个 `kb_core` 转"的东西
（网关、启动器、传输实现），而 `kb_svc/crates/` 只留协议与主进程本身。

## 它实现协议里的哪一块

握手分两个层面，**两个层面的消息都是公开协议**，但"谁实现哪一半"不同：

| 层面 | 解决什么 | 消息 | 谁实现 |
| :--- | :--- | :--- | :--- |
| **系统层** | 找得到、连得上 | [`IpcReadyNotice`](../../../kb_svc/crates/abs_kb_core_handshake/src/system_.rs)（`event: "ipc_ready"`，带 `ipc_name_file` / `protocol_version` / `pid`） | **本 crate 收**；`kb_core::serve_` 发 |
| **应用层** | 谈得成 | `Request::Hello` → `Reply::Hello` → `Event::Ready` | `kb_svc_servo_ipc::Client`（本机 IPC）/ 将来的 TCP 客户端 |

所以本 crate **只实现系统层握手的接收侧**，具体是：

- 把通知解成协议类型而不是手抠 JSON 字段（字段名改了会编译不过）；
- 把"等 stdout 那一行"做成可取消、与运行时无关的等待（见下）；
- 管好子进程的归属（见下）。

它**不实现**的：

- **应用层握手**（`TrHandshake` / `Request::Hello`）：那是连上之后的事；
- **"怎么连上端点"**：它只把名字文件交出去，连接由调用方用
  `kb_svc_servo_ipc::Client::connect(runtime_dir)`（或远程的 TCP 客户端）完成。

系统层的消息类型在 [`abs_kb_core_handshake`](../../../kb_svc/crates/abs_kb_core_handshake/) 而不在本 crate：
那是一份只依赖 `serde` 的纯协议 crate，谁都可以依赖它；而起进程、读管道这些
`std::process` 的事不能进协议 crate。**本 crate 的依赖里只有这一个协议 crate**，
没有 `abs_kb_svc` / `abs_kb_svc_v1_desktop`。

## 为什么是一个独立的 crate

1. **它有两个调用方，而且分属不同的 workspace**：`kb_core_rproxy`
   （主 workspace 的 plugins）与 `kb_admin_desktop` 的 Rust 侧（**独立 workspace**）。
   这段逻辑原来长在 `kb_core_rproxy` 里，而那个 crate 只有 `[[bin]]`、没有 lib
   target，外部拿不到它——留在原地就等于让桌面端复制一份。
2. **它不属于任何单个调用方**：桌面端要起"本机的 `kb_core`"，rproxy 要起"上游的
   `kb_core`"，两边是同一件事（同一套参数、同一种收场语义）。
3. **依赖面刻意最小**：只要 `abs_kb_core_handshake` 的**消息类型**、`abs_cancel` 与
   `serde_json`；**不要 compio、不要 ipc-channel、也不要业务协议**。桌面端把它编进
   自己的原生库时，不会被拖进一个 TCP 运行时、本机 IPC 实现或整套业务类型。
4. **它有自己的收场语义**，值得独立测试与文档：子进程归属、取消、future 被丢弃
   三条路径必须都收敛到"不留下孤儿进程"。

## 形状

```rust
let spec = LaunchSpec {
    kb_core: "/usr/local/bin/kb-core".into(),
    runtime_dir: "/run/user/1000/llm_kb".into(),
    storage_dir: "/run/user/1000/llm_kb/storage".into(),
};

let launched = start(&spec).await?;                      // 不可取消
let launched = start(&spec).may_cancel_with(token).await?; // 可取消
```

`start()` 返回的 future **与异步运行时无关**：`read_line` 发生在一条专职线程上，
future 只轮询完成量与取消令牌，所以 tokio / compio / `futures_lite::block_on`
都能驱动它。**它不自带定时器**——"等多久算超时"由调用方造取消令牌决定
（`abs_cancel` 令牌），这正是"超时策略归调用方、等待机制归本 crate"的切法。

三条收场路径共用同一个子进程守卫：

| 收场 | 子进程 |
| :--- | :--- |
| 成功 | 所有权交给 `Launched`，它在 `Drop` 时结束进程 |
| 取消 / 读通知失败 | 守卫被丢弃 → 结束进程 |
| future 被直接丢弃 | 同上 |

## 验证

```bash
cargo test -p kb_core_starter
```

2 个单元测试（通知解析）+ 7 个集成测试（用假 `kb_core` 脚本覆盖 正常 / 提前退出 /
坏通知 / 可执行文件不可用 / 已取消不起进程 / **等待中取消并结束子进程** /
`kb_core_beside`）+ 3 个文档测试。

## 相关文档

- [`abs_kb_core_handshake`](../../../kb_svc/crates/abs_kb_core_handshake/README.md)：
  两个层面握手各自解决什么、为什么分开；
- [`kb_core_rproxy`](../kb_core_rproxy/README.md)：
  本 crate 的第一个调用方（网关）。
