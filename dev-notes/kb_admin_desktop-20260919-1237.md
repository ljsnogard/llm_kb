# `kb_admin_desktop`：工作区 / 会话改名 + 会话命名与"空会话不落盘"

- 日期：2026-09-19 12:37
- 状态：**本轮实施记录**。
- 范围（用户口径）：
  1. 增加**工作区改名**与**会话改名**；
  2. 会话名默认由 `kb_core` 起，来源是**第一个问题的前若干字**；新建时还没提问就叫
     「新会话」；
  3. **落盘的会话必须有真实对话内容，或者有默认名字以外的会话名**——空的
     「新会话」不许落盘。
- 前置：
  - [`kb_admin_desktop-20260918-1712.md`](kb_admin_desktop-20260918-1712.md) §3：
    "改"的提案（当时叫 `UpdateWorkspace`，本轮按用户口径收窄成"改名"）；
  - [`kb_admin_desktop-20260918-1740.md`](kb_admin_desktop-20260918-1740.md)：
    会话正文、`TrGeneration`、模拟 LLM；
  - `kb_core/src/store_/mod.rs`：落盘布局与既有语义（本轮加了第 4 条约定）。

---

## 0. 结论

| 问题 | 结论 |
| :--- | :--- |
| 改名怎么表达 | 协议新增 `Request::RenameWorkspace` / `Request::RenameSession` 与 `Reply::WorkspaceRenamed` / `Reply::SessionRenamed` |
| 为什么不是 `UpdateWorkspace` | 用户口径是"改名"；工作区路径的修改没有界面需求，收窄成只改 `name` 更不容易误伤 |
| 会话名谁起 | `kb_core`：`Store::normalize_title_` 已有"显式标题优先，否则取首条用户消息前 24 字"的逻辑，`CreateSession`/`Ask` 都走它 |
| 空的「新会话」 | `StoreError::EmptySession`：`create_session` 与 `save_session` 都拒绝"没有消息 + 只有默认名" |
| 那「新会话」按钮呢 | 改成**客户端草稿**：不发请求；首次提问时才 `CreateSession`（带首问）+ `Ask` |
| 重复记录问题 | `Ask` 按 `turn_id` 幂等：用户回合不重复添加，回答标识由用户回合标识确定性推导 |
| 验证 | 真 `kb_core`：改名 → 草稿建会话（名为问题）→ 提问（去重）→ 会话改名 → 空会话被拒 → 重连可见（§4） |

---

## 1. 协议：两个改名请求（**公开面变更**）

```rust
// request_.rs
RenameWorkspace { workspace_id: WorkspaceId, name: String },
RenameSession   { workspace_id: WorkspaceId, session_id: SessionId, title: String },

// reply_.rs
WorkspaceRenamed(Workspace),        // 改名之后的工作区
SessionRenamed(SessionSummary),     // 改名之后的会话摘要
```

trait 上分别落在既有的两个域：

```rust
TrWorkspaceService::rename_workspace(workspace_id, name) -> Workspace
TrSessionService::rename_session(workspace_id, session_id, title) -> SessionSummary
```

三个实现方都补齐（`kb_core::KbService`、`kb_svc_servo_ipc::Client`、以及两份测试
mock），派发分支、`reply_kind_` / `request_kind_` 同步更新。

两条语义取舍：

1. **只改 `name`，不碰路径**。1712 §3 曾提案 `UpdateWorkspace { name, path }`；
   本轮按"改名"收窄——工作区路径若允许改，界面要额外处理"目录不存在"这类
   问题，而本轮的目标只是名字。`Store::rename_workspace` 因此也明确注释
   "不碰磁盘目录"。
2. **工作区名不能空白**（[`StoreError::EmptyName`]）：工作区没有"从消息推导"
   这条退路，空名字会让它在界面上无法辨认。**会话标题可以空白**——那表示
   "交回服务端重新推导"（取首条用户消息，取不到就是默认名），所以它也能用来
   清掉手工起的名字。

---

## 2. 会话命名与"空会话不落盘"

