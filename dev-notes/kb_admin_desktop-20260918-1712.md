# `kb_admin_desktop`：工作区 / 会话列表的增删（本轮），以及三个后续设计

- 日期：2026-09-18 17:12
- 状态：**本轮实施记录 + 待拍板事项**。
- 范围（按用户口径"先完成工作区列表和会话列表相关的功能"）：
  **工作区与会话的增 / 删 / 查**打通，服务端权威；
  **不做**会话正文（`GetSession`）与 `kb_core` 的模拟 LLM；
  **不做**工作区 / 会话的"改"。
- 前置：
  - [`kb_admin_desktop-20260918-1034.md`](kb_admin_desktop-20260918-1034.md) §11–§14：
    连接链路、FRB 的扁平 DTO 约定、上一轮"只读列表"的结论；
  - [`kb_core-20260917-1527.md`](kb_core-20260917-1527.md)：本地文件存储的布局与语义；
  - [`llm_kb-20260917-1655.md`](llm_kb-20260917-1655.md)：项目级落位表与未决项。

---

## 0. 结论

| 问题 | 结论 |
| :--- | :--- |
| 列表的增删走哪条路 | 全部用协议里**已经存在**的请求；`kb_core` 与 `abs_kb_svc_v1_desktop` 本轮**一行未改** |
| 谁承担"路径"的语义 | `AddWorkspace` 的 `path` 是 **`kb_core` 所在主机上**的目录；客户端只提交字符串，不碰自己的文件系统 |
| 公开面的变化 | 只发生在客户端：`kb_client_conn_mgr::KbClient` 加 4 个方法，FRB 加 4 个函数与 3 个扁平 DTO，Dart 的 `KbClientApi` 跟着镜像 |
| 新增内容会不会在下次连线还在 | 会：`kb_core` 落盘（`<storage_dir>/workspaces/*.json`、`sessions/<w>/*.json`），本轮已用真进程实测（§5） |
| 还差什么 | 会话正文、模拟 LLM（§4）；工作区 / 会话的"改"（§3）；多 `kb_core` 同时连接（§2） |

本轮**没有**新增任何协议请求，也没有改任何线格式——这是刻意的：服务端
（`kb_core/src/ipc_.rs` 的 `KbService`）在上一轮就已经把
`TrWorkspaceService` / `TrSessionService` 的每个方法实现完了，缺的只是客户端
到界面这条链路。

---

## 1. 本轮落地（按层）

| 层 | 位置 | 内容 |
| :--- | :--- | :--- |
| 连接管理器 | `kb_clients/crates/kb_client_conn_mgr/src/client_.rs` | `KbClient::{add_workspace, remove_workspace, create_session, remove_session}`，都由 `gen_mcf2` 展开成可取消 future，走既有的 `request_` 分派（本机 IPC / TCP 共用） |
| FRB 视图 | `kb_clients/kb_admin_desktop/rust/src/api/kb.rs` | `WorkspaceReport` / `SessionReport` / `OpReport`（全扁平）+ `add_workspace` / `remove_workspace` / `create_session` / `remove_session` |
| Dart 门面 | `lib/src/services/kb_client_api.dart` | 4 个方法镜像到 `KbClientApi` 与 `FrbKbClientApi` |
| 状态 | `lib/src/state/connection_controller.dart` | `addWorkspace` / `removeWorkspace` / `addSession` / `removeSession` / `newSessionInSelectedWorkspace`；每个都返回"空串 = 成功"，成功后刷新对应列表 |
| 界面 | `lib/src/widgets/sidebar/server_workspace_list.dart`、`sidebar_panel.dart` | 列表右上角 `+` 新建工作区；工作区行悬停出「新建会话 / 删除工作区」；会话行悬停出「删除会话」；删除工作区先确认；侧边栏顶部「新会话」按钮在已连接时改走服务端 |

三条实现取舍：

1. **`local_id` 不进入客户端 API**。协议里
   `AddWorkspaceRequest.local_id` / `CreateSessionRequest.local_id` 是"客户端
   本地先创建"时的簿记字段；线上仍然带着它往返一次，但 `KbClient` 的方法只回
   `Workspace` / `SessionSummary`——调用方要的是服务端分配的标识。这与协议
   `rpc_.rs` 里给 trait 定的口径一致。
2. **删除工作区的级联由服务端执行**（`Store::remove_workspace` 删会话目录再删
   工作区文件）。客户端只发 `RemoveWorkspace`，界面上用一句确认文案把"会连带
   删掉会话、且不可撤销"讲清楚。
3. **未连接时界面仍退回本地那一套**（`AppController`）。这样 widget 测试不需要
   初始化原生库，没有服务端也能起界面；代价是"本地工作区"与"服务端工作区"
   在同一个位置有两种来源，等本地那一套退役时一并收掉。

---

## 2. 决策：左侧栏左上角改成"当前主机名 + 切换 `kb_core`"（**用户要求记入 dev-notes**）

### 2.1 用户的口径

