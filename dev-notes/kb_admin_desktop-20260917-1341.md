# kb_admin_desktop × kb_core 通信需求调查

- 日期：2026-09-17
- 状态：**调查结果**。对应的数据定义已落地在 [`abs_kb_svc::v1::desktop/`](../../kb_svc/crates/abs_kb_svc/src/v1/desktop/mod.rs)。
- 前置文档：[`abs_kb_svc-20260917-1254.md`](abs_kb_svc-20260917-1254.md)（选型与 `abs_kb_svc` 定位）

---

## 0. 结论摘要

1. **客户端目前与 `kb_core` 之间没有任何通信**：所有状态都在本机 `shared_preferences` 里，
   `flutter_rust_bridge` 只完成了初始化（`rust/src/api/simple.rs` 里只有一个 `greet` 示例）。
   所谓"改造为新的连接架构"实际上是**第一次**把这条通道接起来，不存在需要迁移的存量协议。
2. **9 个本地存储键里，5 个是纯界面偏好（永远留在客户端），4 个应当改为服务端读写**。
3. 需要与 `kb_core` 通信的功能共 **6 类**：对话（生成 + 取消）、LLM 服务配置、
   工作区、会话与历史、目录浏览，以及握手/状态同步。
4. 有 **3 处界面已经预留但尚未接线的能力**：流式增量渲染、取消生成按钮、
   目录浏览面板——它们决定了数据定义里必须先有对应的消息类型。
5. 本次交付的数据定义见 [`abs_kb_svc::v1::desktop/`](../../kb_svc/crates/abs_kb_svc/src/v1/desktop/mod.rs)：
   15 个请求、11 个应答、9 个事件、6 个标识新类型，全部为纯数据 + serde 派生；
   **23 个单元测试 + 9 个文档测试通过**，`cargo clippy` 对本 crate 无告警。
6. **工作区与会话由 `kb_core` 管理并多端同步，标识由它分配；但客户端可以本地先创建，
   同步时再拿标识**（见 §5.1）。为此协议引入 [`LocalId`] 作为"尚未同步"的临时标识。

---

## 1. 调查方法与范围

**范围**：`kb_clients/kb_admin_desktop/` 的全部非生成代码（Dart 侧 7097 行、Rust 侧 291 行），
重点是 `lib/src/{models,services,state,widgets}` 与 `rust/src/api`。

**判据**：一个功能"需要与 `kb_core` 通信"，当且仅当它的正确性依赖知识库服务端持有的状态
（数据库、插件、磁盘目录、LLM 凭据），而不是客户端自己的界面偏好。

**方法**：从数据来源反推——逐个检查 [`LocalStore`](../../kb_clients/kb_admin_desktop/lib/src/services/local_store.dart)
持久化的键、`AppController` 的内存状态、以及各 widget 调用 `AppController` 的方法，
看每一项"如果 kb_core 不知道它，功能还能不能成立"。

---

## 2. 现状：一切都在客户端本地

启动链路（`lib/main.dart`）：

```text
LocalStore.open()  →  AppController(snapshot)  →  runApp
                   ↘  RustLib.init()（仅初始化，无业务接口）
```

- `AppController` 的每一处修改都只做两件事：`notifyListeners()` + 写 `shared_preferences`
  （工作区写入还有 500ms 防抖，见 `_scheduleWorkspaceSave`）。
- `LocalStore` 一共 9 个键，全部在客户端：

| 键 | 内容 | 归属 |
| :--- | :--- | :--- |
| `kb.theme_mode` | 主题模式 | 客户端本地 |
| `kb.sidebar_width` | 侧边栏宽度 | 客户端本地 |
| `kb.sidebar_collapsed` | 侧边栏是否折叠 | 客户端本地 |
| `kb.file_panel_shown` | 文件面板是否展开 | 客户端本地 |
| `kb.file_panel_width` | 文件面板宽度 | 客户端本地 |
| `kb.services` | LLM 服务列表（**含 API key 明文**） | **应改为服务端** |
| `kb.active_service` | 当前生效的服务 | **应改为服务端** |
| `kb.workspaces` | 工作区 → 会话 → 消息 整棵树 | **应改为服务端** |
| `kb.active_workspace` | 当前选中的工作区 | 客户端本地（界面状态） |

