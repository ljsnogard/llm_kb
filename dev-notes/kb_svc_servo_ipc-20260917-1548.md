# kb_svc_servo_ipc 的落地方案

- 日期：2026-09-17 15:48（16:2x 更新：记录团队决定、trait 形状验证与首批落地）
- 状态：
  - **§1 已实测确定**（spike，可复现）；
  - **§2 两项决定已由团队拍板**：分层走 **方案 B**（先在 `abs_kb_svc` 立全量按域 trait），
    事件通道**第一步就开**；
  - **§3 的 trait 签名已复核通过并落地**：`TrKbEndpoint` / `RpcError` /
    `TrWorkspaceService` / `TrSessionService` / `TrKbService` 已在
    `kb_svc/crates/abs_kb_svc_v1_desktop/src/rpc_.rs`；
    §3.4 的其余域（设置 / 目录 / 握手 / 生成 / 事件）仍待补；
  - **§3 的落地顺序已走完第 3、4、5 步**：`kb_svc_servo_ipc` 已建，
    `kb_core` 已接上并在真实存储上跑通一轮增删查改；
  - **§5.4 / §5.5 记录了后续两次纠正**：取消要一路贯到 `store_`（不再有裸
    `async fn`），端点文件名改回「日期 + UUID」且由 `kb_core` 决定。
- 落地记录见 §5。
- 前置文档：
  - [`abs_kb_svc-20260917-1254.md`](abs_kb_svc-20260917-1254.md)：ipc-channel 选型、
    异步 RPC 设计、待决策项清单；
  - [`kb_core-20260917-1527.md`](kb_core-20260917-1527.md)：已完成的地基
    （compio + 本地文件存储 + 临时 CRUD 命令行）；
  - `kb_svc/crates/abs_kb_svc_v1_desktop/README.md` §5（实现方契约）与 §10（尚未确定的事项）。

---

## 0. 三个未知数的实测结论

落地前有三个未知数会直接决定公开 API 的形状。它们都先用 spike 验证掉，
再动手写接口。

| 未知数 | 为什么它决定 API | 结论 |
| :--- | :--- | :--- |
| one-shot server 只能接受一次连接，`kb_core` 怎么服务第 2、3…N 个客户端？ | 决定"引导/会合"要不要、以什么形状出现在公开 API 里 | ✅ **每接受一个就重建 + 原子重发名字 + 客户端带重试**（顺序 3/3、并发 3/3，含 1 次与 2 次真实重试） |
| `IpcReceiver::to_stream()` 能不能在 **compio** 上安全消费？ | 决定连接抽象用 `Stream`（运行时无关）还是自建"IO 线程 + Waker 完成量" | ✅ 可以：等待 300 ms 期间 compio 定时器走 **29** 步；内联 `recv()` 是 **0** 步 |
| `gen_mcf2` 能不能直接写进 trait / impl？ | 决定按域 trait 的形状（GAT 手写还是宏展开） | ❌ **不能**：宏只作用于模块级自由函数。⇒ trait 必须**手写 GAT**，宏生成的类型用来填那个关联类型。该形状已实测可编译可运行 |

### 0.1 这些数字是怎么来的（本机一次性实验，脚本未入库）

> **不要照着找脚本**。这一轮的 spike 是一个**一次性**的临时工程
> （当时放在 `external/ipc-channel-poc/`，不属于 workspace，而且整个 `external/`
> 现在已被 `.gitignore` 忽略）。它验证的是**上游库的行为 + 我们的设计选择**，
> 不是本仓库的代码，因此没有随仓库发布——这里保留的是**证据**，不是可复现流程。
>
> 为什么不把它留下来：它是那套机制的一份**拷贝**（自己的请求类型、自己的客户端），
> 在真 crate 漂移之后**仍然能跑绿**——一份过期的"绿"比没有更糟。

当时记下的原始输出（那一版的端点名字文件还是固定名，后来改成了「日期 + UUID」）：

```text
COMPIO-STREAM   value=1 elapsed=300.940228ms ticker_ticks_during_wait=29
COMPIO-BLOCKING value=1 elapsed=300.740158ms ticker_ticks_during_wait=0
SERVER-DONE clients=3
CLIENT-DONE pid=40 round_trips=2 elapsed=1.744873ms last=Some("1/echo:第 1 问")
CLIENT 第 1 次重试后连上
CLIENT-DONE pid=41 round_trips=2 elapsed=1.431789ms last=Some("1/echo:第 1 问")
CLIENT 第 2 次重试后连上
CLIENT-DONE pid=42 round_trips=2 elapsed=1.373064ms last=Some("1/echo:第 1 问")
```