> `kb_admin_desktop` 从理论上来说可以一个前端同时连接多个 `kb_core`，因此界面
> 也要开始和 DSH 有所区别。左侧栏左上角现在固定显示 `llm_kb` 是巨大的浪费，
> 应该改为一个"显示当前主机名、同时可以切换不同主机上的 `kb_core` 连接"的按钮。

### 2.2 为什么这件事比换个标题大

现在的连接状态是**单连接**的：

- Rust 侧 `rust/src/api/kb.rs` 用 `static CURRENT: Mutex<Option<(String, Arc<KbClient>)>>`
  只留一个已连上的客户端，`connect_to` 再连一次就把旧的丢掉（`local-launch`
  的旧子进程会随之结束）；
- Dart 侧 `ConnectionController` 也只有一份 `_profileName / _workspaces / _sessions`。

"同时连多个"意味着从"当前连接"变成"连接集合 + 当前选中的那个"，并且要决定：

| 问题 | 倾向 |
| :--- | :--- |
| 工作区 / 会话缓存 | 按连接标识分别缓存，切换只切"视图"，不重连、不清缓存 |
| 子进程归属 | 每条 `local-launch` 连接各持有自己的子进程；断开某一条只收它自己那个 |
| 界面状态（当前工作区 / 会话） | 每个连接各记一份，切回来还在原处 |
| 右上角按钮的语义 | "当前主机"（点开=在已连的主机之间切换 + 管理连接方式），不再是"连接方式" |

### 2.3 主机名从哪来：**客户端自己的花名**，不改协议（2026-09-18 17:35 修订）

> **初稿判断错了**：曾以为"界面要显示主机名，就必须在 `ServerInfo` 里加 `host`
> 字段"，并据此列了"要不要改协议"的方案 A / B。用户随后澄清了口径：

> 主机名是一种"花名"或助记符，是客户端上用户自己任意起的，与握手协议没有关系；
> 它只存在于 `kb_admin_desktop` 自己的配置项中。

也就是说：**"主机名"就是客户端连接配置里的 `Connection::name`**（`kb_client_config`
的 TOML 里 `[[connections]] name = "实验室"`），它本来就已经是"界面上显示的名字 +
`default` 引用的键"。它**不是**对端主机名，也不进握手——`kb_core` 那边根本不知道
自己被叫什么。

因此这条决策的落法是：

- **协议零改动**：`ClientInfo` / `ServerInfo` 保持原样；
- 连接对话框里那栏从"名字"改叫**"主机名"**，说明文案点明"只是客户端这边的花名"；
- 左侧栏左上角的按钮显示当前连接的 `name`，点开在**已配置的连接**之间切换；
  切换 = 断开旧的、连新的（`local-launch` 的旧子进程随之结束）。

**"同时保持多条连接"没有做**：本轮实现的是"切换活动连接"（同一时刻仍只有一条）。
要做到真正的多连接，得把 Rust 侧的 `static CURRENT` 与 Dart 侧的一整套缓存改成
"连接集合 + 当前选中"，那是另一轮的事。实现与验证见
[`kb_admin_desktop-20260918-1740.md`](kb_admin_desktop-20260918-1740.md) §1。

---

## 3. 待拍板：工作区 / 会话的"改"

用户口径里的"增删查改"目前只做到了**增删查**——因为协议里没有"改"的请求：

| 想做的操作 | 协议里有吗 | 存储层有吗 |
| :--- | :--- | :--- |
| 改工作区名字 / 路径 | ❌ 无 `UpdateWorkspace` | ✅ `Store::save_workspace` |
| 改会话标题 | ❌ 无 `RenameSession` | ✅ `Store::rename_session` |
| 改会话正文 | 不叫"改"：追加消息属于生成域（§4） | ✅ `Store::append_turns` |

> **2026-09-18 17:35 补充**（与 §2.3 对照）：主机名只活在客户端配置里，所以改它
> 不需要协议；而**工作区名字是 `kb_core` 记录的一部分**（落盘在
> `<storage_dir>/workspaces/<w>.json` 的 `name` 字段），用户口径是"纯粹按用户意愿
> 起的名字，但改动影响的是 `kb_core` 中的记录"——所以它必须发一条请求给 `kb_core`，
> 也就是下边提案里的 `UpdateWorkspace`。这正是"改"至今没做的唯一原因。

补法很直接，建议的形状（**待拍板后才实现**）：

```rust
// request_.rs
UpdateWorkspace { workspace_id: WorkspaceId, name: String, path: String },
RenameSession  { workspace_id: WorkspaceId, session_id: SessionId, title: String },

// reply_.rs
WorkspaceUpdated(Workspace),      // 或复用 Reply::WorkspaceAdded 的载荷形状
SessionRenamed(SessionSummary),

// rpc_.rs
TrWorkspaceService::update_workspace(Workspace)
TrSessionService::rename_session(workspace_id, session_id, title)
```

