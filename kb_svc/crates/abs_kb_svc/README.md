# abs_kb_svc

知识库服务协议的**聚合 crate**：把各个协议 crate 摆到一个稳定的路径下，
让调用方不必逐个记住"哪一版、哪一类对端的协议在哪个 crate 里"。

**它自己一行类型都不定义**，`[dependencies]` 里也只有协议 crate——
没有运行时、没有传输、没有业务实现。

## 聚合出了什么

```text
abs_kb_svc::v1::desktop::X
        │
        └── 别名到 crate `abs_kb_svc_v1_desktop`
                ├── abs_kb_core_handshake   （系统层握手：IpcReadyNotice）
                └── 应用层握手 + 业务数据 + 按域异步 RPC trait
```

| 路径 | 内容 | 定义在 |
| :--- | :--- | :--- |
| `v1::desktop::*` | 桌面客户端 `kb_admin_desktop` × `kb_core` 的全部数据与按域异步 RPC trait | [`abs_kb_svc_v1_desktop`](../abs_kb_svc_v1_desktop/) |
| `v1::desktop::IpcReadyNotice` 等 | **系统层握手**：`kb_core` 公布 IPC 端点的那条通知 | [`abs_kb_core_handshake`](../abs_kb_core_handshake/)，由上一个转出 |
| `v1::desktop::ClientInfo` / `ServerInfo` / `PROTOCOL_VERSION` | **应用层握手**：双方身份与协议版本 | `abs_kb_svc_v1_desktop` |

将来增加"别的对端"（例如 `v1::plugin`）或"别的版本"时也在这里加一行，
**调用方的路径不变**。

## 为什么保留它

协议在 2026-09-18 按"版本 + 对端"拆成了独立 crate：

- `abs_kb_svc_v1_desktop`：协议 v1 的桌面端；
- `abs_kb_core_handshake`：两个层面握手里**系统层**的消息。

拆分之后，实现方（`kb_core`、`kb_svc_servo_ipc`、`kb_core_rproxy`）**一行都不用改**：
它们仍然写 `abs_kb_svc::v1::desktop::X`。这就是聚合层存在的全部理由——
**换布局不动调用方**。

实现方式是给协议 crate 起一个别名，而不是逐个 `pub use` 它的类型：

```rust
pub mod v1 {
    pub use abs_kb_svc_v1_desktop as desktop;
}
```

按 `AGENTS.md` 第 5 条不用通配符；别名整个 crate 相当于"这份清单只在新 crate 的
根文件里逐项列举一次"，不会出现两份需要同步的清单。

## 什么时候不必经过它

需要**整套业务协议**的调用方才该依赖本 crate。只做一件事的调用方直接依赖细粒度
crate 更划算：

| 调用方 | 直接依赖 | 理由 |
| :--- | :--- | :--- |
| `kb_core_starter` | `abs_kb_core_handshake` | 它只要"端点就绪"这一条消息，不必把工作区 / 会话 / 服务那整套业务协议（以及背后的 `abs_llm`）拖进依赖树 |
| 只需要应用层握手 + 工作区/会话列表的客户端 | `abs_kb_svc_v1_desktop` + `abs_kb_core_handshake` | 同上，且少一层间接 |

## 相关文档

- [`abs_kb_svc_v1_desktop/README.md`](../abs_kb_svc_v1_desktop/README.md)：
  协议的设计约定、契约与接口形状（**主要内容在那里**）；
- [`abs_kb_core_handshake/README.md`](../abs_kb_core_handshake/README.md)：
  系统层握手的消息与它的发送/接收方；
- 根 [`README.md`](../../../README.md) §2：各 crate 的分工。