> 客户端代码自己已经写明了这个结论：
> `local_store.dart` 顶部注释——"等 `kb_core` 补上这些接口，本文件里除了界面偏好之外的部分
> 都应改为服务端读写"；`settings_dialog.dart`——"接入 kb_core 之后会改为由服务端统一保管"。

---

## 3. 逐项分类：哪些功能需要 IPC

| # | 功能 | 现状 | 需要 IPC？ | 依据 |
| :--- | :--- | :--- | :---: | :--- |
| 1 | 主题 / 侧边栏 / 文件面板布局 | 本地持久化，已可用 | ❌ | 纯界面偏好，与知识库无关 |
| 2 | 工作区列表（增 / 删 / 选） | 本地持久化；新增时手填 name + path | ✅（增删） | 工作区指向磁盘目录，属于知识库域；`workspace_list.dart` 有 TODO 要换成目录选择器 |
| 3 | 会话（新建 / 选择 / 删除） | 本地持久化 | ✅（新建/删除/列表） | 历史归属服务端；`chat_session.dart` 注释"等服务端补上多会话接口，再把 id 换成服务端下发的" |
| 4 | 消息与历史 | 本地持久化 | ✅ | 同上；且"一轮生成"必须走 IPC |
| 5 | **对话生成（提问 → 流式增量 → 结束）** | **未接线**：`_send()` 只本地追加两条消息并提示"对话通道尚未接入" | ✅ **核心** | 只有 kb_core 能调插件、只有插件持有 LLM 上下文 |
| 6 | **取消生成** | `Composer` 已有 `busy` / `onCancel` 参数，但 `conversation_pane` 没传 | ✅ | 界面已预留，未接线 |
| 7 | LLM 服务配置（增 / 改 / 删 / 切换生效） | 本地持久化，界面完整（`settings_dialog.dart` 554 行） | ✅ | API key 应由服务端保管；服务端要拿它调插件 |
| 8 | API key 掩码回显 | 客户端常量 `kMaskedApiKey = '••••••••'` | ✅（约定） | 掩码语义必须与 `kb_svc_salvo::settings::MASKED_KEY` 一致，否则会把掩码写进配置 |
| 9 | 工作区文件浏览 | **空占位**，面板骨架 + 空状态文案 | ✅（规划） | 文案明写"文件浏览器将在 kb_core 提供目录接口后接入" |
| 10 | 知识库检索 / 编辑 | 不存在 | —（未来） | 本轮不定义 |
| 11 | FRB 原生层 | `RustLib.init()` + `greet` 示例 | ❌ | 但新架构**必须**依赖它：Dart 侧要通过 FRB 调用 Rust 侧的 `abs_kb_svc`/`kb_svc_servo_ipc` |

**三条对数据定义的直接影响**：

- 第 5、6 条要求"流式 + 可取消"，这决定了必须有一个**事件推送方向**，
  以及一个"取消"请求——不能只做请求/应答。
- 第 7、8 条要求"key 只回掩码、编辑时不覆盖"，这决定了需要一个三态
  （保留 / 设置 / 清空）的 key 更新语义，而不是一个字符串。
- 第 9 条虽然是空占位，但界面骨架已经在了，数据定义里必须预留目录列举。

---

## 4. 需要交流的数据（对照已交付的代码）

定义位置：[`kb_svc/crates/abs_kb_svc/src/v1/desktop/`](../../kb_svc/crates/abs_kb_svc/src/v1/desktop/mod.rs)。
下面只列"为什么需要它"，字段细节以代码为准。

### 4.1 握手与状态