两组客户端的差别是刻意的：顺序那组只证明"能接连服务"，**并发那组才是重点**——
3 个客户端同时抢同一个已公布的端点，只有一个能连上，另外两个必须走重试路径，
实测能收敛（分别重试 1 次与 2 次）。

trait 形状那一组（`gen_mcf2` + 手写 GAT）当时的输出：

```text
PLAIN      names=["笔记", "资料"]
CANCELLABLE names=["笔记", "资料"]
CANCELLED  outcome=Err(SpikeError)
TRAIT-SPIKE-DONE
```

它证明了三件事：trait 里可以声明受 `TrMayCancel` 约束的 GAT；
`gen_mcf2` 生成的类型可以当作那个 GAT 的实现；调用方既能 `.await`，
也能 `.may_cancel_with(token).await`。

### 0.2 这些结论现在是靠什么守着的

结论本身已经是代码与文档的一部分，**不依赖上面那个实验**：

| 结论 | 落位（都被跟踪） | 守着它的测试 |
| :--- | :--- | :--- |
| 引导 = 重建端点 + 重发名字 + 客户端重试 | `kb_svc_servo_ipc/src/{rendezvous_,listener_}.rs`、`src/lib.rs` 的模块文档 | `kb_svc_servo_ipc/tests/round_trip.rs::client_retries_until_the_server_publishes_`、`::a_second_client_is_served_after_the_first_disconnects_` |
| 应用层握手必须真的走一遍（含版本校验） | `abs_kb_svc_v1_desktop/src/handshake_.rs`、`kb_core/src/ipc_.rs` | `kb_svc_servo_ipc/tests/round_trip.rs::application_handshake_round_trips_and_rejects_version_mismatch_` |
| `gen_mcf2` 不能写进 trait ⇒ 手写 GAT | `abs_kb_svc_v1_desktop/src/rpc_.rs` 的模块文档 | `abs_kb_svc_v1_desktop/tests/rpc_contract.rs`（用同一套写法，形状不对就编译不过） |
| 阻塞不得进入异步执行器 | `abs_kb_svc_v1_desktop/README.md` §5 第 1、2 条 | **没有**直接回归测试，见下 |

> "执行器不被饿死"这一条目前只靠**文档与约定**：当时那个"等待期间跑计时器、
> 数它走了多少步"的做法完全可以写成一条常驻回归测试，但**尚未做**。
> 想补的话，它属于 `kb_svc_servo_ipc` 的测试，而不是留在 spike 里。

## 1. 已经确定、不必再讨论的部分

### 1.1 引导机制：重发端点名 + 客户端重试

```text
kb_core 侧（accept 是阻塞调用 → 放进 compio 的阻塞线程池）:
  启动时: 生成 <runtime-dir>/kb-<YYYYMMDD>-<uuid>.ipc，并清掉上次残留的 kb-*.ipc
  循环 {
      ① IpcOneShotServer::new()  →  得到一个新端点名（/tmp/.tmpXXXXXX/socket）
      ② 原子写进上面那个名字文件（先写 .tmp 再 rename）
      ③ accept()  ← 阻塞
      ④ 拿到客户端的 (请求接收端, 应答发送端, 事件发送端) → 交给处理逻辑
      ⑤ 把名字文件内容清空（这个端点已经用掉了）
  }

客户端侧:
  循环 {
      在 <runtime-dir> 里找内容非空的 kb-*.ipc
      connect 成功就发引导消息；失败（端点被抢了 / 正在换下一个）
      就睡 20 ms 再来，直到超时
  }
```

- **文件名由 `kb_core` 决定**，沿用旧 `plugin_socket` 的「日期 + UUID」约定：
  每次启动独一无二，历史残留不会挡路。文件名的语义是"服务端实例"，
  **内容**才是"当前可连的端点名"。
- ipc-channel 自己生成的 socket 仍在 `/tmp/.tmpXXXXXX/`（父目录 0700），
  我们只负责把端点名转达出去——`IpcOneShotServer` 没有提供指定 socket 路径的接口
  （见 §5.5）。