### 2.1 命名规则

`kb_core` 的 `Store::normalize_title_` 本来就做了这件事：

```text
显式 title 非空   → 用它
否则             → 取 turns 里第一条「用户」消息的第一行，最多 24 个字符（按字符，
                   不是字节，中文不会截半个）；一条都没有 → DEFAULT_TITLE_「新会话」
```

本轮没有改它，只是**让它真正生效**：以前 `append_turns` 不刷新标题，于是
"先建空会话、再提问"的会话会一直叫「新会话」；现在会话是被**带着第一个问题**建出来
的（见 §2.3），标题在创建那一刻就定下来了。

### 2.2 第 4 条存储约定

`store_/mod.rs` 的模块文档从"三条硬性约定"变成"四条"，新增：

> **空会话不落盘**：一个会话要么有消息，要么有一个非默认的名字；两者都不满足时
> `create_session` / `save_session` 返回 [`StoreError::EmptySession`]。

判断抽成 `ensure_persistable_(title, turns)`，`create_session` 与 `save_session`
共用。后者是 `append_turns` / `rename_session` 的落盘入口，所以这条约定**绕不过去**：
不管从哪个入口，一个"空的「新会话」"都进不了磁盘。

`StoreError::EmptySession` / `EmptyName` 在 `ipc_.rs` 里都翻成
`ErrorCode::BadRequest`——这是调用方输入不合法，不是服务端故障。

### 2.3 那「新会话」按钮怎么办：客户端草稿

既然空会话不能落盘，点「新会话」就不能马上发 `CreateSession`。做法是
**客户端持有一个草稿会话**：

```text
点「新会话」        → ConnectionController._draftSession = true（不发请求）
                     对话区标题显示「新会话」，输入框可用
发出第一条消息      → ① CreateSession { title: None, turns: [ user(turn_id, 问题) ] }
                         kb_core 据此起名（问题开头若干字）并落盘
                      ② Ask { session_id, turn_id: 同一个, question: 问题 }
                         服务端发现这一轮的用户回合已存在 → 只追加回答
                      → 草稿结束，选中新建出来的会话
```

第 ② 步靠的是 **`Ask` 的 `turn_id` 幂等**（见下一节）。

用户口径里"新建会话时还没有提问，名字就叫「新会话」"因此在界面上如实成立：
草稿在会话列表里还不存在，但对话区的标题就是「新会话」。

### 2.4 `Ask` 的幂等（草稿流程的前提）

`kb_core::ask_async` 改成：

```text
读一次会话（拿现有 turns）
  ├─ 已经有本轮的回答 → 原样返回（重试不会再生成第二条回答）
  ├─ 已经有本轮的用户回合 → 只追加回答
  └─ 都没有 → 追加「用户 + 回答」
```

"本轮的回答"要能被认出来，就不能用随机标识：回答的 `turn_id` 现在由用户回合标识
**确定性推导**（`answer_turn_id_`，形如 `t-1` → `t-1-a`）。代价是标识不再随机，
但换来的是"先建会话、再提问"与"重试一次提问"都安全——这正好对应
`dev-notes/kb_admin_desktop-20260918-1740.md` §2.1 里"以后要做流式"的那个缺口。

---

## 3. 客户端与界面

| 层 | 改动 |
| :--- | :--- |
| `kb_client_conn_mgr::KbClient` | `rename_workspace` / `rename_session` |
| FRB `rust/src/api/kb.rs` | `rename_workspace` / `rename_session`；`create_session` 增加 `turn_id` + `question` 两个参数（空 `turn_id` = 不带首问） |
| Dart `KbClientApi` | 同上三个方法 |
| `ConnectionController` | `_draftSession` + `draftingSession`；`startDraftSession` 取代原来的 `addSession`（后者会建空会话，已被约定禁止）；`renameWorkspace` / `renameSession`；`ask` 支持草稿 |
| 侧边栏 | 工作区行悬停多一个「工作区改名」，会话行悬停多一个「会话改名」；工作区行的 `+` 改成开草稿；「新会话」按钮也开草稿 |
| 对话区 | 草稿态标题显示「新会话」，空态提示改成"输入第一个问题，kb_core 会用这个问题给它起名" |