| 数据 | 为什么需要 |
| :--- | :--- |
| `ClientInfo` / `ServerInfo`（含 `protocol_version`） | 客户端与服务端版本可能不同步；不一致要明确拒绝 |
| `Event::Ready(ServerState)` / `Event::StateChanged(ServerState)` | `plugin_online`、服务列表、生效服务、服务端版本——旧协议里的 `ready` 帧就有这些，界面靠它决定"能不能提问" |

### 4.2 对话

| 数据 | 对应界面 |
| :--- | :--- |
| `Request::Ask(AskRequest)` | `conversation_pane._send()` |
| `Request::Cancel { turn_id }` | `Composer.onCancel`（已预留） |
| `Event::TurnStarted` | `ChatTurn.state = streaming` 的起点 |
| `Event::Delta(TextDelta { logic, text })` | `ChatTurn.text` / `ChatTurn.reasoning`（按 `logic` 分流） |
| `Event::ToolCall` | `ChatTurn.toolCalls` |
| `Event::Usage` | `ChatTurn.usage` |
| `Event::TurnFinished { reason }` | `ChatTurn.state = done` |
| `Event::Error` / `ErrorReply` | `ChatTurn.notice`（`is_error = true`） |

> `ChatTurn` 的每个字段都能在旧协议里找到对应物：`chat_turn.dart` 顶部注释就写着
> "字段命名与 `kb_svc_salvo::wire` 的浏览器侧帧保持一致"。

### 4.3 LLM 服务

| 数据 | 对应界面 |
| :--- | :--- |
| `Request::{ListServices, UpsertService, RemoveService, UseService}` | `settings_dialog.dart` 的服务列表 / 编辑 / 删除 / 「使用」 |
| `Reply::{ServiceList, ServiceUpdated, Ack}` | 上面四个操作的返回 |
| `ServiceSummary`（含 `has_api_key` / `api_key_masked`） | 服务列表项的回显 |
| `ApiKeyUpdate::{Keep, Set, Clear}` | 编辑对话框里"没动 key"与"清空 key"必须可区分 |
| `MASKED_API_KEY` | 与 `kMaskedApiKey` / `settings::MASKED_KEY` 对齐 |

### 4.4 工作区 / 会话

| 数据 | 对应界面 |
| :--- | :--- |
| `Request::{ListWorkspaces, AddWorkspace, RemoveWorkspace}` | `workspace_list.dart` |
| `Workspace`（`workspace_id` / `name` / `path`） | 侧边栏条目 + 文件面板头部回显路径 |
| `Request::{ListSessions, CreateSession, RemoveSession, GetSession}` | 会话列表与 `startSession()` |
| `SessionSummary` / `SessionDetail` / `Event::SessionChanged` | 会话标题、时间、消息数；标题由首条用户消息派生（现由 `_deriveTitle` 本地做） |

### 4.5 目录浏览

| 数据 | 对应界面 |
| :--- | :--- |
| `Request::ListDirectory` → `Reply::DirectoryListing` | `workspace_file_panel.dart`（目前是空状态占位） |
| `DirEntry` / `DirEntryKind` | 文件树条目 |

---

## 5. 与现有客户端模型的差异（有意为之）

### 5.1 标识归属与"本地先创建"

这是本轮新增的决策，也是协议里唯一带**状态机**的地方。

| 对象 | 谁分配标识 | 能不能本地先创建 | 协议如何表达 |
| :--- | :--- | :--- | :--- |
| 工作区 | **`kb_core`** | ✅ 可以 | 请求带 [`LocalId`]，应答回配对关系 |
| 会话 | **`kb_core`** | ✅ 可以 | 同上（且必须先同步工作区） |
| 一轮生成 | 客户端 | —（它本来就是客户端发起的） | `AskRequest.turn_id` |
| LLM 服务 | 用户取名 | — | `ServiceId` 是名字，不是 uuid |

**为什么工作区/会话的标识必须由 `kb_core` 分配**：它们是被 `kb_core` 持有、
**多端同步**的对象。如果让客户端自己生成标识，两台客户端离线各自新建后同步就会撞车，
或者需要一套"标识合并"逻辑。由服务端分配则天然只有一个权威来源。