- 发布必须原子（`.tmp` + `rename`），与 `kb_core` 存储层同一个手法。
- **`connect` 失败不是错误**，只是"再等等"；重试要有上限。
- 并发抢同一个端点时只有一个能连上，其余快速失败并重试（实测无挂起）。

### 1.2 compio 侧：连接内不得内联阻塞

`compio_runtime::Runtime` 是 **thread-local** 的（其文档原话："It is a thread-local
runtime, meaning it cannot be sent to other threads"）。一次内联 `recv()` 就是把该
运行时的执行线程整个占住——实测 300 ms 内 compio 定时任务走了 **0** 步。

于是只有两种合法写法：

1. **`IpcReceiver::to_stream()`**（推荐）：ipc-channel 内部有进程级 router 线程阻塞在
   `IpcReceiverSet::select()` 上，把消息转给 `futures_channel::mpsc`；
   `IpcStream: futures_core::Stream`，与运行时无关，实测在 compio 上正常。
2. 自建「专用 IO 线程 + Waker 完成量」：`abs_kb_svc` README §4.2 的方案 E，
   留作需要 per-endpoint 精细控制（多路复用、取消传播）时的后路。

**先按 1 落地，把 2 写进模块文档作为后备**。

### 1.3 一条连接上的通道形状

`envelope_.rs` 已写死这条约定：一对通道（一个上行、一个下行）足够，
`RequestEnvelope::request_id` 负责把应答配回请求；事件不需要信封。

引导消息（客户端 → `kb_core` 的第一条消息）就是**端点三元组**：

```rust
(
    IpcReceiver<RequestEnvelope>,  // 上行：请求
    IpcSender<ReplyEnvelope>,      // 下行：应答（用 request_id 配回）
    IpcSender<Event>,              // 下行：服务端主动推送（Ready / Delta / SessionChanged…）
)
```

好处：请求可在服务端并发处理、应答乱序不串；事件不会被大应答堵住；
将来改成"每请求一对通道"时引导消息形状不用动。

> 注意 ipc-channel 的 `send` 不阻塞、通道无界（`abs_kb_svc-20260917-1254.md` §3.5），
> **背压必须在语义层自己做**（README §5 第 6 条）。

---

## 2. 已拍板的两项决定

| 问题 | 结论 |
| :--- | :--- |
| `kb_core` 与 `kb_svc_servo_ipc` 的耦合方式 | **方案 B**：先在 `abs_kb_svc` 定义按业务域拆分的异步 RPC trait，再由 `kb_svc_servo_ipc` 落地实现；`kb_core` 面向 trait 编程 |
| 引导消息是否带事件通道 | **现在就开**（三通道） |

方案 B 的一个连带影响需要记下来：`abs_kb_svc` 的 trait **只依赖 `abs_cancel`，
不依赖 `gen_mcf2`**（宏只能作用于模块级自由函数，trait 里用不了；trait 只需手写
GAT 并约束到 `TrMayCancel`）。因此：

- `kb_core` 的依赖链上仍然**没有** `gen_mcf2` —— `AGENTS.md` 第 4 条对
  `kb_core::store_` 那些普通 `async fn` 的例外依然成立，它们不必改成可取消 future；
- `gen_mcf2` 只出现在**写实现**的 crate 里（`kb_svc_servo_ipc` 的客户端代理、
  以及 `kb_core` 服务端那侧的 trait 实现）。若那两处用到宏，则这两个 crate 自己
  要加 `#![feature(impl_trait_in_assoc_type)]`。

---

## 3. 已复核的 trait 签名（§3.1–§3.3 已落地）

这是方案 B 的"全量按域 trait"。§3.1–§3.3 已按下面这些签名落到
`kb_svc/crates/abs_kb_svc_v1_desktop/src/rpc_.rs` 并从 `desktop/mod.rs`
逐个导出（不用通配符）；§3.4 的其余域待补。

> 落进代码时 `rustfmt` 会把下面那些 GAT 写法的换行收成一行（它认为折行更差），
> 因此**以代码为准**，本文件保留可读的折行形式。

### 3.1 公共基底与错误分层

每个 trait 都要能表达"实现方的传输层错误"，所以先有一个基底 trait；
业务错误则复用协议里已有的 `ErrorReply`（`error_.rs`）。

