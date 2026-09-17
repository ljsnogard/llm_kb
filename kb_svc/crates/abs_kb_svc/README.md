# abs_kb_svc

知识库服务的 **业务通信抽象层**：定义各插件与 `kb_core` 通信的抽象代码接口、通信内容，
以及一个**与异步运行时无关的异步 RPC 业务接口**。

> **当前状态：设计提案 + 首批数据定义已落地。**
> [`src/v1/desktop/`](src/v1/desktop/mod.rs) 已经给出桌面客户端与 `kb_core` 之间的
> 通信数据（15 个请求 / 11 个应答 / 9 个事件），并有单元测试与文档测试覆盖。
> 抽象的异步 RPC trait 与插件侧数据（`v1::plugin`）**尚未落地**，
> 其中的公开 API 属于**对外约定**，按 `AGENTS.md` 第 1 条须经团队确认后才能落到代码。
>
> 背景、证据与待决策项见：
> - [`dev-notes/abs_kb_svc-20260917-1254.md`](../../../dev-notes/abs_kb_svc-20260917-1254.md)（定位与选型决策）
> - [`dev-notes/kb_admin_desktop-20260917-1341.md`](../../../dev-notes/kb_admin_desktop-20260917-1341.md)（客户端通信需求调查）

---

## 1. 它是什么 / 不是什么

| | 内容 |
| :--- | :--- |
| **是** | ① **通信内容**——跨进程通信的业务数据语义（谁说、说什么、字段含义）；<br>② **信道底层抽象**（收发端点、连接、事件）；<br>③ **异步 RPC 业务接口**——把 `kb_core` 的能力抽象成若干带异步关联函数的 trait。 |
| **不是** | 不是任何具体传输的实现。它不依赖 ipc-channel、不依赖 socket、不依赖 tokio，也不定义"消息在线上长什么样"。 |

**一句话**：`abs_kb_svc` 规定"通信双方在**语义**上如何对话"，
把"这些对话在**物理上**如何搬运"留给实现 crate。

## 2. 为什么需要它

三条已确认的技术决策直接决定了本 crate 的存在理由
（详见 dev-notes 的 §2）：

1. **`kb_core` 是独立 server，暂不考虑迁移到移动端。**
   于是跨平台不再是选型的一级约束。
2. **将来若需要别的部署形态，通过替换 IPC 实现来适配，而不是死磕某一种传输。**
   要实现这一点，业务代码就不能直接依赖任何一种传输——这正是本 crate 的职责。
3. **尽可能简化技术链条。**
   业务层与传输层之间不再插入需要单独维护的镜像类型或编解码约定。

## 3. 分层与依赖方向

```text
   kb_core                 kb_plugins/*              kb_admin_desktop
  （服务进程）               （插件进程）                （客户端进程）
       │                         │                          │
       └────────────┬────────────┴──────────────┬───────────┘
                    │   只依赖「业务语义 + 异步 RPC 接口」
              ┌─────▼──────┐
              │ abs_kb_svc │                        ← 必选
              └─────┬──────┘
                    │   由实现 crate 落地
     ┌──────────────┼───────────────┬─────────────────┐
     │              │               │                 │
┌────▼─────┐  ┌─────▼──────┐  ┌─────▼──────┐  ┌───────▼───────┐
│ kb_svc_  │  │ kb_svc_    │  │ kb_svc_    │  │ （将来其它）   │  ← 可选，择一
│ servo_ipc│  │ socket     │  │ thread     │  │               │
│(ipc-chan)│  │            │  │(同进程线程)│  │               │
└──────────┘  └────────────┘  └────────────┘  └───────────────┘
```

要点：

- **箭头单向**：`abs_kb_svc` 不认识任何实现 crate，实现 crate 认识 `abs_kb_svc`；
- 子进程与客户端**只必选** `abs_kb_svc`，实现 crate 按需要的传输形态**可选**引入；
- 换传输 = 换一个实现 crate，业务代码不动。

## 4. 三块内容