**为什么又允许本地先创建**：工作区指向的是**本机磁盘目录**，用户在离线或还没连上
`kb_core` 时完全可能先建好；会话更是随手就建。若强制"必须先连上才能建"，
界面就无法在未连接状态下使用，也会让"新建"变成一个可能失败的远程调用。

**协议怎么表达这件事**：用**两个不同的类型**，而不是"一个可以为空的 id"：

```text
客户端本地                          kb_core
─────────                          ────────
新建工作区 → LocalId("l-…")          （尚不存在）
   └── Request::AddWorkspace { local_id, name, path } ──►  分配 WorkspaceId
   ◄── Reply::WorkspaceAdded { local_id, workspace }  ──   并持久化
把 LocalId 换成 WorkspaceId；此后一律用服务端标识

（会话同理，且 CreateSession 需要先拿到 workspace_id）
```

这样做的好处是把"还没同步"这件事**顶到类型层面**：把 `LocalId` 当成 `WorkspaceId`
用是编译错误，不可能因为忘记同步而悄悄产生悬挂引用。

**同步顺序**：先工作区、后会话——`CreateSessionRequest.workspace_id` 只能来自
`Reply::WorkspaceAdded`。会话同步时可以把**客户端已经攒下的消息**一并带上
（`CreateSessionRequest.turns`），这样离线期间的历史不会丢。

**标识格式**：`<前缀>-<uuid-v4>`（`w-` / `s-` / `t-` / `l-` / `q-`）。
前缀只为便于人读，接收方不得解析它——标识始终当作不透明字符串。
生成函数集中在 [`ids_`](../../kb_svc/crates/abs_kb_svc/src/v1/desktop/ids_.rs) 里，
避免两端各写一份格式。

### 5.2 其余差异

| # | 差异 | 理由 |
| :--- | :--- | :--- |
| 1 | 协议里 `Workspace` **不含** `sessions`，会话要单独拉 | 避免每次列工作区都把整棵历史树传一遍；客户端自己组装界面结构 |
| 2 | "当前选中的工作区 / 会话"**不上传** | 那是每个客户端各自的界面状态（多窗口时会不同），留在本地 |
| 3 | "当前生效的 LLM 服务"**要上传** | 它决定 kb_core 用哪个服务调插件，是服务端状态 |
| 4 | `Role` 直接用 `abs_llm::v1::cont::Role`（4 个变体） | 客户端目前只有 `user` / `assistant` 两个；复用而不镜像，客户端忽略其余即可 |
| 5 | 时间用 `updated_at_millis: i64` | 不传格式化字符串；客户端 `ChatSession.updatedAt` 是 `DateTime`，映射是一次乘法 |
| 6 | `turn_id` 由**客户端**生成（`AskRequest.turn_id`） | 现状 `ChatTurn.id` 是本地 `local-<micros>`；由客户端生成才能让"提问"与"第一条增量"之间的窗口期也有标识可用 |
| 7 | 新增了旧协议没有的三类消息：工作区、会话、目录 | 界面上已经有了或已预留，缺了它们这次改造就只完成一半 |
| 8 | 业务错误是 `Reply::Error`，传输错误不是协议的一部分 | 与 `abs_kb_svc` README §5 第 7 条一致：两类错误不能混在一个类型里 |

### 5.3 表示形式上的两条硬约束（实测踩出来的）

协议的 serde 写法**不能随便选**：目标传输 `kb_svc_servo_ipc` 用 ipc-channel，
而它 0.23 的内部编解码器是 **postcard**，**不自描述**。实测结果：

| 写法 | JSON | postcard |
| :--- | :--- | :--- |
| 内部标签 `#[serde(tag = "type")]` | ✅ | ❌ 编码"成功"但解码报 "This is a feature that PostCard will never implement" |
| `#[serde(skip_serializing_if = "…")]` | ✅ | ❌ 解码报 `DeserializeUnexpectedEnd` |
| 外部标签（serde 默认）+ 不用 `skip_serializing_if` | ✅ | ✅ |