```rust
/// 所有按域 RPC trait 的公共基底。
///
/// 关联类型 `Error` 是**实现方**的错误类型（客户端代理是 ipc-channel 的传输错误，
/// 服务端逻辑是它自己的内部错误）。
pub trait TrKbEndpoint {
    /// 实现方的错误类型。
    type Error: core::error::Error + Send + Sync + 'static;
}

/// 一次 RPC 调用的失败。
///
/// 刻意把两类失败分成两个**变体**（`abs_kb_svc_v1_desktop/README.md` §5 第 7 条：
/// 传输层错误与业务错误不得混在一个类型里），这样调用方一眼能看出
/// "该重试还是该提示用户"。
#[derive(Debug)]
pub enum RpcError<E> {
    /// 服务端明确答复"不行"：这是正常应答的一个分支。
    Business(ErrorReply),

    /// 连接、编解码、对端消失……由实现方定义。
    Transport(E),
}
```

### 3.2 工作区

```rust
pub trait TrWorkspaceService: TrKbEndpoint {
    type ListWorkspaces<'f>: TrMayCancel<
        'f,
        MayCancelOutput = Result<WorkspaceList, RpcError<Self::Error>>,
    >
    where
        Self: 'f;

    type AddWorkspace<'f>: TrMayCancel<
        'f,
        MayCancelOutput = Result<Workspace, RpcError<Self::Error>>,
    >
    where
        Self: 'f;

    type RemoveWorkspace<'f>: TrMayCancel<
        'f,
        MayCancelOutput = Result<(), RpcError<Self::Error>>,
    >
    where
        Self: 'f;

    fn list_workspaces<'f>(&'f self) -> Self::ListWorkspaces<'f>;
    fn add_workspace<'f>(&'f self, request: AddWorkspaceRequest) -> Self::AddWorkspace<'f>;
    fn remove_workspace<'f>(&'f self, workspace_id: WorkspaceId) -> Self::RemoveWorkspace<'f>;
}
```

### 3.3 会话

```rust
pub trait TrSessionService: TrKbEndpoint {
    type ListSessions<'f>: TrMayCancel<'f, MayCancelOutput = Result<SessionList, RpcError<Self::Error>>> where Self: 'f;
    type CreateSession<'f>: TrMayCancel<'f, MayCancelOutput = Result<SessionSummary, RpcError<Self::Error>>> where Self: 'f;
    type GetSession<'f>: TrMayCancel<'f, MayCancelOutput = Result<SessionDetail, RpcError<Self::Error>>> where Self: 'f;
    type RemoveSession<'f>: TrMayCancel<'f, MayCancelOutput = Result<(), RpcError<Self::Error>>> where Self: 'f;

    fn list_sessions<'f>(&'f self, workspace_id: WorkspaceId) -> Self::ListSessions<'f>;
    fn create_session<'f>(&'f self, request: CreateSessionRequest) -> Self::CreateSession<'f>;
    fn get_session<'f>(&'f self, workspace_id: WorkspaceId, session_id: SessionId) -> Self::GetSession<'f>;
    fn remove_session<'f>(&'f self, workspace_id: WorkspaceId, session_id: SessionId) -> Self::RemoveSession<'f>;
}
```

### 3.4 其余按域 trait（第一步先不实现，签名一并定下来）

| trait | 方法 | 对应协议 |
| :--- | :--- | :--- |
| `TrHandshake` | `hello(client: ClientInfo) -> ServerInfo` | `Request::Hello` / `Reply::Hello` |
| `TrSettingsService` | `list_services() -> ServiceList`、`upsert_service(UpsertServiceRequest) -> ServiceSummary`、`remove_service(ServiceId) -> ()`、`use_service(ServiceId) -> ()` | `ListServices` / `UpsertService` / `RemoveService` / `UseService` |
| `TrDirectoryService` | `list_directory(workspace_id, relative_path: String) -> DirectoryListing` | `ListDirectory` |
| `TrGeneration` | `ask(AskRequest) -> 事件流`、`cancel(TurnId) -> ()` | `Ask` / `Cancel` |
| `TrEventSource` | `subscribe() -> 事件流`（`abs_async_iter::TrFlux`） | `Event` |
| `TrKbService` | `TrKbEndpoint + TrWorkspaceService + TrSessionService + TrSettingsService + TrDirectoryService + TrGeneration + TrEventSource` 的组合 trait | — |

