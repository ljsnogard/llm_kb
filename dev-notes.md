# llm_kb 开发目标

## 第一个开发目标

1. `kb_svc_salvo`

提供模仿 `deepseek-harness` 界面的应用，同时启动两个 http 服务。
- 第一个是 http 服务是基于 unix domain socket 的，用于和 `kb_rig_llm` 进行 RPC 通信的服务，
  也即用来向 LLM 插件下发指令和接收数据的服务。  
  `kb_rig_llm` 通过类似于 websocket 的长连接，来等待 `kb_svc_salvo` 服务器来下发用户输入的问题。
  `kb_rig_llm` 应该作为一个由 `kb_svc_salvo` 启动的子进程的方式运行，因此在启动的时候就能够知道运行这个服务的 unix domain socket 文件名。

- 第二个是用于和用户直接交互的服务，服务内容包括，接收用户输入的问题并转发给 `kb_rig_llm` 用于向 LLM 发出提问
  然后将 `kb_rig_llm` 收到的回复动态地展示在界面上。也即 `kb_svc_salvo` 需要提供一个极为简单的 html 页面和必要的脚本，
  用来支持动态地显示 `kb_rig_llm` 上传来的 text delta。

---

## 第一个开发目标：可行性与设计方案

> 本节只记录设计与论证。§1–§12 是最初的可行性分析；§13 起记录已经拍板的决策与
> PoC 进展（决策正文以 §13 为准，若与 §1–§12 冲突，以 §13 为准）。

### 1. 需求拆解与可行性结论

把目标拆成 6 个可独立验证的子能力，逐项核对骨架与依赖现状：

