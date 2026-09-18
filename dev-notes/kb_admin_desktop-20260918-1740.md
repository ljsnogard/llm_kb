# `kb_admin_desktop`：主机名花名 + 会话正文 + `kb_core` 模拟 LLM

- 日期：2026-09-18 17:40
- 状态：**本轮实施记录**。
- 范围：
  1. 左侧栏左上角改成"主机名（客户端花名）+ 切换 `kb_core`"的按钮；
  2. 打通**会话正文**：选中会话读 `GetSession`，在中间对话区渲染；
  3. 在 `kb_core` 里实现**临时模拟的 LLM**：提问把问题逆序输出当回答，一问一答落盘。
- 前置：
  - [`kb_admin_desktop-20260918-1712.md`](kb_admin_desktop-20260918-1712.md)：
    上一轮（工作区 / 会话列表增删）、§2.3 的主机名口径修订、§3 的"改"提案；
  - [`kb_core-20260917-1527.md`](kb_core-20260917-1527.md)：本地文件存储；
  - `abs_kb_svc_v1_desktop/src/rpc_.rs`：按域 RPC trait 的形状与约定。

---

## 0. 结论

| 问题 | 结论 |
| :--- | :--- |
| "主机名"是什么 | **客户端连接配置里的 `name`（花名）**，与握手协议无关，协议零改动 |
| 左上角按钮 | 显示当前连接的花名 + 状态副标题；点开在已配置连接之间**切换** |
| `Ask` 的协议形状 | 新增 `TrGeneration` trait（只有 `ask`），纳入 `TrKbService`；应答复用 `Reply::SessionDetail` |
| 模拟 LLM 在哪 | 在 `kb_core` 的 `KbService` 里，用 `Store::append_turns` 落两条消息 |
| 会话正文怎么来 | `GetSession`（服务端早已实现，本轮只补客户端到界面） |
| 验收 | 真 `kb_core`：A 进程提问 → 落盘 → B 进程重连仍能看到问题与逆序回答（§4） |

---

## 1. 左上角：主机名（花名）+ 切换连接

### 1.1 名字从哪来（口径已修订）

初稿（1712 §2.3）曾判断"要显示主机名就得给 `ServerInfo` 加字段"。用户澄清后修正为：

> 主机名是一种"花名"或助记符，是客户端上用户自己任意起的，与握手协议没有关系；
> 它只存在于 `kb_admin_desktop` 自己的配置项中。

所以它就是 `kb_client_config` 里 `Connection::name`——那个字段本来就同时承担
"界面显示名"与"`default` 引用的键"两个职责。本轮唯一要改的是**呈现与文案**：

- 连接对话框那一栏从"名字"改为**"主机名"**，并加一句"只是客户端这边的花名"；
- 左上角显示它，副标题给"`kb_core <版本> · 本机/远程`"（未连接时是"未连接 kb_core"）。

### 1.2 切换怎么落地

`_HostRow` 用 `PopupMenuButton` 列出 `ConnectionController.profiles`，选中哪一条就
把它交给 `ConnectionController.connect(profile)`；当前连的那条打一个勾，最后一项是
"管理连接方式…"（打开原来的连接对话框）。没有任何已配置项时，按钮直接开对话框。

`connect()` 本来就会替换掉旧客户端（Rust 侧 `static CURRENT` 被覆盖，旧的
`local-launch` 子进程随 `Arc` 释放而被收掉），所以"切换"不需要新机制。

**与"同时连多个"的区别（未做）**：现在仍然只有一条活动连接，切换即断开重连；
Dart 侧的工作区 / 会话 / 正文缓存也仍然只有一份。真正的多连接需要把两侧都改成
"连接集合 + 当前选中"，留待以后。

折叠轨道上对应的是一个 `dns` 图标按钮，颜色反映连接状态，点开同一个菜单。

---

## 2. 会话正文与模拟 LLM

### 2.1 协议：新增 `TrGeneration`（**公开面变更**）

`rpc_.rs` 里原来只到工作区 / 会话 / 握手三个域，"生成域"是明确留白的。本轮补上：

