# llm_kb

`llm_kb` 是个人知识库应用：知识内容主要来自 LLM 服务生成的内容以及用户个人编辑整理，
最终由一个以 **Turso** 为载体的知识数据库持有，对外提供增删查改与搜索、检索能力。
其他能力（与 LLM 通信、编辑管理界面……）以**插件**的形式存在。

进程之间走**本机 IPC**（`kb_svc_servo_ipc`，基于 servo/ipc-channel）；
需要跨机（比如局域网另一台机器）时，由 `kb_core_rproxy` 把 IPC 暴露成 TCP。

---

## 1. 目录结构

```text
llm_kb/
├── kb_svc/crates/       知识库服务本身：主进程 + 协议 + 传输实现
├── kb_plugins/crates/   插件与配套进程：LLM 插件、跨机网关
├── kb_clients/          客户端应用（Flutter）
├── dev-notes/           开发日志：技术决策的前因后果（新记录都写这里）
├── external/            临时验证工程（不属于 workspace）；整个目录已 gitignore
├── AGENTS.md            代码修改纪律
├── Cargo.toml           workspace 定义与共用依赖
└── rust-toolchain.toml  工具链锁定
```

## 2. 各 crate 的分工

### `kb_svc/crates/` —— 服务端

| crate | 职责 | 状态 |
| :--- | :--- | :--- |
| `kb_core` | **主进程**（可执行文件 `kb-core`）：持有工作区与会话的存储、把按域 RPC 实现接上 IPC、常驻服务客户端 | ✅ 可跑 |
| `abs_kb_svc` | **协议聚合层**：把各协议 crate 挂到稳定的 `abs_kb_svc::v1::desktop::*` 路径下，**自己不定类型** | ✅ |
| `abs_kb_svc_v1_desktop` | **协议 v1 桌面端**：`kb_admin_desktop` × `kb_core` 的数据（15 请求 / 11 应答 / 9 事件）、按业务域拆分的异步 RPC trait、应用层握手。**协议内容的唯一出处** | ✅ 首批域 |
| `abs_kb_core_handshake` | **系统层握手**：`kb_core` 用 `--handshake-prompt=stdio` 公布的 `IpcReadyNotice`（启动方据此找到 IPC 端点） | ✅ |
| `kb_core_starter` | 启动 `kb_core` 子进程并**异步**等它公布 IPC 端点文件名的可取消 future；rproxy 与客户端共用 | ✅ |
| `kb_svc_servo_ipc` | 上面那套协议的**传输实现**：ipc-channel 的引导、三通道连接、客户端代理、服务端派发 | ✅ 可跑 |
| `abs_llm` | LLM 的**语义抽象**：对话角色、增量输出、用量、能力集等与 provider 无关的词汇 | ✅ |
| `kb_svc_salvo` | 第一版基于 Salvo + HTTP/WebSocket 的实现 | ❌ **已废弃**，待删（仅作历史资料） |

### `kb_plugins/crates/` —— 插件与配套进程

| crate | 职责 | 状态 |
| :--- | :--- | :--- |
| `kb_rig_llm_v1_agent` | 用 `rig` 直连 LLM 服务商，自己保留完整对话上下文 | 🚧 |
| `kb_rig_llm_v1_adapt` | rig 的原始数据 → `abs_llm::v1` 的转换 | 🚧 |
| `kb_core_rproxy` | **跨机网关**：启动一个 `kb_core`，自己监听 TCP，把远程访问者当作"格式与 ipc 客户端相同"的客户端转发 | ✅ 可跑（**无鉴权、无 TLS**，仅用于受信网络） |

### `kb_clients/` —— 客户端

| 目录 | 职责 | 状态 |
| :--- | :--- | :--- |
| `kb_admin_desktop` | Flutter 桌面客户端，含 flutter_rust_bridge 的 Rust 侧 | 🚧 界面骨架 |

## 3. 一次请求怎么走

