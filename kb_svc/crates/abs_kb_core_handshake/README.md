# abs_kb_core_handshake

**系统层握手**：`kb_core` 公布自己 IPC 端点的那条通知。

```text
kb_core（子进程）                         启动它的父进程
  Listener::bind(runtime_dir)
  → 往 stdout 打一行 IpcReadyNotice ─────►  读这一行，得到 ipc_name_file
                                            → 之后照常去连 IPC / TCP 端点
```

## 它属于哪一块

客户端要跟 `kb_core` 谈成事，要过两道握手。
**两个层面的消息都是公开协议**，区别只在"消息之外的机制"归谁：

| 层面 | 解决什么 | 消息 | 定义在 |
| :--- | :--- | :--- | :--- |
| **系统层** | 找得到、连得上 | `IpcReadyNotice` + `HandshakeNoticeKind`（`event: "ipc_ready"`） | **本 crate** |
| **应用层** | 谈得成 | `PROTOCOL_VERSION` / `ClientInfo` / `ServerInfo` / `Request::Hello` → `Reply::Hello` → `Event::Ready` | [`abs_kb_svc_v1_desktop`](../abs_kb_svc_v1_desktop/)（经 `abs_kb_svc` 聚合） |

"机制"指挑哪种内核端点、端点放在哪、失败怎么重试——那些仍由传输实现决定
（`kb_svc_servo_ipc` 那条"扫运行时目录"的路径压根不经过本 crate）。

## 为什么是一个独立的 crate

因为**只想启动并找到 `kb_core` 的调用方不该依赖整套业务协议**。

`kb_core_starter` 就是这种调用方：它起 `kb_core` 子进程、读这一行通知，就结束了。
如果这条通知留在业务协议 crate（`abs_kb_svc_v1_desktop`）里，那么
`kb_core_starter` 会连带依赖工作区 / 会话 / 服务那整套类型，以及它们背后的
`abs_llm` 等一串东西——为了一个只有四个字段的通知。

单独成 crate 之后，本 crate 的依赖只有 `serde`（连 `std` 都不开，
`alloc` 就够），`kb_core_starter` 也只需要依赖它。

## 谁发、谁收

| 谁 | 做什么 |
| :--- | :--- |
| `kb_core`（`serve_`） | 用 `IpcReadyNotice` **序列化**，`writeln!` 到 stdout 并 flush；stdout 其余时间保持干净（默认 `--handshake-prompt none`） |
| `kb_core_starter` | `serde_json::from_str::<IpcReadyNotice>` **反序列化**，取出 `ipc_name_file` |

共用同一份定义的收益是：改字段名、改 `event` 取值都会**编译不过**，
而不是"跑起来才发现启动不了"。这正是把这条通知从"两边各自拼 JSON"提升为协议的目的。

## 它承诺什么、不承诺什么

- 只承诺 **`ipc_name_file` 这个文件名**：`kb_core` 会把"当前可连的端点名"
  写进那个文件，但那是它真正开始 `accept` 时的事；
- 因此拿到通知之后**仍可能有短暂连不上**，连接方必须带重试
  （`kb_svc_servo_ipc::Client` 就是这么做的）；
- 独立运行、不需要父进程转告端点的客户端（自己扫运行时目录的那种）不经过这条通知。

## 线格式与验证

```json
{"event":"ipc_ready","ipc_name_file":"…/kb-20260918-….ipc","protocol_version":1,"pid":1234}
```

线格式由单元测试钉住：JSON 的字面量与结构体互相往返、`event` 必须是字符串
`"ipc_ready"`（写枚举名 `"IpcReady"` 或别的值都解不开），并额外验证
postcard 往返（本仓库对协议类型的通用约束是"不依赖自描述格式"）。

```bash
cargo test -p abs_kb_core_handshake
```

## 相关文档

- [`abs_kb_svc_v1_desktop/README.md`](../abs_kb_svc_v1_desktop/README.md)：
  应用层握手与业务协议；
- [`kb_core_starter/README.md`](../kb_core_starter/README.md)：本 crate 的主要调用方；
- [`dev-notes/kb_admin_desktop-20260918-1034.md`](../../../dev-notes/kb_admin_desktop-20260918-1034.md) §9：
  两个层次都算公开协议这条决定的来龙去脉。