```rust
pub trait TrGeneration: TrKbEndpoint {
    type Ask<'f>: TrMayCancel<'f, MayCancelOutput = Result<SessionDetail, RpcError<Self::Error>>>
    where Self: 'f;
    fn ask<'f>(&'f self, request: AskRequest) -> Self::Ask<'f>;
}
```

- 纳入 `TrKbService` 的父 trait 集合（blanket impl 同步加一项），因此
  `kb_svc_servo_ipc::Connection::serve` 的约束自动覆盖它；
- 三个实现方都补齐了：`kb_core::KbService`（真实现）、`kb_svc_servo_ipc::Client`
  （客户端代理）、`kb_svc_servo_ipc/tests/round_trip.rs` 的 `TestService`。

**`Ask` 的应答复用 `Reply::SessionDetail`**，没有新增 `Reply` 变体。理由写在
`TrGeneration` 的文档里：真正要做的是**流**（`TurnStarted` → `TextDelta` →
`TurnFinished`），那套形状要配合 `abs_async_iter::TrFlux` 定；而在"落盘、重连可见"
这个验收目标下，同步一问一答就够，且 `SessionDetail` 正好是调用方要的东西
（两条新消息 + 最新摘要）。等流式生成落地时，这个返回类型会换成流形状——
**那才是下一次公开协议变更**。

`Reply::SessionDetail` 被 `GetSession` 与 `Ask` 共用，靠 `request_id` 关联不会混；
这条"一个应答变体服务两个请求"的临时用法在 `connection_.rs` 的派发分支上有注释。

### 2.2 `kb_core`：模拟 LLM 与落盘

`ipc_.rs` 里新增 `ask_async`：

```text
校验取消令牌
  → simulated_exchange_(&request)        造两条 Turn（纯函数，可单测）
       user      : turn_id = 客户端给的，text = 问题
       assistant : turn_id = 服务端生成，text = 问题按字符逆序，notice = "临时模拟"
  → Store::append_turns(..)              两条一起落盘，摘要的 turn_count/updated_at 由存储层维护
  → Store::get_session(..)               回提问之后的完整会话
```

两个刻意的选择：

- **逆序按字符**（`chars().rev()`）而不是按字节：中文不会碎成半个；
- **助手回合挂一条非错误的 `Notice`**：界面会渲染成一行说明，用户一眼能看出这不是
  真模型答的，而不是被"逆序输出"搞糊涂。

`Store::append_turns` 因此从"只有测试在调"变成生产路径，`#[allow(dead_code)]` 一并删掉。

### 2.3 客户端到界面

| 层 | 新增 |
| :--- | :--- |
| `kb_client_conn_mgr::KbClient` | `get_session(...)`、`ask(request)`（都走既有的 `request_` 分派） |
| FRB `rust/src/api/kb.rs` | `TurnView`（一条消息）、`SessionDetailReport`、`get_session(...)`、`ask(...)` |
| Dart `KbClientApi` | `getSession` / `ask` 两个方法 |
| `ConnectionController` | `selectSession` / `loadSessionDetail` / `ask`；正文缓存 `_details`、选中态 `_selectedSessionId` |

Dart 侧的失效规则：切工作区时清掉选中的会话；删会话 / 删工作区 / 断开 / 重连时
清掉对应缓存；提问成功后用返回值直接更新正文，并刷新那个工作区的会话列表
（`turn_count` 与活动时间变了）。

---

## 3. 界面接线

- **侧边栏会话行**：点一下 = `selectSession(workspaceId, sessionId)`（先拉列表再拉正文，
  两步都命中缓存就不重复发请求）；选中态按 `selectedSessionId` 高亮。
- **对话区**（`conversation_pane.dart`）：两种数据源——
  - 已连接：面包屑用服务端的工作区名 / 会话名，正文是 `SessionDetailReport.turns`
    经 `chatTurnOf_` 映射成的 `ChatTurn`；输入框发送 = `ConnectionController.ask`；
  - 未连接：原样退回本地那一套（发送只补一条"还没有连接 kb_core"的说明）。