改名对话框是一个通用的 `_RenameDialog`（预填当前名字，留空即交回服务端推导——
会话标题如此，工作区名会被客户端直接拦下并提示"名字不能为空"）。

---

## 4. 验证

环境：`CARGO_HOME=$PWD/external/cargo-home`；Flutter/Dart 在 `/root/develop/flutter/bin`。

| 项 | 命令 | 结果 |
| :--- | :--- | :--- |
| 全 workspace 编译 | `cargo check --workspace --all-targets` | 通过（只有既有告警） |
| 协议 + 传输 | `cargo test -p abs_kb_svc_v1_desktop -p kb_svc_servo_ipc` | 全绿（mock 补了两个改名方法） |
| `kb_core` | `cargo test -p kb_core` | **34 项**全绿；新增 `rename_workspace_updates_name_in_place_`、`empty_session_needs_a_custom_name_`、`ask_is_idempotent_for_the_same_turn_`、`draft_flow_names_the_session_from_the_first_question_`、`rename_workspace_and_session_through_the_service_` |
| 连接管理器 | `cargo test -p kb_client_conn_mgr` | 2 单元 + **8 集成**（新增 `tcp_profile_renames_workspace_and_session_`）+ 1 文档，全绿 |
| Dart 静态检查 | `flutter analyze` | **No issues found** |
| Dart 测试 | `flutter test` | **49 项全绿**（新增草稿流程、工作区改名、会话改名与空白回落、界面草稿提问） |

### 真进程端到端

起真 `kb-core` → 一个客户端进程做完整套动作 → 丢弃 → **另一个新进程**重连：

```text
=== 第一次连线：改名 / 草稿建会话 / 提问 / 空会话应被拒 ===
工作区改名: 旧库 → 新库
草稿建会话: 《你好世界》
提问后消息数=2（去重后应为 2） 回答=Some("界世好你")
会话改名: 《改过的标题》
空会话按要求被拒: 服务端拒绝: 空会话需要一个自定义名字，或者至少带上第一条消息

=== 第二次连线（新进程）：应当看到新名字与新标题 ===
工作区 1 个
  w-257da83d…	新库	会话 1 个
    会话《改过的标题》 2 条
      [User] 你好世界
      [Assistant] 界世好你
```

磁盘上的 `sessions/<w>/<s>.json` 摘要为
`{'title': '改过的标题', 'turn_count': 2}`——改名与内容都真的落了盘。
第 ② 步"提问后消息数=2"同时证明了 `Ask` 的去重生效（否则会是 3 条）。
临时脚本用完已删。

---

## 5. 没做 / 下一步

1. **工作区改路径**：本轮只改名；`Store::save_workspace` 本来就能改路径，缺的是
   界面与"目录校验"，需要时再开。
2. **流式生成与事件**：`Ask` 仍是同步一问一答（`dev-notes/kb_admin_desktop-20260918-1740.md` §2.1）。
3. **真正的多连接**：左上角仍只是"切换活动连接"（同前一份记录 §1.2）。
4. **空名字的界面文案**：现在由客户端拦工作区的空名字；会话的空标题表示
   "重新推导"，两者语义不同，界面上已经分别提示。

---

## 6. 相关文档

- [`kb_admin_desktop-20260918-1712.md`](kb_admin_desktop-20260918-1712.md) §3：
  改名提案的由来（本轮落地，并收窄成"改名"）；
- [`kb_admin_desktop-20260918-1740.md`](kb_admin_desktop-20260918-1740.md)：
  会话正文、`TrGeneration` 与模拟 LLM；
- `kb_svc/crates/kb_core/src/store_/mod.rs`：四条硬性约定与 `ensure_persistable_`；
- `kb_svc/crates/kb_core/src/ipc_.rs`：`ask_async` 的幂等与 `simulated_answer_`。