**一条统一的映射规则**（写进模块文档，让派发层没有解释空间）：

> trait 方法**收**协议里的请求载荷类型（`AddWorkspaceRequest` / `CreateSessionRequest`
> / `AskRequest`…），**回**协议里的应答载荷类型（`Workspace` / `SessionSummary` /
> `SessionDetail` / `WorkspaceList` / `SessionList`），并省略纯客户端的簿记字段：
> - `Reply::WorkspaceAdded { local_id, workspace }` → 返回 `Workspace`
>   （`local_id` 是调用方自己填的，不必往返）；
> - `Reply::Ack` → 返回 `()`；
> - `Reply::Error(ErrorReply)` → 变成 [`RpcError::Business`]；
> - 编码/连接/对端消失 → [`RpcError::Transport`]。

**事件流**的形状（`ask` 与 `subscribe` 共用）需要 `abs_async_iter::TrFlux`，
其确切签名在实现那一步再定（它同样要满足"每项可取消"）。

---

## 4. 建议的 crate 形状（草案，尚未实施）

```text
kb_plugins/crates/kb_svc_servo_ipc/
├── Cargo.toml          # ipc-channel 0.23 (async) + abs_kb_svc；不依赖 tokio / compio
└── src/
    ├── lib.rs          # 模块地图与公开导出（逐个列举）
    ├── error_.rs       # 传输层错误 → 填进 abs_kb_svc 的 RpcError::Transport
    ├── rendezvous_.rs  # 名字文件的原子发布与读取、客户端重试
    ├── listener_.rs    # 接受循环：重建 one-shot server → 公布 → accept
    └── connection_.rs  # 一条连接的收发端点（请求流 / 应答与事件发送端）
```

依赖方向：`kb_svc_servo_ipc` → `abs_kb_svc` + `ipc-channel`；**反向禁止**。

---

## 5. 落地顺序与进展

1. ~~复核 §3 的签名~~ → ✅ 已按 §3 原样通过；
2. ~~把 §3.1–§3.3 的 trait 落到 `abs_kb_svc`~~ → ✅ **已完成**，见 §5.1；
3. ~~新建 `kb_svc_servo_ipc`：引导 + 连接 + 客户端代理 + 服务端派发~~ → ✅ **已完成**，见 §5.3；
4. ~~在 `kb_core` 里实现这七条工作区/会话方法~~ → ✅ **已完成**，见 §5.3；
5. ~~加集成测试：真起服务端 + 客户端走 IPC 跑完一轮增删查改~~ → ✅ **已完成**（放在 `kb_core::ipc_` 的单元测试里，因为 `kb_core` 是 bin crate，`tests/` 拿不到它的内部模块）；
6. 决定 `kb_core/src/cli_.rs` 那组临时子命令的去留。

### 5.1 第 2 步实际落成的东西

| 位置 | 内容 |
| :--- | :--- |
| `abs_kb_svc_v1_desktop/src/rpc_.rs` | `TrKbEndpoint`、`RpcError<E>`、`TrWorkspaceService`（3 个方法）、`TrSessionService`（4 个方法） |
| `abs_kb_svc_v1_desktop/src/error_.rs` | 给 `ErrorReply` 补 `Display` + `core::error::Error`（`RpcError::Business` 需要一个错误类型） |
| `abs_kb_svc_v1_desktop/src/lib.rs` | 新增 `mod rpc_;`，逐个导出四个名字 |
| `abs_kb_svc_v1_desktop/tests/rpc_contract.rs` | 一份用 `gen_mcf2` 展开的 mock 实现 + 6 项契约测试 |
| `abs_kb_svc/Cargo.toml` | 直接依赖 `abs_cancel`；dev 依赖 `gen_mcf2`（**库不依赖它**）、`futures-lite` |

顺带修掉一个**既有的隐性构建缺陷**：`abs_kb_svc` 的 `serde` 只开了 `derive`
（`default-features = false`），而协议类型里全是 `String` / `Vec<T>` / `Option<T>`，
它们需要 serde 的 `alloc` 才有 `Serialize` / `Deserialize`。此前它只在
workspace 整体构建时靠 `kb_core` 的 `serde_json` 把 `std` 打开而"碰巧"能编译，
单独 `cargo check -p abs_kb_svc` 会失败。现在补上 `alloc`，并把这句"刻意不开
std/alloc"的旧注释改正。