- **重建**：`ConversationPane` 现在自己监听 `ConnectionController`（与
  `ServerWorkspaceList` 同一套做法）。`KbAdminApp` 只在 `AppController` 变化时重建，
  不听连接状态——这一点上一轮就在侧边栏踩过一次（1712 §13.2 第 1 条），这次提前处理了。

消息渲染复用现成的 `MessageList`：用户气泡、助手正文、`notice` 提示条都已经支持，
所以模拟 LLM 的那条说明能直接显示出来。

---

## 4. 验证

环境：`CARGO_HOME=$PWD/external/cargo-home`；Flutter/Dart 在 `/root/develop/flutter/bin`。

| 项 | 命令 | 结果 |
| :--- | :--- | :--- |
| 全 workspace 编译 | `cargo check --workspace --all-targets` | 通过（只有 `kb_rig_llm_v1_agent` 的既有依赖告警） |
| 协议 + 传输 | `cargo test -p abs_kb_svc_v1_desktop -p kb_svc_servo_ipc` | 全绿；新增 `ask_is_dispatched_to_the_generation_domain_` |
| `kb_core` | `cargo test -p kb_core` | 29 项全绿；新增 `simulated_exchange_reverses_by_chars_`、`ask_persists_the_exchange_for_the_next_connection_` |
| 连接管理器 | `cargo test -p kb_client_conn_mgr` | 2 单元 + 7 集成（新增 `tcp_profile_reads_session_and_asks_`）+ 1 文档，全绿 |
| FRB 重生成 | `flutter_rust_bridge_codegen generate` + `(cd rust && cargo check)` | 通过；生成的 `TurnView` / `SessionDetailReport` 都是普通 Dart 类 |
| Dart 静态检查 | `flutter analyze` | **No issues found** |
| Dart 测试 | `flutter test` | **46 项全绿**（新增：选正文、提问、主机切换、对话区显示逆序回答；golden 组照旧跳过） |

### 真进程端到端（核心验收）

起真 `kb-core` → **进程 A** 建工作区 + 会话 + 提问 → 丢弃连接 → **进程 B**（新进程）
重连、列会话、读正文：

```text
=== 第一次连线：建工作区 + 建会话 + 提问 ===
提问后消息数=2 回答=Some("界世好你")
=== 第二次连线（新进程）：应当还能看到问题与逆序回答 ===
工作区 1 个
  w-d16527fd…	e2e库	会话 1 个
    会话 s-08c0e53e… 《第一问》 2 条
      [User] 你好世界
      [Assistant] 界世好你
```

磁盘上的 `sessions/<w>/<s>.json` 也确认带着这两条消息与那条 `notice`。临时脚本
用完已删，不在仓库里。

---

## 5. 没做 / 下一步

1. **工作区 / 会话的"改"**：仍缺 `UpdateWorkspace` / `RenameSession`，提案见
   1712 §3；这是协议变更，需要先拍板。
2. **流式生成与事件**：现在是同步一问一答，`TrGeneration` 的返回类型以后要换成流；
   `Request::Cancel` 也等那时一起定。
3. **真正的多连接**：本轮只做"切换活动连接"；"一个前端同时连多个 `kb_core`、
   各自保留列表与正文缓存"还没做。
4. **会话正文的分页 / 增量**：现在每次都整份取回（会话不大时没问题）。

---

## 6. 相关文档

- [`kb_admin_desktop-20260918-1712.md`](kb_admin_desktop-20260918-1712.md)：上一轮列表增删、
  主机名口径修订、§3 的"改"提案；
- `kb_svc/crates/abs_kb_svc_v1_desktop/src/rpc_.rs`：`TrGeneration` 的形状与理由；
- `kb_svc/crates/kb_core/src/ipc_.rs`：`ask_async` 与 `simulated_exchange_`；
- `kb_svc/crates/kb_core/src/store_/mod.rs`：`append_turns` 的落盘语义；
- `kb_clients/kb_admin_desktop/lib/src/widgets/conversation/conversation_pane.dart`：
  对话区的两种数据源。