⇒ 因此：**枚举一律用 serde 默认的外部标签**（`{"Ask": {…}}`），
**不使用 `skip_serializing_if`**（可选字段一律编码为 `Option::None` 分支，
空集合编码为 `[]`），也不要使用 `#[serde(flatten)]`。

这两个坑的共同点是：**只做编码看不出问题**。内部标签编码还会"成功"地产生垃圾字节，
错到解码时才炸。所以 `envelope_.rs` 里那组测试是做**真解码**的，不是只比字节。

> 代价：JSON 调试输出比旧的 `"type":"ask"` 啰嗦。收益：它真的能在目标传输上跑起来。
> 这也顺带说明为什么不能照抄 `kb_svc_salvo::wire` 的写法——那套是给 JSON/WebSocket 用的。

### 5.4 文件拆分原则

`desktop` 从单文件改为目录（`desktop/mod.rs` + 11 个子模块），拆分原则是：

> **只有需要内部共享逻辑与内部私有访问的代码才放在同一个文件里。**

因此：

- 按**数据概念**分文件（标识 / 握手 / 消息内容 / 服务 / 工作区 / 目录 / 错误 / 三个方向的消息 / 信封），
  而不是按"类型 vs 函数"分层；
- 目前只有 [`ids_`](../../kb_svc/crates/abs_kb_svc/src/v1/desktop/ids_.rs) 需要聚合——
  那里的 `string_id!` 宏是**内部共享逻辑**，所有标识类型共用它；
- `mod.rs` **统一控制导出**：子模块是私有的，`pub use` 决定对外名字。
  好处是公开路径始终是 `abs_kb_svc::v1::desktop::<类型名>`，
  以后在文件之间挪动类型不会影响任何使用者。

### 5.5 两条命名 / 导出规则（`AGENTS.md` 第 5 条，2026-09-17 新增）

团队随后给 `AGENTS.md` 第 5 条补了两条硬性要求，`desktop` 已按此调整：

| 规则 | 在本模块的落地 |
| :--- | :--- |
| **非公开导出的 mod 名以 `_` 结尾** | 11 个子模块改为 `content_` / `envelope_` / `error_` / `event_` / `fs_` / `handshake_` / `ids_` / `reply_` / `request_` / `service_` / `workspace_`（文件名同步加 `_`）。`desktop` 本身是公开导出的，保持原名 |
| **`mod.rs` 导出子 mod 类型时禁用通配符 `*`，必须逐个列举** | `pub use content_::{Notice, TokenUsage, ToolCallRecord, Turn, TurnState};` 这样逐项写出 |

第二条的收益不只是"符合规范"：**"对外暴露了什么"在本文件里一眼可数**。
用 `pub use x::*;` 时，往子模块里新增一个 `pub` 类型就会悄悄进入公开 API；
逐项列举则把它变成一个必须显式做的决定，与 `AGENTS.md` 第 1 条
（公开 API 变更须经讨论）是一致的。

`desktop` 的公开路径没有变化（仍是 `abs_kb_svc::v1::desktop::<类型名>`），
`_` 只出现在模块名与文件名上。

---

## 6. 这次改造会波及的现存文件

| 文件 | 预期改动 |
| :--- | :--- |
| `lib/src/services/local_store.dart` | 保留 5 个界面偏好键；移除 `kb.services` / `kb.active_service` / `kb.workspaces` 的读写 |
| `lib/src/state/app_controller.dart` | 服务、工作区、会话的增删改查从"改内存 + 落盘"改为"发请求 + 用应答/事件更新内存" |
| `lib/src/widgets/conversation/conversation_pane.dart` | `_send()` 接 `Ask`；`Composer` 传 `busy` / `onCancel` |
| `lib/src/widgets/settings/settings_dialog.dart` | 明文提示文案（"接入 kb_core 之后会改为由服务端统一保管"）需要更新；key 编辑改用三态语义 |
| `lib/src/widgets/files/workspace_file_panel.dart` | 空状态换成真实的目录列举 |
| `lib/src/widgets/sidebar/workspace_list.dart` | 新增工作区的对话框按 TODO 换成目录选择器 |
| `rust/src/api/simple.rs` | 从示例 `greet` 改为暴露 `abs_kb_svc` + `kb_svc_servo_ipc` 的客户端句柄 |
| `rust/Cargo.toml` | 增加对 `abs_kb_svc` / `kb_svc_servo_ipc` 的依赖（注意：该 crate 是独立 workspace，见其注释） |