```text
kb_admin_desktop / 远程客户端
      │ ① 系统层握手：找到端点（本机看 IPC 端点文件；跨机连 rproxy 的 TCP 端口）
      │    消息 = abs_kb_core_handshake 的 IpcReadyNotice
      │ ② 应用层握手：Request::Hello → Reply::Hello
      │    消息 = abs_kb_svc_v1_desktop 的协议（经 abs_kb_svc 聚合）
      ▼
  kb_core_rproxy（可选：只有跨机时才需要）
      │ TCP 帧 = [u32 长度][种类][postcard]，上行经有界环形缓冲做背压
      ▼
  kb_svc_servo_ipc::Client ──IPC──► kb_svc_servo_ipc::Listener
                                          │
                                  kb_core::ipc_::KbService   ← 按域 RPC 的服务端实现
                                          │
                                  kb_core::store_::Store     ← 本地文件（将来换 Turso）
                                          │
      ◄──────── ReplyEnvelope ────────────┘
```

- **协议**（数据、trait、握手语义）只在协议 crate 里定义——
  `abs_kb_svc_v1_desktop`（应用层与业务）与 `abs_kb_core_handshake`（系统层）；
  `abs_kb_svc` 只是把它们聚合到稳定路径下。换传输只换实现 crate。
- 两条链路的细节：本机 IPC 见 `kb_svc_servo_ipc` 的 crate 文档，
  跨机见 `kb_core_rproxy` 的 README。

## 4. 当前状态

**已经跑通的**：本机 IPC 上的工作区 / 会话增删查改（`kb_core` 常驻服务客户端）、
应用层握手与版本校验、跨机网关的请求转发与上行背压。

**还没做的**：其余业务域（设置 / 目录浏览 / LLM 生成与事件流）、并发服务多客户端、
优雅退出、鉴权、以及把存储从本地文件换成 Turso。
每一项的来龙去脉见 `dev-notes/` 下的对应文档。

## 5. 构建环境

工具链由仓库根的 `rust-toolchain.toml` 锁定为 **nightly**，有两个原因：

- `abs_llm` 使用了 `#![feature(try_trait_v2)]`，stable 编译不过；
- 可取消 future 的宏 `gen_mcf2` 要求 `#![feature(impl_trait_in_assoc_type)]`。

`rustup` 会按该文件自动准备工具链与 `rustfmt` / `clippy`，无需手工 `rustup default`。

客户端 `kb_admin_desktop` 里的 flutter_rust_bridge 子项目由 Cargokit 驱动，
它不读 `rust-toolchain.toml`（`rustup run` 会覆盖工具链文件），因此另有一份
`kb_clients/kb_admin_desktop/rust/cargokit.yaml` 把工具链对齐到同一条通道。

> 本机沙箱的 `~/.cargo` 是只读的；在这台机器上构建要
> `CARGO_HOME="$PWD/external/cargo-home" cargo test --workspace`
> （正常开发机不需要），详见 `dev-notes/llm_kb-20260917-1655.md` §4。

## 6. 文档在哪

- `kb_svc/crates/abs_kb_svc_v1_desktop/README.md`：协议 v1 的定位、契约与接口形状；
  数据见 `abs_kb_svc_v1_desktop/src/`。
- `kb_svc/crates/abs_kb_core_handshake/README.md`：系统层握手（`IpcReadyNotice`）。
- `kb_svc/crates/abs_kb_svc/README.md`：聚合层的用途与"什么时候不必经过它"。
- `kb_svc/crates/kb_core/README.md`：主进程的操作手册（命令行、存储布局、实测命令）。
- `kb_plugins/crates/kb_core_rproxy/README.md`：跨机网关的用法与能力边界。
- `kb_svc/crates/kb_core_starter/README.md`：启动 `kb_core` 并等 IPC 端点的那个 crate。
- `dev-notes/`：技术决策的前因后果与开放事项，索引见
  [`dev-notes/llm_kb-20260917-1655.md`](dev-notes/llm_kb-20260917-1655.md)。