验证：`cargo test -p abs_kb_svc` ⇒ 23 单元 + **6 集成** + 10 文档测试全绿；
`cargo clippy --workspace --all-targets` 无告警。

mock 的样板价值：实现方（`kb_svc_servo_ipc` 的客户端代理、`kb_core` 的服务端逻辑）
可以照抄这个文件的结构——模块级 `async fn` + `#[gen_may_cancel_future(Xxx)]`，
然后在 `impl` 里 `type Xxx<'f> = XxxAsync<'f, 'f>;`。
**实现 crate 需要 `#![feature(impl_trait_in_assoc_type)]`。**

### 5.2 仍未闭合的一点

取消的落点（§3 约定 3）：`RpcError` 目前只有 `Business` / `Transport` 两个变体，
"用户主动取消"与"连接断了"因此都落在 `Transport` 里。契约测试
`cancellation_lands_on_transport_error_` 把这条约定钉住了，但**如果界面将来需要
区分这两者，就要给 `RpcError` 加第三个变体**——那又是一次公开 API 变更。

### 5.3 第 3–5 步实际落成的东西

`kb_plugins/crates/kb_svc_servo_ipc/`：

| 文件 | 内容 |
| :--- | :--- |
| `src/rendezvous_.rs` | 名字文件 `<runtime-dir>/kb-<日期>-<uuid>.ipc` 的生成、清理、原子发布/清空/读取，以及客户端的重试连接 |
| `src/listener_.rs` | `Listener`：`bind` 时清掉残留名字；`accept` 阻塞地等服务端下一个客户端，接受后立刻撤下名字 |
| `src/connection_.rs` | `Connection`：三通道端点 + `serve(&service)` + 请求派发（`Request` → trait → `Reply`） |
| `src/client_.rs` | `Client`：实现 `TrWorkspaceService` / `TrSessionService` 的代理，含专用应答路由线程 |
| `src/error_.rs` | `ServoIpcError`（传输层错误，填进 `RpcError::Transport`） |

`kb_svc/crates/kb_core/`：

| 文件 | 内容 |
| :--- | :--- |
| `src/ipc_.rs` | `KbService`：trait 的服务端实现，转发给 `Store`，并把 `StoreError` 翻成 `ErrorCode`（`NotFound` / `BadRequest` / `Internal`） |
| `src/error_.rs` | `CoreError`：把存储失败与 IPC 失败收在一处 |
| `src/serve_.rs` | 改成真正的服务循环：`Listener::bind` → `spawn_blocking(accept)` → `connection.serve(&service)` |
| `src/main.rs` | 打开 `#![feature(impl_trait_in_assoc_type)]`、挂上新模块、`dispatch_` 改用 `CoreError` |

**错误分层的落点**（这是本 crate 与 `abs_kb_svc` README §5 第 7 条对齐的地方）：

| 情形 | 服务端返回 | 客户端拿到 |
| :--- | :--- | :--- |
| 存储说"目标不存在" | `Reply::Error(NotFound)` | `RpcError::Business(ErrorReply { NotFound })` |
| 存储说"标识不合法" | `Reply::Error(BadRequest)` | `RpcError::Business(..)` |
| 存储 I/O 失败 | `Reply::Error(Internal)` | `RpcError::Business(..)` |
| 客户端等应答时连接断了 | —（服务端已消失） | `RpcError::Transport(ServoIpcError::PeerClosed)` |
| 调用方用取消令牌中止 | —（请求仍会被处理） | `RpcError::Transport(ServoIpcError::Cancelled)` |

`KbService` 的 `TrKbEndpoint::Error` 是 `Infallible`——服务端这一侧不产生传输层
错误，所有失败都是业务错误；派发层里 `RpcError::Transport` 那条分支对本实现不可达。

**验证**（`CARGO_HOME` 需指向仓库内的可写缓存，见 [`llm_kb-20260917-1655.md`](llm_kb-20260917-1655.md) §4）：

```bash
CARGO_HOME="$PWD/external/cargo-home" cargo test --workspace   # 140 项全绿
CARGO_HOME="$PWD/external/cargo-home" cargo clippy --workspace --all-targets
```

其中两个关键用例：

- `kb_svc_servo_ipc::tests/round_trip.rs`（6 项）：纯 IPC 层——三通道往返、
  业务错误穿线、客户端重试、第二个客户端、取消、超时；