### 4.1 通信内容（业务数据语义）

跨进程通信用到的数据类型的**语义定义**：一轮生成、一条增量、用量、能力集、
服务配置、工作区与会话……它们描述"业务上这是什么"。

**表示形式由实现需要决定**：本 crate 的数据类型带 `#[derive(Serialize, Deserialize)]`，
因为实现候选 `kb_svc_servo_ipc`（ipc-channel）的 typed 通道需要它。
这不改变类型的形状——没有定长限制、没有容量上限，`String` / `Vec` / `Option` 都能用。

已落地的部分：

| 模块 | 内容 |
| :--- | :--- |
| [`v1::desktop`](src/v1/desktop/mod.rs) | 桌面客户端 `kb_admin_desktop` × `kb_core`（`mod.rs` + 11 个子模块） |
| `v1::plugin` | 插件 × `kb_core`（**尚未定义**） |

### 4.2 信道底层抽象

对"通信端点"的最小抽象，供实现 crate 落地：

- 连接的建立与关闭；
- 单向/双向消息的收发；
- 对端消失、连接被关闭等事件的表达。

这层刻意做薄：**能表达语义即可**，不追求覆盖所有传输的能力（例如不在此处抽象"零拷贝"，
因为并非所有实现都提供它）。

### 4.3 异步 RPC 业务接口

把 `kb_core` 的能力抽象成**一个个带异步关联函数的 trait**，
按业务域拆分（例如会话、设置、工作区、插件管理、知识库检索）。

调用者（`kb_core` 自身、插件、桌面客户端）只面向这些 trait 编程；
不同模块提供不同实现（ipc-channel / socket / 同进程线程 / mock）。

**流式内容**（LLM 的增量输出）用 `abs_async_iter::{TrAsyncIterator, TrFlux}` 表达，
而不是把增量塞进一次请求/应答里——`v1::desktop` 的 `Event` 就是这条流的元素类型。

## 5. 实现方必须遵守的契约（硬性）

这一节是本 crate 最重要的部分：**它把"阻塞 IO 不得进入异步执行线程"从经验之谈变成接口契约。**

1. **future 里禁止阻塞。**
   任何由本 crate 的 trait 返回的 future，其 `poll` **不得**执行会阻塞 OS 线程的操作
   （`recv`、`select`、同步文件 IO、不可控的锁等待……）。
   阻塞只能发生在实现方自有的 **IO 线程**上。

   > 依据：本机实测显示，在异步任务里内联调用阻塞 `recv()`，会在 300 ms 等待期间
   > 让 `current_thread` 运行时的定时任务执行 **0 次**；改用"专用线程 + 完成量"后，
   > 同一场景执行 **26 次**。

2. **IO 线程模型由实现方决定，但端点不得交给调用者的执行器线程。**
   一个端点一条线程、还是一个线程用多路复用（如 `IpcReceiverSet`）服务多个端点，
   属于实现细节；但把阻塞端点直接暴露给调用方是禁止的。

3. **完成量必须运行时可无关。**
   把结果从 IO 线程交回异步侧所用的原语只能建立在 `core::task::Waker` /
   `futures_core` 层面，**不得**出现 tokio / async-std / compio 的私有类型。
   本 crate 的公开 API 里也不得出现任何具体运行时的类型。

4. **唤醒不得丢失。**
   "IO 线程先完成、异步侧后注册 `Waker`"这一时序必须被正确处理；
   这是该模式最常见的缺陷，表现为偶发卡死而非稳定报错。

5. **取消必须可达。**
   调用者丢弃 future 后，实现方要么向对端发出显式取消信号，要么依赖连接断开被对端感知；
   **不得**让对端永久阻塞在等待上。

6. **背压必须在语义层表达。**
   不要依赖传输层提供背压（例如 ipc-channel 的通道是无界的、`send` 永不阻塞）。
   "同一会话同时只允许一轮进行中"这类约束由本 crate 的语义定义，实现方只需忠实执行。