---

## 7. 尚未覆盖 / 待决策

1. **知识库检索与编辑**：尚未存在，本轮不定义；将来应是 `v1::desktop` 的增量。
2. **多客户端并发**：同一会话被两个客户端同时提问时如何裁决（现有语义是"同一会话同时只允许一轮"）；
   同一工作区被两台客户端同时改名/删除时的冲突裁决也未定义。
3. **目录浏览的范围**：只列举（本轮）还是要读文件内容 / 上传附件；
   后者需要大载荷路径，与 `kb_svc_servo_ipc` 的 `IpcSharedMemory` 相关。
4. **本地创建对象的回收**：客户端建了 `LocalId` 却始终没同步（或者同步失败）时如何清理；
   以及同一次同步被重发时如何避免重复创建工作区（幂等性由 `LocalId` 承担，需要 kb_core 侧实现）。
5. **批量同步**：本轮是"一个工作区一个请求、一个会话一个请求"，
   离线攒了很多对象时会变成 N 次往返；是否需要一次 `SyncLocalState` 批量提交。
6. **`Conversation` 语义**：旧设计的"对话上下文归 agent 所有"（`dev-notes.md` §2.3）
   与"会话与历史归 `kb_core`"需要对齐——两者都成立，但边界要写清楚
   （agent 持有 LLM 上下文，`kb_core` 持有面向界面的历史）。

**已经关闭的问题**：

- ~~历史消息的归属~~ → 已定：工作区与会话**由 `kb_core` 管理并多端同步**，
  因此历史留在服务端；`GetSession` / `SessionDetail` / `SessionChanged` 保留（见 §5.1）。

---

## 8. 附录：交付物与验证

- **代码**：`kb_svc/crates/abs_kb_svc/src/v1/desktop/`（`mod.rs` + 11 个子模块）
  - 入口文档：[`desktop/mod.rs`](../../kb_svc/crates/abs_kb_svc/src/v1/desktop/mod.rs)
  - 标识与本地先创建：[`ids_.rs`](../../kb_svc/crates/abs_kb_svc/src/v1/desktop/ids_.rs)
  - postcard 兼容性的守门测试：[`envelope_.rs`](../../kb_svc/crates/abs_kb_svc/src/v1/desktop/envelope_.rs)
- **验证**（在真实 workspace 内，无需任何替身）：
  - `CARGO_HOME=$PWD/external/cargo-home cargo test -p abs_kb_svc`
    ⇒ 23 个单元测试 + 9 个文档测试通过
  - `CARGO_HOME=$PWD/external/cargo-home cargo clippy -p abs_kb_svc --all-targets`
    ⇒ 本 crate 源码无告警
  - 其中 `every_sample_message_round_trips_through_postcard_` 与
    `optional_fields_are_always_encoded_` 守住 §5.3 的两条表示约束

### 8.1 验证环境说明

`~/.cargo` 在本机是只读的（见 `dev-notes.md` §6），因此需要一个可写的 `CARGO_HOME`；
本仓库使用 `external/cargo-home/`（根 `.gitignore` 已忽略）。这不是项目配置问题。

> 本记录早先版本曾提到一个"隔离验证工程"（`external/abs_kb_svc-harness/`），
> 那是因为当时 HEAD 上的 `abs_llm` 处于迁移中途、编译不过。
> **`abs_llm` 已修复**，该替身工程已删除，现在直接在工作区内验证。