值得注意的是"改"是**幂等**的（同一次重命重复发没有副作用），这与
`AddWorkspace` 的幂等性（1655 §3 第 5 条）是两回事，不必一起解决。

---

## 4. 下一轮：会话正文 + `kb_core` 的模拟 LLM

> **2026-09-18 17:40 已完成**：本节的设计原样落地，实现与验证见
> [`kb_admin_desktop-20260918-1740.md`](kb_admin_desktop-20260918-1740.md)。
> 与当初设想的两处差别：`Ask` 的应答复用了 `Reply::SessionDetail`（不是 `Ack`），
> 生成域真的抽成了 `TrGeneration` trait 并纳入 `TrKbService`。

用户描述的目标：**只要提问，`kb_core` 就把提示词逆序输出一遍作为回答**；
因为会话内容落盘，重新连线后还能看到新内容。

设计上的第一个岔路口是**要不要现在就引入事件流**：

- 协议里生成域的形状（`TrGeneration` + 事件订阅）在 `rpc_.rs` 的"尚未定义"
  一节里明确留着，要配合 `abs_async_iter::TrFlux` 定；
- 但本轮的验收目标是"内容落盘、重连可见"，**不需要**流式增量。

因此建议下一轮先做**同步一问一答**这条最小闭环：

```text
Request::Ask(AskRequest { workspace_id, session_id, turn_id, question, … })
   → kb_core：把 user 消息与 assistant 消息（question 的逆序）一起
      用 Store::append_turns 落盘
   → Reply::Ack（或携带落盘后的 SessionSummary / 两条 Turn）

Request::GetSession { workspace_id, session_id } → Reply::SessionDetail
   → 界面把正文渲染到中间对话区
```

`Ask` 的载荷、`Turn` / `TurnState`、`SessionDetail` 都是协议里现成的；逆序回答
只是 `question.chars().rev().collect()` 这一行临时逻辑。等真正的 LLM 插件接上时，
把"同步 AppendTurns"换成"事件流 + 落盘"，界面那一侧再跟着改成订阅。

这条同样**不改变公开协议**（`Ask` / `GetSession` 早已定义），所以它不需要额外的
拍板；它只是把 `kb_core` 里现在回 `BadRequest` 的两个请求接上。

---

## 5. 验证

环境：`CARGO_HOME=$PWD/external/cargo-home`（`~/.cargo` 本机只读，见 1655 §4）；
Flutter/Dart 在 `/root/develop/flutter/bin`。

| 项 | 命令 | 结果 |
| :--- | :--- | :--- |
| 连接管理器单测 + 集成 | `cargo test -p kb_client_conn_mgr` | 2 单元 + **6 集成**（新增"TCP 上的工作区/会话增删"）+ 1 文档，全绿 |
| 静态检查 | `cargo clippy -p kb_client_conn_mgr --all-targets` | 无告警 |
| 客户端 Rust 侧 | `(cd kb_clients/kb_admin_desktop/rust && cargo check)` | 通过（改完 `crate::api` 后已 `flutter_rust_bridge_codegen generate` 重生成） |
| Dart 静态检查 | `flutter analyze` | **No issues found** |
| Dart 测试 | `flutter test` | **42 项全绿**（原 37 + 新增 4 个状态用例 + 1 个对话框 widget 用例；golden 组照旧跳过） |
| 真进程端到端 | 起真 `kb-core` → 客户端进程 A 建工作区 + 会话 → 客户端进程 B（新进程）重连 | B 看到 A 建的两条；磁盘上 `workspaces/<w>.json` 与 `sessions/<w>/<s>.json` 都在（临时脚本用完已删） |

端到端的实际输出（节选）：

```text
=== 第一次连线：建工作区 + 建会话 ===
已建工作区: w-4cbc68b1… e2e库
已建会话: s-7256398f… e2e会话
=== 第二次连线（新进程）：应当还能看到 ===
工作区 1 个
  w-4cbc68b1…	e2e库	/tmp/e2e
    会话 1 个
      s-7256398f…	e2e会话	0 条

=== kb_core 日志 ===
客户端已连接
客户端已断开，继续等待下一位
客户端已连接
客户端已断开，继续等待下一位
```

两个"连接 → 断开"就是两次独立的客户端进程；第二条能看到第一条写的东西，
说明"新增内容在下一次连线依然可见"这条目标在本轮的数据范围内成立。

---

## 6. 相关文档

- [`kb_admin_desktop-20260918-1034.md`](kb_admin_desktop-20260918-1034.md)：
  连接配置、三种连接方式、FRB 的扁平 DTO 约定、上一轮的只读列表；
- `kb_clients/crates/kb_client_conn_mgr/README.md`：本轮的 4 个新方法；
- `kb_clients/kb_admin_desktop/README.md`：界面上的入口与"本轮边界"；
- `kb_svc/crates/kb_core/src/store_/mod.rs`：落盘布局与"会话属于工作区"的语义；
- `kb_svc/crates/abs_kb_svc_v1_desktop/src/rpc_.rs`：生成域"尚未定义"的原文。