7. **错误要分层。**
   "传输层错误"（对端断开、编码失败）与"业务错误"（服务不存在、缺少 API Key）
   不得混在同一个类型里：前者走 impl 的错误类型，后者是正常应答的一个分支。
8. **协议类型的 serde 写法受传输约束。**
   实现候选用的 ipc-channel 内部是 **postcard**（不自描述），因此协议类型：
   枚举**一律用 serde 默认的外部标签**，**不得使用 `skip_serializing_if`**，
   也不要使用 `#[serde(flatten)]`。

   > 依据：实测这两个写法都只在**解码**时失败——内部标签报
   > "This is a feature that PostCard will never implement"，
   > `skip_serializing_if` 报 `DeserializeUnexpectedEnd`；而编码看起来是成功的。
   > `v1::desktop` 的 `envelope_.rs` 用 postcard **真解码**的往返测试守住这两条。

## 6. 接口形状（对齐仓库当前的异步约定）

本仓库的异步抽象已经在 2026-09-17 迁移到 **`abs_cancel` v0.2 + `gen_mcf2` + `abs_async_iter`**，
`abs_kb_svc` 沿用这套约定，以保证整个仓库的一致性：

| 约定 | 说明 |
| :--- | :--- |
| trait 名前缀 `Tr` | 例如 `TrMessage`、`TrLlmService` |
| **不用裸 `async fn`** | 用 GAT 关联类型 + `abs_cancel::TrMayCancel`（v0.2），避免 `dyn` 兼容性与 box 开销 |
| 具体 future 由 `gen_mcf2::gen_may_cancel_future` 生成 | 满足 `AGENTS.md` 第 4 条；同时得到 `Future` 与 `TrMayCancel` 两套实现 |
| 流式内容用 `abs_async_iter` | `TrFlux` 订阅、`TrAsyncIterator` 逐项拉取，每项都可取消 |
| 成功/失败用 `anylr::TrEitherOf` 或裸 `Result` | 与该模块既有写法保持一致 |
| 错误类型实现 `core::error::Error` | 便于在 `no_std` 语境下复用 |
| 字符串泛化用 `abs_str::TrStringView` | 仅在确实需要泛化时引入 |

**`gen_mcf2` 的用法**（摘自其宏文档，序号即约定）：

1. 只能作用于 `async fn`；
2. 显式声明所需生命周期，不要用 `__` 结尾的标识符（宏保留该后缀）；
3. 最后一个泛型**类型**参数是取消令牌类型，且 where 子句里约束 `TrCancellationToken`；
4. 全部约束写在 where 子句里，不要内联在泛型参数上；
5. 最后一个函数参数必须是该取消令牌类型，且**按值**传入。

**示意形状**（尚未落地，不保证编译）：

```rust
/// 一次会话（ask / cancel）的抽象接口。
///
/// `ask_async` 本身是同步函数，只负责构造 future；
/// 真正的等待发生在返回的 `AskAsync` 上，而它由 `gen_may_cancel_future` 生成。
pub trait TrSession {
    type Req;
    type Resp;
    type Err: core::error::Error;

    /// 发起一问；取消通过 `Cancel` 语义单独表达，见 §5 第 5 条。
    type AskAsync<'f>: abs_cancel::TrMayCancel<
            'f,
            MayCancelOutput = Result<Self::Resp, Self::Err>,
        >
    where
        Self: 'f;

    fn ask_async<'f>(&'f self, request: Self::Req) -> Self::AskAsync<'f>;
}
```

> 完整的 trait 划分（会话 / 设置 / 工作区 / 插件 / 检索各占几个 trait、粒度多粗）
> 属于待确认事项，见 dev-notes §5。

## 7. 已知的实现候选

| 实现 crate | 传输 | 状态 |
| :--- | :--- | :--- |
| `kb_svc_servo_ipc` | servo/ipc-channel 0.23（跨进程 channel；Unix 走 socketpair + fd 传递，macOS 走 Mach port，Windows 走命名管道） | **已选定**，尚未实现（可行性验证见 dev-notes §3） |
| `kb_svc_thread`（暂名） | 同进程线程 + 内存队列 | 备选；用于测试或"单进程内跑全部组件"的形态 |
| `kb_svc_socket`（暂名） | 保留 socket 形态 | 备选；应对未来跨机/跨平台需求 |