- `kb_core::ipc_::tests_::ipc_round_trip_reaches_the_local_store_`：
  **真存储 + 真 IPC** 的端到端——在独立线程起 `Listener` + `KbService`（底层是
  真实目录上的 `Store`），客户端跑完一轮增删查改，并回到存储目录核对 JSON 文件
  确实写出来了、删工作区确实级联删掉了会话目录。

### 5.4 取消的处理方式（曾被误判，已纠正）

`kb_core` 引入 `gen_mcf2` 之后，`AGENTS.md` 第 4 条自然落到这个 crate 上。
**最初的处理是错的**：当时以"`kb_core` 是 bin、没有对外库接口"为由，只给
`KbService` 的方法加了取消，把 `store_::Store` 的普通 `async fn` 留下了。

团队纠正了这条判断，理由值得完整记下来：

> 即使是内部的 async 方法，只要它有被取消的可能性，就应该使用 `gen_mcf2`。
> 没有被取消可能性的，一般只有直接返回一个结果的；即使是调用非本项目范围内的
> 异步函数，一样有实现取消的方法，我们不能从设计上就抹杀了这种需求。

现在的做法（`kb_core/src/store_/mod.rs`）：

- `Store` 的**每个操作**都由 `gen_may_cancel_future` 展开出一对类型：
  `XxxAsync`（`IntoFuture`）与 `XxxFuture`（`.may_cancel_with(token)`）。
  `Store` 上不再有裸 `async fn`；
- 取消的落点是 `race_cancel_`：**整个操作体**与取消信号赛跑，令牌先触发就返回
  `StoreError::Cancelled`，尚未完成的等待随 future 被丢弃；
- 私有辅助（`read_json_` / `write_json_` / `create_dir_all_` …）**不再各自包一层
  宏**：它们只在已经被令牌包住的操作体里被调用，等待会随外层 future 一起被丢弃，
  没有第二个调用者需要独立的 future 类型。它们没有假定"调用者不会取消"，
  只是把这件事交给唯一的外层统一处理；
- `KbService` 把 IPC 请求带的令牌**继续传给** `Store`（`.may_cancel_with(cancel)`），
  而不是丢掉它自己造一个新的。
- 守住这条的测试是 `store_::tests_::store_operations_honor_cancellation_`：
  用 `CancelledToken` 调 `add_workspace` 会拿到 `StoreError::Cancelled`，
  **并且磁盘上不会留下那个工作区**。

### 5.5 端点文件名：由 `kb_core` 决定，保留「日期 + UUID」

同一轮里纠正的第二件事：端点名字文件原来是固定的 `kb-core.ipc`，
这既丢掉了旧 `plugin_socket` 的命名约定，也把"叫什么名字"默认让给了传输库。

现在：`Listener::bind` 生成 `<runtime-dir>/kb-<YYYYMMDD>-<uuid-v4>.ipc`
（见 `rendezvous_::new_name_file_in`，日期算法直接沿用 `plugin_socket`），
客户端在运行时目录里找内容非空的 `kb-*.ipc`。
**文件名由 `kb_core` 决定**，与传输实现无关——换掉 ipc-channel 也保留这条约定，
因为"本机 IPC"这件事本身是 `kb_core` 的需求，而不是某个库的。

一条需要写清楚的边界：`ipc-channel` 的 `IpcOneShotServer` 把自己绑在
`tempdir()/socket` 上，**没有**提供指定路径的接口（`OsIpcOneShotServer::new()`
里写死，`OsIpcReceiver::from_fd` 是私有的，`IpcReceiver` 也没有公开构造函数）。
所以"操作系统层面的 socket 路径"目前仍由它决定；由 `kb_core` 决定并对外公布的
是那个名字文件。若将来要让 socket 本身也落在我们指定的路径上，就得自己建监听
socket（`libc` + `SOCK_SEQPACKET`）而不再用 `IpcOneShotServer`——那是一次独立决策。

## 6. 仍然没有答案的问题

- 跨机客户端是否显式排除在近期范围外（决定要不要预留网络实现）；
- `buffex` / `mm_ptr` 在 `abs_kb_svc` 里的定位（保留 / 替换 / 下沉）；
- `AddWorkspace` 的重复投递按 `LocalId` 幂等由谁保证；
- "同一会话同时只允许一轮进行中"的背压语义由谁执行。