| 编号 | 子能力 | 可行性 | 结论与依据 |
| :---: | :--- | :---: | :--- |
| R1 | `kb_svc_salvo` 同时监听两个服务：一个 Unix domain socket、一个给用户用的 HTTP | ✅ 可行 | Salvo 0.96 提供 [`salvo_core::conn::UnixListener`](https://docs.rs/salvo_core/latest/salvo_core/conn/unix/struct.UnixListener.html)（`unix` feature）与 `TcpListener`，二者可用 `JoinedListener::new(a, b)` 合并后交给同一个 `Server::serve()`，共享同一个路由表与状态。 |
| R2 | 与 `kb_rig_llm` 之间的长连接 RPC（类似 websocket） | ✅ 可行 | Salvo 的 [`WebSocketUpgrade::upgrade`](https://docs.rs/salvo_extra/latest/salvo_extra/websocket/struct.WebSocketUpgrade.html)（`websocket` feature）只依赖 HTTP/1.1 的 `Upgrade` 机制，对底层传输类型不敏感，因此在 UDS 连接上同样成立。若不想引入 ws 依赖，可退化为 UDS 上的 JSONL（换行分隔 JSON）长连接，语义等价。 |
| R3 | `kb_rig_llm` 作为 `kb_svc_salvo` 的子进程启动，启动时即知道 socket 路径 | ✅ 可行 | socket 路径由父进程生成（含 pid / 会话 id），通过命令行参数 + 环境变量注入子进程；`std::process::Command` 或 `compio` 的 `process` feature 均可。 |
| R4 | 用户服务接收问题并转发给插件 | ✅ 可行 | 纯应用逻辑：浏览器 → 服务端 →（按 R2 的通道）→ 插件。服务端是唯一的转发中枢，需要一份内存态会话状态机（见 §5）。 |
| R5 | 回复以 text delta 形式动态显示在页面 | ✅ 可行 | `abs_llm::v1::LlmRespEvent::TextDelta` 与 `TrTextDelta`（含 `LogicOutput` 区分 Answer / Reasoning）已经定义好了语义，浏览器侧只需增量追加 + 区分样式。Salvo 亦可选 `sse` feature，但既然 R2 已用 WebSocket，浏览器侧直接复用同一种长连接即可。 |
| R6 | 提供「极为简单」的 HTML 页面和脚本，模仿 `deepseek-harness` 界面 | ✅ 可行（低成本版） | `deepseek-harness`（DSH）是 TypeScript/React 应用，**不能**直接复用其前端；但可以低成本复刻其观感：设计令牌是纯 CSS 变量（`--dsw-alias-*` / `--dsw-static-*`，深色底为 `rgb(21,21,23)`），真实值集中在 `packages/client/ui-theme/src/styles/design-platform.css`。用 `include_str!` 内嵌单页 HTML + 原生 JS 即可，无需前端工具链。 |

**总体结论：第一个开发目标在现有骨架上可行，但骨架近乎为空，实际工作量集中在新建模块与协议约定，而不是攻克某个不确定的技术点。**
R2 与 R6 各有一处需要提前决策的取舍（见 §4、§7.1）。

### 2. 骨架现状盘点

| 位置 | 现状 | 与目标的差距 |
| :--- | :--- | :--- |
| `kb_svc/crates/kb_svc_salvo` | 只有 `Cargo.toml` + 空的 `src/lib.rs`，**零依赖**，且没有 `[[bin]]` | 目标 1 的主战场，几乎全部待写 |
| `kb_svc/crates/abs_kb_svc` | 空 `lib.rs`，依赖 `buffex` / `mm_ptr` 但未使用 | 插件抽象与生命周期协议尚无一行定义 |
| `kb_svc/crates/abs_llm` | 唯一有内容的 crate：`v1/{cont,interact,error}.rs` 定义了 `TrLlmService` / `TrChatRequest` / `TrResponseStream` / `TrTextDelta` / `LogicOutput` / `LlmRespEvent` / `FinishReason` 等 | 抽象可用，但**没有实现者**，也没有「请求从哪来」的建模 |
| `kb_plugins/crates/kb_rig_llm` | 空 `lib.rs`，无依赖，也没有 `[[bin]]` | 需要成为可执行子进程，并实现 `abs_llm::v1` 的某个 provider |
| `kb_svc/crates/kb_core` | 有 `Cargo.toml` 与空 `main.rs`，但**没有出现在根 `Cargo.toml` 的 `members` 中** | 目前不参与工作区构建；README 里把它描述为主进程，与 `kb_svc_salvo` 的定位重叠，定位需澄清 |
| 根 `Cargo.toml` | `compio` 0.19（`fs` / `macros` / `time`）已声明；`tokio` 未声明但已在本地 cargo 缓存中 | 运行时选型必须先定（见 §4） |

`cargo check --workspace --all-targets` 当前通过，仅有一批「未使用的 workspace 依赖 / 字段」警告。

### 3. 总体架构

```
                        ┌───────────────────────── kb_svc_salvo（主进程）──────────────────────────┐
                        │                                                                          │
  浏览器 (Web UI)       │   TcpListener（127.0.0.1:PORT）        UnixListener（$XDG_RUNTIME_DIR/…sock） │
  ┌───────────┐  ws     │   ┌───────────────────────┐            ┌───────────────────────────┐      │
  │ index.html│◄───────►│   │ GET /            静态页│            │ WS /plugin  （长连接）     │      │
  │ app.js    │         │   │ GET /assets/*     资源 │            │   ↑ 下行：问题 / 取消      │      │
  └───────────┘         │   │ WS /chat          推送 │            │   ↓ 上行：delta / 事件     │      │
                        │   └──────────┬────────────┘            └──────────┬────────────────┘      │
                        │              │        SessionHub（内存态会话）    │                           │
                        │              └────────────► ◄────────────────────┘                           │
                        │                        PluginSupervisor（子进程生命周期 / 自动重启）         │
                        └───────────────────────────────────┬──────────────────────────────────────────┘
                                                            │ spawn(child) + 注入 socket 路径
                                                            ▼
                                            ┌──────────────────────────────┐
                                            │ kb_rig_llm（子进程）          │
                                            │  UDS 客户端 + LLM provider    │
                                            │  （实现 abs_llm::v1 抽象）     │
                                            └──────────────────────────────┘
```

要点：

1. **单进程、双监听器**。`kb_svc_salvo` 同时 bind TCP 与 UDS，共享同一个 `Router` 与共享状态（`Arc<AppState>`）。不是两个进程、也不是两个 `Server`。
2. **服务端是唯一中枢**。插件只与 `kb_svc_salvo` 通信，浏览器也只与 `kb_svc_salvo` 通信；插件永远不知道浏览器存在。
3. **子进程由服务端拉起**。因此 UDS 路径可以在 spawn 之前确定并注入。
4. `Store` / `AffixState` 之类由 Salvo 提供的注入机制承载 `Arc<AppState>`。

### 4. 关键决策：异步运行时（必须先定）

这是本目标唯一的架构级风险：**Salvo 0.96 强依赖 tokio**（`salvo_core` 直接依赖 `tokio` / `hyper` / `hyper-util`），而根 `Cargo.toml` 目前选的是 **compio**（io_uring 风格的 completion-based 运行时）。两者不能在同一线程上互跑，但可以共存于同一进程的不同线程/运行时。

候选方案：

| 方案 | 说明 | 评价 |
| :--- | :--- | :--- |
| **A. 主运行时改用 tokio** | `kb_svc_salvo` 与 `kb_rig_llm` 都用 tokio；compio 暂时保留在依赖表中但不使用 | 摩擦最小，与 Salvo 生态一致；缺点是暂时搁置 compio 的零拷贝 I/O 优势 |
| **B. `compio::main` + `compio::runtime::Runtime::run_in_executor` 内嵌单线程 tokio** | 入口仍是 compio，Salvo 跑在专用 tokio 线程 | 两个运行时都要维护，`AppState` 需是 `Send + Sync` 且跨运行时同步只能用 `std` 或 tokio 原语；复杂度高 |
| C. 不用 Salvo，基于 hyper + 自写 UDS accepter | 完全自主，理论可行 | 放弃已选定的框架，工作量与维护成本最高，不建议 |

**建议采用 A**，并在方案 B 上保留后路（若后续确需 compio 的 io_uring 能力，再以 `run_in_executor` 桥接）。

> ✅ **已决策（见 §13.1）**：采用方案 A，主运行时定为 **tokio**。`kb_svc_salvo`
> 通过 Salvo 间接依赖 tokio，不再把 compio 作为主运行时；工作区依赖中保留 compio
> 供将来按需使用。

### 5. 模块设计（`kb_svc_salvo`）

建议拆成下列模块（均为新增，**待批准**）：

```
kb_svc_salvo/src/
  lib.rs        // 公开组装入口：SalvoConfig、build_router()、run()
  main.rs       // [[bin]]：CLI/GUI 启动、日志、优雅退出
  config.rs     // CLI 参数 / 环境变量 / 默认路径（socket 路径、监听地址、插件可执行文件）
  state.rs      // AppState：SessionHub + PluginSupervisor 的共享句柄
  web/          // 用户侧服务（TCP listener）
    mod.rs      // GET /、GET /assets/*、WS /chat
    assets/     // include_str! 内嵌的 index.html / app.js / app.css
  plugin/       // 插件侧服务（UDS listener）
    mod.rs      // WS /plugin 握手、注册、心跳
  session/      // 内存态会话状态机
    mod.rs      // SessionHub、TurnId、TurnState、broker（tokio::sync::broadcast）
  supervisor.rs // 子进程 spawn / 退出监测 / 退避重启 / socket 文件清理
  wire.rs       // 线协议 JSON 类型（插件↔服务端、服务端↔浏览器）
```

**会话状态机（`session`）** 的最小形态：

- `SessionHub` 持有当前 `Turn`（单会话 MVP：一个 `current_turn: Option<Turn>`）。
- `Turn` 具有 `id`、`question`、`state ∈ {Pending, Streaming, Finished, Cancelled, Failed}`。
- 浏览器侧用 `tokio::sync::broadcast` 订阅下行事件，这样多个标签页能同时看到同一轮输出；必须显式处理 `RecvError::Lagged`（重放快照或提示重连）。
- 插件断线时，处于 `Streaming` 的轮次应被判定为失败或中断，并向浏览器补发 `Finished`/`Error`。

**`PluginSupervisor`** 的职责：

- 生成 socket 路径（建议 `$XDG_RUNTIME_DIR/llm_kb/plugin-<pid>.sock`，回退到 `std::env::temp_dir()`），确保父目录存在并设 `0600`；
- spawn 子进程，注入 `--socket <path>` 与 `LLM_KB_PLUGIN_SOCKET`；
- 监听子进程退出（tokio `Child::wait`），按指数退避重启，并把重启原因推给 UI；
- 退出时 kill 子进程并删除 socket 文件（含 panic 场景的兜底清理）。

### 6. 线协议设计（两段，均待批准）

**A. 插件 ↔ 服务端（UDS 长连接）**：换行分隔 JSON（JSONL），每行一个完整帧。选择它而非 WebSocket 的理由是调试成本几乎为零（`socat - UNIX-CONNECT:…` 即可手测），且语义上不需要 ws 的分片/掩码/ping。

下行（服务端 → 插件）：

| `type` | 字段 | 对应 `abs_llm::v1` |
| :--- | :--- | :--- |
| `hello_ack` | `session`, `server_version` | — |
| `ask` | `turn_id`, `question` | 构造一次 `TrChatRequest` |
| `cancel` | `turn_id` | 触发取消令牌（见 §11） |
| `ping` | `ts` | — |

上行（插件 → 服务端）：

| `type` | 字段 | 对应 `abs_llm::v1` |
| :--- | :--- | :--- |
| `hello` | `plugin_version`, `provider` | — |
| `started` | `turn_id`, `model_id`, `capabilities` | `TrLlmService::capabilities` + `TrChatRequest::model_id` |
| `delta` | `turn_id`, `kind ∈ {answer, reasoning}`, `text` | `LlmRespEvent::TextDelta` / `LogicOutput` |
| `tool_call` | `turn_id`, `id`, `name`, `arguments` | `LlmRespEvent::ToolCall` / `TrToolCall` |
| `usage` | `turn_id`, `input`, `output`, `total` | `TrUsage` |
| `finished` | `turn_id`, `reason ∈ {completed, max_tokens, cancelled, tool_call, other}` | `LlmRespEvent::Finished` / `FinishReason` |
| `error` | `turn_id?`, `class`, `message` | `LlmError` 的类别映射 |
| `pong` | `ts` | — |

**B. 浏览器 ↔ 服务端（WebSocket `/chat`）**：同一套上行/下行事件的裁剪版（`started`/`delta`/`tool_call`/`usage`/`finished`/`error`），另加 `ask` 与 `cancel`。上行/下行共用 `turn_id`，为将来多会话留出标识位。

字段集合与 `wire.rs` 严格一一对应，避免出现「协议里有一套、内存里又有一套」。

### 7. 前端设计（模仿 DSH 观感的低成本做法）

先明确「模仿 `deepseek-harness` 界面」的三种可能解读，它们的成本相差一个数量级，需要先选定：

> ✅ **已决策（见 §13.2）**：采用 **7.1-A 观感复刻**——手写 HTML + 原生 JS + DSH
> 风格的 CSS 令牌，不引入 React / Vite / node 工具链。下表的 7.1-B「复用 DSH 构建产物」
> 仅作为将来可选方向保留，不在本期范围内。

| 方案 | 做法 | 成本 | 评价 |
| :--- | :--- | :--- | :--- |
| **7.1-A 观感复刻（建议）** | 手写单页 HTML + 原生 JS，复用 DSH 的设计令牌与字体栈 | 1–2 天 | 与目标里「极为简单的 html 页面和必要的脚本」一致；无前端工具链，无 node 依赖 |
| 7.1-B 复用 DSH 前端构建产物 | 构建 `apps/web`（Vite + React），由 `kb_svc_salvo` 托管其 `dist/` | 数天 + 引入 pnpm/node 工具链 | 观感 100% 一致，但需要 DSH 的 `window.__DSH_BOOT__` 引导协议、RPC 契约与 Cordis 插件体系，通信层要整体对齐，成本与耦合度最高 |
| 7.1-C 自建 React 前端 | 另起一套前端工程 | 数天 | 观感要靠自己对齐，收益不明显 |

若后续确实要做 7.1-B，需要先把 DSH 的引导协议（`window.__DSH_BOOT__`）与 `/api/remote.mux` 的帧格式摸清，那属于独立议题，不应混在目标 1 里。

**（以下按 7.1-A 展开）**

目标：**零构建工具链**，`include_str!` 内嵌静态资源，`GET /` 直接返回 `text/html; charset=utf-8`。

- **观感对齐 DSH**：定义同名风格的 CSS 变量（`--dsw-alias-bg-base`、`--dsw-alias-label-primary`、`--dsw-alias-label-caption` 等），深色底 `rgb(21,21,23)`（DSH 的 `--dsw-static-neutral-bluish-950`），浅色底 `#fff`；用 `<html data-theme="dark|light">` 切换。字体栈沿用 DSH 的 `body` 字体族（含 `PingFang SC` / `Microsoft YaHei`）。
- **布局**：顶部会话标题条 + 中部消息流（用户气泡 / 助手块）+ 底部输入区（`textarea` + 发送/取消按钮）。与 DSH 的消息流-输入框两段式结构一致，但去掉侧边栏、设置面板、工具卡片等。
- **增量渲染**：`delta` 帧到达后写入缓冲，用 `requestAnimationFrame` 合并成每帧一次 DOM 写入，避免逐 token 触发布局；流式过程中显示光标块，`finished` 后移除。
- **Reasoning 与 Answer 分开**：`kind: reasoning` 渲染为可折叠的 `<details>`（对应 DSH 的 `ReasoningRow`，运行中带扫光动画），`kind: answer` 渲染为正文。
- **滚动策略**：仅在用户已贴近底部时自动跟随；用户上滚则暂停跟随并显示「回到底部」。
- **渲染器（待补充）**：MVP 先按纯文本 + 简单代码块（三反引号）处理；Markdown 完整渲染与语法高亮列为后续项，避免第一期引入 JS 依赖。

### 8. 配置与启动约定（待批准）

| 项 | 建议默认值 |
| :--- | :--- |
| 用户侧监听 | `127.0.0.1:8788`（仅本机回环） |
| 插件侧 socket **目录** | `$XDG_RUNTIME_DIR/llm_kb`（缺省回退到系统临时目录下的 `llm_kb`），权限 0700 |
| 插件侧 socket **文件名** | ✅ 已定：不接受参数/环境变量指定，每次启动由「日期 + UUID v4」生成，形如 `kb-20260913-f9c628f955594c1fa73965bcac42f091.sock`，权限 0600（§13.6） |
| 插件可执行文件 | 依次尝试：CLI 参数 `--plugin-bin` → 环境变量 `LLM_KB_PLUGIN_BIN` → 与 `current_exe()` 同目录的 `kb_rig_llm` |
| socket 路径下发方式 | 启动插件子进程时通过 `--socket <path>` 与 `LLM_KB_PLUGIN_SOCKET` 两种方式告知（§13.6） |
| 静态资源 | 编译期 `include_str!` 内嵌；仅在开发期保留 `--assets-dir` 覆盖选项 |
| 日志 | `env_logger` + `log`（已在工作区依赖中） |

### 9. 第一个开发目标的范围与验收标准

**范围收窄（MVP）**：单用户、单会话、单插件进程、纯内存态、无持久化、无鉴权、纯文本（含 reasoning）。

**验收标准**：

1. `kb_svc_salvo` 启动后同时监听 TCP 与 UDS，`socat` 可连上 UDS。
2. `kb_rig_llm` 由服务端自动拉起，父子进程关系正确。
3. 浏览器提交一个问题，回答以 delta 分批出现，而非整段一次性出现。
4. reasoning 与 answer 在界面上样式可区分。
5. 点击取消能在 1 秒内向插件下发 `cancel` 并停止输出。
6. 手工 kill 插件进程后，服务端按退避重启，并在界面给出提示。
7. 服务端退出后，socket 文件与子进程都被清理。

### 10. 目前还缺少的内容

**必须先澄清的设计决策**

1. ~~**主运行时选 tokio 还是 compio**~~ → ✅ 已决策为 tokio（§13.1）。
2. ~~**`kb_core` 与 `kb_svc_salvo` 的分工**~~ → ✅ 已决策：`kb_core` 承载二进制与启动编排，`kb_svc_salvo` 保持为库，现阶段视为一体（§13.1）。
3. **`abs_kb_svc` 的定位**：目标 1 的 UDS 协议应定义在 `abs_kb_svc`（抽象层）还是 `kb_svc_salvo`（实现层）？若定义在抽象层，`kb_rig_llm` 便不依赖具体服务实现，但抽象层会引入线协议依赖（serde 等）。
4. **对话上下文的归属**：`TrConversation` 由谁来持有与推进——服务端保存历史，还是每轮把上下文整体下发给插件？
5. **多会话**：MVP 单会话是否接受？协议里的 `turn_id` 是否需要升级为 `session_id` + `turn_id`？
6. ~~**流式通道**~~ → ✅ 已决策：浏览器侧走 HTTP/1.1 + WebSocket（§13.4）。
7. ~~**插件 socket 文件名从哪来**~~ → ✅ 已决策：不接受参数/环境变量指定，每次启动按「日期 + UUID」生成，并在启动子进程时下发（§13.6）。
8. **rig 适配 crate 的命名与导出**：`kb_plugins/crates/` 下新增的适配 crate 叫什么？它导出的是「rig 类型 → `abs_llm::v1` 实现」的转换器，还是直接导出实现了 `TrLlmService` 的 provider？（见 §13.3）
9. **rig 数据原样透传的边界**：`abs_llm` 抽象与 rig 类型之间必然存在无法一一对应的部分（例如 rig 的 reasoning / tool-call 细节），需要明确「原样透传」落到 `wire` 协议时是保留 rig 的原始 JSON 还是先做最小投影。

**待新增的依赖（公开约定，需批准）**

10. **`kb_svc_salvo` 当前甚至没有声明 `salvo` 依赖**，只声明了 `edition` / `name` / `version` / `description`；`kb_rig_llm` 同理。这是第一个要补的洞（PoC 已补上一部分，见 §13.5）。
11. 建议把 `salvo` 固定到 `0.96.*`（当前最新，且 `unix` / `websocket` / `quinn` / `serve-static` / `sse` 均齐备）；Salvo 破坏性变更频繁，浮动的版本范围会带来不可预期的迁移成本。
12. `compio` 需要开启 `net` / `process` feature；主运行时为 tokio 时则不启用。
13. Salvo feature：`unix`（UDS listener）、`websocket`、`serve-static`（若不做 `include_str!` 内嵌）、`quinn`（HTTP/3，见下）、`test`。
14. `serde` + `serde_json`（线协议）；`tokio` 的 `sync`（broadcast/mpsc）；错误处理 crate（`thiserror` 已在工作区依赖表中但无人使用）。
15. rig 及其适配 crate 的依赖（版本、feature、是否需要 provider 特化）尚未确定（§13.3）。

**待补充的工程实现**

16. `kb_core` 需要补上正式启动编排：配置解析、日志初始化、`ServerHandle` 优雅退出；PoC 版本只做了最小实现。
17. `kb_rig_llm` 需要 `[[bin]]`，以及一个 `abs_llm::v1::TrLlmService` 的真实实现（首个 provider），包括 rig 调用、增量到 `LlmRespEvent` 的映射。
18. 新增的 rig 适配 crate（`kb_plugins/crates/` 下）尚无一行代码（§13.3）。
19. 错误类型：需要为「服务端错误」「子进程错误」「线协议错误」定义实现 `std::error::Error` 的类型（`AGENTS.md` 第 9 条）；PoC 已定义 `KbSvcError` 作为起点。
20. 配置解析：CLI 参数/环境变量的具体名称与优先级尚未确定。
21. 静态资源目录（`web/assets/`）、`index.html`、`app.js`、`app.css` 全部待创建。
22. 测试：`abs_llm` 无测试；`AGENTS.md` 要求单测（中文文档注释）+ `tests_/` 集成测试 + 关键路径基准。PoC 已为 UDS + WebSocket 补了集成测试（§13.5），其余（线协议、会话状态机、子进程重启）待补。
23. 日志与可观测性：`log` / `tracing` 的选型（Salvo 内部用 `tracing`，与 `log` 的桥接需要确认）。
24. 进程健壮性细节：僵尸进程回收、socket 文件残留（PoC 已有最小处理）、同一路径重复 bind、优雅退出（`ServerHandle::stop_graceful`）。

**已知风险**

25. **HTTP/3 的现实成本**：README 提到用 HTTP/3 作为第一版通信方式，但 quinn 系 HTTP/3 需要 TLS 证书，即使是 localhost 也要自签与管理信任，对「极简 web 界面」是负收益。已决策第一期走 HTTP/1.1 + WebSocket（§13.4），HTTP/3 延后。
26. ~~**WebSocket over UDS 未在本仓库验证过**~~ → ✅ PoC 已验证可行（§13.5）。
27. **两个运行时的维护成本**：已选 tokio（§13.1），此项风险解除；但若将来重新引入 compio，仍需面对 `Send + Sync` 与跨运行时唤醒问题。
28. **rig 与 `abs_llm` 的语义落差**（新增）：rig 的流式事件类型与 `abs_llm::v1` 的 `LlmRespEvent` 并非一一对应，适配层的映射规则、以及无法映射字段的取舍，需要专门设计与测试。

### 11. 实现顺序与工作量估算

| 阶段 | 内容 | 状态 |
| :---: | :--- | :--- |
| 0 | 确认运行时选型与公开 API 变更；写 UDS + WebSocket 最小 PoC | ✅ 已完成（§13） |
| 1 | `wire.rs` 协议类型 + 编解码 + 单元测试 | ✅ 已完成（§14） |
| 2 | `kb_svc_salvo`：双监听器、`AppState`、聊天与插件 handler | ✅ 已完成（§14） |
| 3 | 前端页面：布局、样式令牌、增量渲染、取消、重连 | ✅ 已完成（§14） |
| 3.5 | 用户配置：LLM 服务选项与 API key 的读写 + 界面配置面板 | ✅ 已完成（§14） |
| 4 | `PluginSupervisor`：spawn / 重启 / 清理 | ⏳ 0.5 天 |
| 5 | `kb_rig_llm` + rig 适配 crate：UDS 客户端 + `abs_llm::v1` 实现 | ⏳ 2 天 |
| 6 | 集成测试与验收 7 条 | ⏳ 1 天 |
| — | **合计** | **约 3.5 天**（剩余） |

### 12. 需要批准的事项清单

按 `AGENTS.md` 第 1 条，以下均为对使用者公开的 API / 接口 / 约定，实施前需要明确同意。
其中第 1、2 条已在 §13 的决策中批准，PoC 已按此落地：

1. ✅ 根 `Cargo.toml` 新增 workspace 依赖（`salvo` / `tokio` / `uuid`）与 `members` 增加 `kb_core`。
2. ✅ `kb_core` 新增二进制目标；`kb_svc_salvo` 保持为库（不新增 `[[bin]]`）。
3. §6 的两段线协议帧格式与字段名。
4. 各 crate 新导出的公开类型与模块路径（`kb_svc_salvo::{poc::*, error::*, plugin_socket::*}`；
   其中 `poc` 为 PoC 临时导出，`plugin_socket` 为正式内容）。
5. ✅ socket 文件名约定已定：不接受 CLI / 环境变量指定，按「日期 + UUID」生成（§13.6）；
   仍需确认：`--socket` 与 `LLM_KB_PLUGIN_SOCKET` 这两个**下发**通道的名称，以及 `--runtime-dir`。
6. ◑ socket 权限（目录 0700 / 文件 0600）已定；默认监听地址 `127.0.0.1:8788` 待确认。
7. ❓ HTTP 路由表（`/`、`/assets/*`、`/chat`、`/plugin`）。
8. ❓ rig 适配 crate 的 crate 名、导出类型与依赖版本（§13.3）。

---

## 13. 已确认的决策与 PoC 进展

### 13.1 进程与 crate 分工（决策）

- `kb_svc_salvo` **保持为库**，不提供二进制目标，不再是 `[[bin]]`。
- 启动 `kb_svc_salvo` 的工作放在 **`kb_core`**：`kb_core` 承载二进制、配置解析、
  日志初始化、插件子进程监管与优雅退出。
- `kb_core` 已加入 workspace `members`。
- **现阶段把 `kb_core` 与 `kb_svc_salvo` 视为一体**，以后再考虑把代码拆开；
  因此两者之间允许直接依赖（`kb_core` 依赖 `kb_svc_salvo`），但仍应避免把
  `kb_core` 的编排逻辑反向塞进库内部。
- 主运行时定为 **tokio**（§4 方案 A）。`kb_svc_salvo` 通过 Salvo 间接依赖 tokio，
  并直接声明 `tokio` 以使用 `#[tokio::main]` / `#[tokio::test]`。

### 13.2 前端（决策）

- 只做 **观感复刻**：手写 HTML + 原生 JS + DSH 风格的 CSS 令牌，
  **不引入 React / Vite / node 工具链**。
- 设计令牌参考 DSH 的 `packages/client/ui-theme/src/styles/design-platform.css`
  （`--dsw-static-*` → `--dsw-alias-*` 两级），深色底 `rgb(21,21,23)`，浅色底 `#fff`。

### 13.3 rig 的接入方式（决策）

- **直接使用 rig 的代码**（rig 负责与 LLM provider 通信、产生流式输出）。
- rig 收到的内容**原样发送**到 `kb_core`（经 UDS 通道），不在插件进程内做语义转换。
- 在 `kb_core` 内部再把 rig 的数据类型转化为符合 `abs_llm` 的实现。
- 为此在 `kb_plugins/crates/` 下**新增一个 crate**，专门实现
  「rig 数据类型 → `abs_llm::v1` 实现」的转换。
- 这带来一条新的依赖方向：适配 crate 同时依赖 `rig` 与 `abs_llm`，
  而 `kb_rig_llm` 依赖该适配 crate；`kb_core` 侧消费 `abs_llm` 抽象，不直接依赖 rig。
- 待定：适配 crate 的具体名称、导出的公开类型（转换器 vs 完整 provider）、
  以及「原样透传」在线协议里的表示（原始 rig JSON 保留多少）。

### 13.4 传输（决策）

- 第一期浏览器侧走 **HTTP/1.1 + WebSocket**，不做 HTTP/3。
- HTTP/3（quinn）延后；理由见 §10 风险点 25（TLS 证书成本对 localhost 是负收益）。

### 13.5 PoC：UDS + WebSocket（已完成，验证通过）

**验证目标**：`dev-notes.md` §10 风险点 26 —— 「Salvo 的 WebSocket 升级能否在
Unix domain socket 上工作」。这是第一期唯一无法靠读代码确定的技术点。

**落地代码**（临时性，PoC 通过后并入正式模块）：

| 路径 | 作用 |
| :--- | :--- |
| `kb_svc_salvo/src/poc.rs` | PoC 服务端：`UnixListener` + `TcpListener` 经 `JoinedListener` 合并，路由表共享；`GET /` 返回文本，`GET /ws` 升级 WebSocket 并回显。`bind()` 返回 `BoundPoc`（含 acceptor、TCP 地址、生成的 socket 路径） |
| `kb_svc_salvo/src/plugin_socket.rs` | socket 路径生成（日期 + UUID）、目录/文件权限、清理守卫（**正式内容**，非临时） |
| `kb_svc_salvo/src/error.rs` | `KbSvcError`（实现 `Debug` / `Display` / `std::error::Error`，含 `source()`） |
| `kb_svc_salvo/tests/poc_uds_websocket.rs` | 集成测试：路径生成约定、UDS 与 TCP 两条路径的握手 + 双向收发、清理守卫 |
| `kb_core/src/main.rs` | 二进制入口，拉起 PoC 服务端（`kb_core [--runtime-dir <dir>] [tcp_addr]`） |
| 根 `Cargo.toml` | 新增 workspace 依赖 `salvo`（`0.96.*`，features：`server`/`server-handle`/`http1`/`http2`/`http2-cleartext`/`unix`/`websocket`/`test`/`logging`）、`tokio`、`uuid`；`members` 增加 `kb_core` |

**验证内容与实测结果**（`cargo test -p kb_svc_salvo`，10 项全部通过）：

| 测试 / 检查 | 断言 | 结果 |
| :--- | :--- | :---: |
| `poc_socket_path_is_generated_with_date_and_uuid`（集成） | 生成路径位于指定运行时目录；文件名匹配 `kb-<YYYYMMDD>-<32 位十六进制>.sock`；文件权限恰为 `0600` | ✅ |
| `poc_ws_over_unix_socket`（集成） | UDS 上握手返回 `101 Switching Protocols`；两轮文本回显（含中文）逐字节一致 | ✅ |
| `poc_ws_and_http_over_tcp`（集成） | TCP 上 WebSocket 回显一致；`GET /` 返回 200 且响应体含 `poc` | ✅ |
| `socket_file_guard_removes_file_on_drop`（集成） | 清理守卫析构后 socket 文件消失 | ✅ |
| `generate_path_embeds_date_and_uuid`、`generate_path_is_unique_per_call`（单元） | 文件名含日期与 UUID；连续 128 次生成无重复 | ✅ |
| `civil_from_days_matches_known_dates`、`today_utc_compact_matches_date_command`（单元） | 公历换算与已知日期一致；生成的日期与 `date -u +%Y%m%d` 逐字节相等 | ✅ |
| `remove_socket_file_is_idempotent`（单元） | 重复删除不存在的文件仍返回 `Ok` | ✅ |
| `cargo test --doc` | `poc` 模块文档示例可编译 | ✅ |
| `cargo clippy -p kb_svc_salvo -p kb_core --all-targets` | 无 clippy 警告（仅剩工作区既有的「未使用 workspace 依赖」与 `kb_core` 二进制名非 kebab-case 提示） | ✅ |

覆盖的四个验证点：

1. UDS 上完成 HTTP/1.1 `Upgrade` 握手，返回 `101 Switching Protocols`；
2. UDS 上 WebSocket 双向收发（客户端 → 服务端 → 客户端）文本逐字节一致；
3. TCP 侧 WebSocket 同样可用；
4. 两个监听器共享同一份 `Router`（`GET /` 在 TCP 上可达）。

**手工实测**（真机跑 `kb_core` 二进制）：

1. 运行时目录被创建为 `drwx------`，其中出现
   `srw------- kb-20260913-f9c628f955594c1fa73965bcac42f091.sock`；
2. `curl -i http://127.0.0.1:8799/` 返回 `200 OK` + 一行说明文本；
3. 用 Python 标准库对该 socket 发 `GET /`，响应与 TCP 侧一致；
4. `timeout -s TERM 3 kb_core …` 的日志依次出现「收到 SIGTERM，开始优雅退出」→
   「已清理 socket 文件」，退出后运行时目录为空。

**关于优雅退出的一点实现说明**：Salvo 的 `Server::serve` 会消费 `Server` 且不提供
「外部关停句柄」的简单入口，因此 `kb_core` 的做法是：先把 `SocketFileGuard` 用
`BoundPoc::take_socket_guard()` 取到 `main` 的作用域，再把 `BoundPoc` 交给后台任务服务。
这样即使任务在关停时被 `abort`，socket 文件清理也不会被连带取消；
服务端本身则有 3 秒的退出等待窗口。

**手工复现方式**：

```bash
CARGO_HOME=/tmp/cargo-home cargo run -p kb_core -- --runtime-dir /tmp/kb-demo-rt 127.0.0.1:8788
# 另一个终端（TCP 侧）：
curl -s http://127.0.0.1:8788/
ls -la /tmp/kb-demo-rt/     # 观察生成出来的 socket 文件名
# 退出：Ctrl-C，或 kill -TERM <pid>；随后该目录应为空
# 更详细的操作手册见 kb_svc/crates/kb_core/README.md
```

**结论**：§10 风险点 26（原编号 21）判定为**不存在**——WebSocket 升级与传输类型无关，
UDS 与 TCP 可以共用同一套 Salvo 路由与会话逻辑，因此 §6 的 JSONL 退化方案**不需要启用**。

**PoC 遗留问题（转入正式实现处理）**：

1. `poc.rs` 是临时模块，正式实现时应替换为 `server` / `web` / `plugin` 三块并删除；
2. `kb_core` 尚无配置文件与环境变量支持（仅 `--runtime-dir` / `tcp_addr` 两个参数）；
3. 尚未接入会话状态机、子进程监管与真实数据通道；
4. 新增的 `dev-dependencies`（`tokio-tungstenite` 0.28 + `futures-util`）只服务于 PoC 测试，
   正式实现时应迁到「协议层测试」或改为自研最小 ws 客户端，避免测试依赖与运行时依赖混用；
5. 服务端的关停目前依赖「等待 3 秒或 abort 任务」，尚未使用 Salvo 的
   `ServerHandle::stop_graceful`（需要在 `serve` 之前持有 handle，正式实现时补上）。

### 13.6 插件 socket 文件名约定（决策 + 已实现）

**决策**：socket **文件名不接受命令行参数或环境变量指定**，而是每次启动时
拼接「日期 + UUID」生成；启动插件子进程时再把该名字告诉插件。

**实现**（`kb_svc_salvo::plugin_socket`，正式内容）：

| 项 | 约定 |
| :--- | :--- |
| 文件名 | `kb-<YYYYMMDD>-<UUID v4 simple，32 位十六进制>.sock`，例如 `kb-20260913-f9c628f955594c1fa73965bcac42f091.sock` |
| 日期来源 | 当前 **UTC** 日期，用纯整数公历换算（Howard Hinnant `civil_from_days`）实现，不引入 `chrono` / `time` |
| UUID | `uuid` crate 的 v4（`fast-rng`），`simple()` 形式去连字符 |
| 目录 | `$XDG_RUNTIME_DIR/llm_kb`，缺失时回退到系统临时目录下的 `llm_kb`；可用 `--runtime-dir` 覆盖**目录** |
| 权限 | 目录 `0700`（仅在新建时设置），socket 文件 `0600` |
| 清理 | `SocketFileGuard` 在析构时删除 socket 文件；清理函数幂等 |
| 下发 | `kb_core` 通过 `--socket <path>` 与 `LLM_KB_PLUGIN_SOCKET` 两种方式传给子进程（阶段 3 实现） |

**为什么不放在命令行**：

1. 文件名不是配置项，而是「本次运行的一次性标识」，暴露给调用方只会制造
   「两个进程对同一个路径理解不一致」的可能；
2. 日期 + UUID 让每次启动天然互不冲突，历史遗留的 socket 文件不会导致 bind 失败；
3. 日期前缀便于排查「这个 socket 是哪天哪个进程留下的」。

**待确认的公开约定**：下发通道的名字（`--socket` / `LLM_KB_PLUGIN_SOCKET`）与
`--runtime-dir` 是否保留。按 `AGENTS.md` 第 1 条，这些名称需要团队确认后才能固化。

### 13.7 环境提示：本机 cargo 缓存只读

本机 `~/.cargo/registry` 与 `~/.cargo/git` 处于只读文件系统，`cargo fetch` 无法写入
缓存，会以 `Read-only file system (os error 30)` 失败。验证时采用的绕行方式是：

```bash
cp -r ~/.cargo/{registry,git} /tmp/cargo-home/   # 复制到可写的 CARGO_HOME
cp ~/.cargo/config.toml /tmp/cargo-home/
CARGO_HOME=/tmp/cargo-home cargo test -p kb_svc_salvo
```

这属于本机沙箱限制，不是项目配置问题；正常开发机上无需这样处理。

---

## 14. Web 界面与用户配置（已实现）

对应目标里的第二件事：`kb_svc_salvo` 提供一个「极为简单的 html 页面和必要的脚本」，
用来动态显示 `kb_rig_llm` 上传来的 text delta；同时 `kb_core` 增加用户可配置的
LLM 服务选项与 API key。

### 14.1 目录与模块落位

前端相关的一切都在 `kb_svc_salvo` 目录内：

```text
kb_svc/crates/kb_svc_salvo/
  src/
    assets.rs         前端资源的内嵌（include_str!）与开发期覆盖
    web.rs            浏览器侧路由：静态资源、设置 API、聊天 WS
    web_ws.rs         浏览器连接的事件循环
    plugin.rs         插件侧路由：/ws/plugin 双向转发
    hub.rs            会话中枢：转发、turn 状态、广播
    wire.rs           两段线协议的 JSON 类型
    settings.rs       LLM 服务选项与 API key 的读写与持久化
    server.rs         路由表 + 双监听器 + 状态注入
    poc.rs            临时：PoC 证据留存
    web/assets/
      index.html      聊天界面（仿 DSH 观感）
      app.css         样式表（DSH 令牌体系 + 深浅色）
      app.js          原生 JS：WebSocket、增量渲染、设置面板
```

`kb_core` 只增加「编排」职责：读取配置文件、组装服务端、启动时把资源目录与
状态注入进去。

### 14.2 路由表（公开约定）

| 路由 | 监听器 | 说明 |
| :--- | :--- | :--- |
| `GET /` | TCP | 聊天界面 |
| `GET /app.css`、`GET /app.js` | TCP | 前端资源（`text/css` / `text/javascript`） |
| `GET /api/settings` | TCP | 读取服务列表与当前生效服务；API key 遮蔽为 `••••••••` |
| `POST /api/settings/services` | TCP | 新增或覆盖一个服务；回传遮蔽值时保留原 key |
| `DELETE /api/settings/services/{id}` | TCP | 删除服务；若删的是生效服务则自动切换 |
| `POST /api/settings/active` | TCP | 切换当前生效服务 |
| `GET /ws/chat` | TCP | 浏览器事件通道 |
| `GET /ws/plugin` | UDS（当前 TCP 也可达） | 插件通道 |

**两个监听器共用同一份路由表**（§13.5 的结论），因此上表在所有监听器上都可达。
这在本机单用户的阶段假设下是可接受的，正式版本需要按监听器区分权限。

### 14.3 线协议（公开约定）

JSON 文本帧，字段与 `abs_llm::v1` 的词汇对齐：

- 浏览器 → 服务端：`ask{turn_id?,question,service_id?}`、`cancel{turn_id}`、`use_service{service_id}`；
- 服务端 → 浏览器：`ready{plugin_online,services,active_service,server_version}`、
  `started{turn_id,service_id,model}`、`delta{turn_id,kind,text}`（`kind ∈ answer|reasoning`）、
  `tool_call{turn_id,id,name,arguments}`、`usage{turn_id,input_tokens?,output_tokens?,total_tokens?}`、
  `finished{turn_id,reason}`、`error{turn_id?,code,message}`；
- 服务端 → 插件：`hello`、`ask{turn_id,service_id,service,question}`、`cancel{turn_id}`、`ping`；
- 插件 → 服务端：`hello`、`started`、`delta`、`tool_call`、`usage`、`finished`、`error`、`pong`。

`delta` 的 `kind` 直接对应 `abs_llm::v1::LogicOutput` 的应用侧子集，
`finished.reason` 对应 `FinishReason`，`usage` 的字段对应 `TrUsage`。

**服务端不做语义翻译**：插件的 `service` 字段是完整配置（含 API key），
由插件侧决定怎么用；服务端只负责路由与状态。

### 14.4 用户配置（公开约定）

- 路径：`$XDG_CONFIG_HOME/llm_kb/config.toml`（回退 `~/.config/llm_kb/config.toml`），
  可用 `kb_core --config <file>` 覆盖；不存在时自动生成带注释的模板。
- 格式：每个 `[services.<id>]` 一份配置，含 `provider` / `model` / `base_url` / `api_key`。
- 写回用 `toml_edit` **逐键修改**，保留用户写的注释（有集成测试守住这一点）。
- **API key 是明文存储的**（当前阶段的安全取舍）；界面上回传时遮蔽，但那只是防肩窥。

### 14.5 前端实现要点

- **零构建工具链**：`include_str!` 内嵌 HTML/CSS/JS；开发期可用
  `kb_core --assets-dir <dir>` 覆盖文件而不必重新编译。
- **观感对齐 DSH**：沿用它的两级令牌命名（`--dsw-static-*` → `--dsw-alias-*`）与
  深色底 `rgb(21,21,23)`；reasoning 折叠行 + 生成中扫光对应 DSH 的 `ReasoningRow`。
- **只走 DOM API**：所有模型文本都用 `textContent` 写入，不用 `innerHTML` 拼接，
  因此模型输出无法变成 HTML/脚本。
- **增量渲染**：`delta` 到达后只改状态，用 `requestAnimationFrame` 合并成每帧一次渲染；
  正文里的 ``` 围栏代码块切成 `<pre>`，未闭合的围栏按普通文本处理。
- **多标签一致**：服务端用 `tokio::sync::broadcast` 广播，多个页面同时能看到同一轮输出；
  订阅落后（`Lagged`）时会收到一条明确提示而不是静默丢内容。
- **降级路径**：插件不在线时，设置面板仍完整可用（可以先把 key 配好），
  提问会收到 `plugin_offline` 的明确提示。

### 14.6 实测结果

`cargo test -p kb_svc_salvo -p kb_core` 全绿（39 项）：

| 组 | 数量 | 覆盖 |
| :--- | :---: | :--- |
| `hub` 单元测试 | 9 | 转发链路、缺服务/缺 key/插件离线、取消、断线、无关事件、ready |
| `settings` 单元测试 | 7 | TOML 解析/错误、遮蔽、往返、注释保留、幂等删除 |
| `wire` 单元测试 | 3 | 帧形状与可选的 `turn_id` |
| `plugin_socket` 单元测试 | 5 | 路径生成、唯一性、公历换算、幂等清理 |
| `web` 单元测试 | 2 | 首页 HTML、空设置接口 |
| `poc_uds_websocket` 集成测试 | 4 | UDS/TCP 上的 WebSocket 与路径约定 |
| `web_chat_roundtrip` 集成测试 | 4 | 浏览器↔插件端到端转发、设置接口遮蔽明文 key |
| `kb_core` 单元测试 | 3 | 四个命令行参数 |
| 文档测试 | 2 | `lib.rs` 与 `settings.rs` 的示例 |

真机跑 `kb_core` 的实际输出（节选）：自动生成配置文件 → 首页 200 + `text/html` →
`app.css` / `app.js` 的 content-type 正确 → `POST /api/settings/services` 后
配置文件被写回且**注释保留** → `GET /api/settings` 返回遮蔽后的 key 且
`active_service` 自动设为该服务 → `SIGTERM` 后 socket 文件被清理。

### 14.7 未完成 / 已知限制

1. **没有真正的 LLM 调用**：`kb_rig_llm` 尚未实现，因此现在提问会得到
   「插件未连接」的提示；界面的流式渲染路径由集成测试用「假插件」验证。
2. **没有浏览器端到端测试**：本环境没有可用的 Playwright/浏览器，
   因此只做了 `node --check` 语法检查 + DOM API 静态审查 + 服务端契约测试。
   加一条 Playwright 用例应当排进阶段 6。
3. **无鉴权、无 TLS、TCP 上也能访问 `/ws/plugin`**：符合当前「本机单用户」的阶段假设。
4. **API key 明文落盘**：见 §14.4。
5. **不保存对话历史**：刷新页面即清空；历史持久化留到知识库阶段。
6. **`--assets-dir` 的覆盖只在启动时读取**：改了前端文件需要重新启动进程。