关于 ipc-channel 的实测结论、限制（一个 one-shot server 只接受一次连接、
通道无界、`to_stream()` 只对 typed 通道提供等）以及它与 iceoryx2 的取舍对比，
见 dev-notes §3。

## 8. 命名与文档约定

- 所有公开项必须有 `///` 文档注释，并按需包含 `# Examples` / `# Panics` / `# Errors`；
  正文以中文为主（`AGENTS.md` 第 8 条）。
- 测试函数（含 `tests/` 与文档测试）必须有中文文档注释，写明
  **测试目标 / 测试手段 / 判定标准**（`AGENTS.md` 第 2 条）。
- agent 编写的私有 struct 字段与私有函数名以 `_` 结尾（`AGENTS.md` 第 5 条）。
- 对外公开的异步方法优先用 `gen_may_cancel_future` 封装，且实现不得假定调用者不会取消
  （`AGENTS.md` 第 4 条）。

## 9. 与其它 crate 的关系

| crate | 关系 |
| :--- | :--- |
| `abs_llm` | 提供 LLM 语义类型与 trait；`abs_kb_svc` 的业务数据会引用其中的类型，但不重复定义 |
| `kb_svc_*`（实现 crate） | 依赖 `abs_kb_svc`；反向依赖禁止 |
| `kb_core` | 面向 `abs_kb_svc` 的 trait 编程；不直接依赖任何实现 crate 的传输类型 |
| `kb_plugins/*`、`kb_admin_desktop` | 同上 |
| `kb_svc_salvo` | **已废弃**（HTTP 通道整体删除）；其中的会话/设置语义将迁入新结构 |

### 现有依赖的遗留问题

`abs_kb_svc/Cargo.toml` 当前已经依赖 `buffex` 与 `mm_ptr` 两个自研 crate
（缓冲区交换与内存映射指针抽象）。它们与"信道抽象"在职责上存在重叠，
是保留、替换还是下沉**尚未决定**，见 dev-notes §5 决策项 5。
在结论明确之前，本 crate 不应继续扩大对这两个依赖的使用。

## 10. 尚未确定的事项

以下问题在本 crate 落地前必须明确（完整清单见 dev-notes §5）：

1. **trait 划分粒度**：一个大 trait 还是按业务域拆成多个；
2. **`no_std` 与否**：抽象层本身可以用 `core` 表达，但业务数据大概率需要 `alloc`；
3. **引导/会合机制**：客户端如何找到 `kb_core` 的服务端点（属于实现层，但会影响抽象层的"连接"语义）；
4. **跨机需求**：是否在近期范围内显式排除；
5. **`buffex` / `mm_ptr` 的定位**：见 §9 的"现有依赖的遗留问题"；
6. **本地创建对象的回收与幂等**：客户端建了 `LocalId` 却始终没同步（或同步失败）
   时如何清理；同一次同步重发时如何避免重复创建（应由 `kb_core` 侧按 `LocalId` 幂等）。

**已经关闭的问题**：

- ~~载荷表示（是否用 serde）~~ → 已定：使用 typed 通道 + `#[derive(Serialize, Deserialize)]`。
  "零 serde" 只是"以内存共享的 IPC 为前提"时的目标，该前提已放弃，因此它不再是约束。
  详见 dev-notes `abs_kb_svc-20260917-1254.md` §0.1 澄清二。
- ~~会话历史的归属~~ → 已定：工作区与会话**由 `kb_core` 管理并多端同步**，
  历史留在服务端，`GetSession` / `SessionDetail` / `SessionChanged` 保留。
  客户端的"当前选中"仍然留在本地。
  详见 dev-notes `kb_admin_desktop-20260917-1341.md` §5.1。
