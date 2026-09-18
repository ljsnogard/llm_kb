# `kb_admin_desktop` 经本机 IPC / 远程 TCP 连 `kb_core` 的可行性研究

- 日期：2026-09-18 10:34
- 状态：**可行性研究**。只读调查 + 只读实测；**没有改动任何代码**。
  实测脚本与临时工程都在 `.gitignore` 忽略的 `external/feasibility-spike/` 下；
  除本文件外只动了两份开发日志：给
  [`kb_admin_desktop-20260917-1700.md`](kb_admin_desktop-20260917-1700.md) 补了
  "Flutter SDK 已补齐"的后记，并把本文件加进
  [`llm_kb-20260917-1655.md`](llm_kb-20260917-1655.md) §2 的落位表。
- 问题：
  1. `kb_admin_desktop` 启动时读**自己独立的配置文件**，按其中若干项「与 `kb_core`
     的连接方式」建连——可行吗？
  2. 本机方式＝用命令行启动 `kb_core` 子进程并连它的 IPC；远程方式＝经
     `kb_core_rproxy` 走 TCP。两条路都要能完成**应用层握手**并**列工作区 / 列会话**。
  3. 把 `kb_core_rproxy` 里的启动逻辑提取成独立 crate `kb_core_starter`，让
     `kb_core_rproxy` 变成"真正的代理"——可行吗？
- 前置：
  - [`kb_core_rproxy-20260917-1749.md`](kb_core_rproxy-20260917-1749.md)：stdio 端点握手、rproxy 的已拍板决定（§7）
  - [`kb_admin_desktop-20260917-1341.md`](kb_admin_desktop-20260917-1341.md)：客户端通信需求与协议数据来源
  - [`kb_svc_servo_ipc-20260917-1548.md`](kb_svc_servo_ipc-20260917-1548.md)：IPC 的引导机制与落地
  - [`kb_admin_desktop-20260917-1700.md`](kb_admin_desktop-20260917-1700.md)：本机 GUI 测试能力
    （**该文"本机没有 Flutter SDK"的结论已经过时**，见 §1.6）

> **2026-09-18 11:50 更新**：§9 记录了团队对 §6 中若干项的**拍板结果**、`kb_core_starter`
> **已经提取落地**的事实，以及一条**对 1749 §7.1 的修订**——`kb_core` 的 stdio 就绪通知
> 纳入 `abs_kb_svc`，两个层面的握手都算公开协议。§2.3 的配置示例已按拍板结果改成 TOML；
> §1–§8 的调研内容保持原样（它们仍然解释"为什么最终是这些形状"）。

---

## 0. 结论

| # | 子问题 | 结论 | 代价 / 缺口 |
| :--- | :--- | :--- | :--- |
| 1 | 客户端自持配置文件（若干连接方式，启动时读取） | ✅ **可行，工程量小** | 只需定格式、位置与"谁来解析"；无技术障碍 |
| 2 | 本机方式：启动 `kb_core` + 连 IPC | ✅ **零件全齐**，且已实测跑通 | 唯一的"缺口"是启动逻辑现在**锁在 rproxy 的 bin 里**，外部拿不到 |
| 3 | 远程方式：TCP 经 `kb_core_rproxy` | ⚠️ **服务端齐、客户端缺** | 现在只有一个一次性示例 `examples/probe.rs`；没有可复用的 TCP 客户端，帧编解码是 rproxy 的私有模块 |
| 4 | 提取 `kb_core_starter`，rproxy 改用它 | ✅ **机械改动** | 约 160 行搬家 + 依赖整理；`launch_` 本身不依赖 rproxy 的任何东西 |
| 5 | 客户端独立 workspace 引用主 workspace 的 crate | ✅ **实测通过** | path 依赖 + workspace 继承字段都能正确解析（§7.2） |
| 6 | FRB 把协议类型桥到 Dart | ⚠️ **能用但形状要挑** | 协议类型会大量退化成 opaque；建议客户端 Rust API 只暴露扁平的窄接口与自有 DTO（§3.5） |
| 7 | 端到端"握手 + 列工作区 + 列会话" | ✅ **服务端已实现这几个请求**，两条传输都能到达 | `Event::Ready` 尚未实现（不是本轮必需）；事件推送也还没有 |
| 8 | 本机验收能力 | ⚠️ **SDK 与构建链已在位**，但当前沙箱下 `/root` 只读 | Rust 侧可正常 `cargo check/test`；Dart/Flutter 侧要放宽沙箱才能跑（§1.6） |

一句话：**三条都可行，没有硬阻塞。** 真正的成本不在"提取 starter"或"读配置"，
而在两处：**远程 TCP 客户端目前不存在**，以及**FRB 桥接的形状**（协议类型不能直接当界面模型用）。

---

## 1. 现状核实（本轮实际读码 / 实跑）

### 1.1 协议层：本轮要用的东西全都已经定义好

`kb_svc/crates/abs_kb_svc_v1_desktop/src/` 里已经有：

| 需要的东西 | 位置 |
| :--- | :--- |
| 应用层握手 | `Request::Hello(ClientInfo)` → `Reply::Hello(ServerInfo)`；`handshake_.rs` 讲清它与系统层握手的边界 |
| 握手 trait | `TrHandshake::hello(ClientInfo) -> ServerInfo` |
| 工作区 / 会话 | `TrWorkspaceService`（3 方法）+ `TrSessionService`（4 方法）；`Request::{ListWorkspaces, ListSessions, …}` |
| 客户端代理 | `kb_svc_servo_ipc::Client` **已经实现** `TrHandshake` + 两个业务域 trait，并有裸信封入口 `send_envelope` |

因此"按照 `abs_kb_svc` 公开的协议握手并取列表"这件事，**协议侧零改动**。

### 1.2 服务端已经实现了本轮要用的全部请求

`kb_svc/crates/kb_core/src/ipc_.rs` 里 `KbService` 实现了 `TrHandshake` +
`TrWorkspaceService` + `TrSessionService`：`Hello` 会校验 `protocol_version`
（不匹配回 `BadRequest`），`ListWorkspaces` / `ListSessions` 直接转发给 `Store`。
`cargo test -p kb_core` 实测 **27 项全绿**（§7.1），其中
`ipc_round_trip_reaches_the_local_store_` 是"真起服务端、真走 ipc-channel、真落盘"。

**边界**：`Event::Ready` / `Event::StateChanged` 服务端尚未推送，`Ask` / `Cancel` /
设置 / 目录浏览也都还没有（服务端回 `BadRequest`）。本轮只要"握手 + 两个列表"，
所以不受影响；但界面上依赖 `plugin_online` 的判断暂时没有数据来源。

### 1.3 本机路径的零件齐全，但被锁在 rproxy 的 bin 里

`kb_plugins/crates/kb_core_rproxy/src/launch_.rs` 已经实现了本机路径需要的**全部**系统层动作：

```text
Command::new(kb_core) --handshake-prompt stdio --runtime-dir R --storage-dir S
  → 读 stdout 的一行 JSON，取出 ipc_name_file
  → Launched { child_, name_file_ }（Drop 时 kill 子进程）
```

它对外只有 `LaunchSpec` / `Launched` / `LaunchError` / `launch()` / `default_kb_core_path()`，
**不依赖 rproxy 的任何其它模块**。问题只有一个：`kb_core_rproxy` 只有 `[[bin]]`、
**没有 lib target**，所以这些 `pub` 项对别的 crate（包括客户端）**不可达**。

配合 `kb_svc_servo_ipc::Client::connect(runtime_dir)`（含重试），本机路径就是
"launch → connect"两步，今天就能跑通；缺的只是"把它放到一个可被依赖的 crate 里"。

### 1.4 远程路径：网关有了，客户端没有

`kb_core_rproxy` 已经能：启动 `kb_core` → 读 stdio 通知 → 连 IPC → 监听 TCP →
把远程客户端的 `RequestEnvelope` 原样转发、把 `ReplyEnvelope` 原样送回。
TCP 帧是 `[u32 BE 长度][1 字节种类][postcard 载荷]`，种类 `0=请求 / 1=应答 / 2=事件(预留)`，
单帧上限 8 MiB。

但客户端一侧只有 `examples/probe.rs`：一个**一次性的、阻塞的**示例（发两个请求就退出），
既没有超时、也没有 request_id 路由、也不处理种类 2 的帧。可复用的远程客户端**尚不存在**；
帧编解码（`src/frame_.rs`）是 rproxy bin 的私有模块，外部也拿不到。

### 1.5 客户端现状：与 `kb_core` 之间没有任何通道

- Dart 侧 9 个本地键全在 `shared_preferences`（`lib/src/services/local_store.dart`），
  与知识库有关的三项（`kb.services` / `kb.active_service` / `kb.workspaces`）本就
  计划改成服务端读写（见 1341 号记录 §2）；
- Rust 侧只有一个 `greet` 示例（`rust/src/api/simple.rs`），`main.dart` 里
  `RustLib.init()` 失败也不阻塞启动；
- 客户端自带的 Rust crate（`rust/Cargo.toml`）是**独立 workspace**（空 `[workspace]`），
  edition 2021，只依赖 `flutter_rust_bridge 2.13.0`，**没有**任何连接/配置能力。

### 1.6 本机验收能力（更正 1700 号记录）

1700 号记录的结论"本机完全没有 Flutter / Dart SDK"**已经过时**。本轮实际查到：

| 检查项 | 结果 |
| :--- | :--- |
| Flutter / Dart SDK | **在**：`/root/develop/flutter/bin/{flutter,dart}` |
| Linux 桌面构建链 | **在**：`clang` / `cmake` / `ninja` / `pkg-config` / `xvfb-run` 都在，`pkg-config --exists gtk+-3.0` 为真 |
| 当前沙箱下运行 | ⚠️ `/root` **只读**，`flutter` 启动时报 `engine.stamp.tmp.*: Read-only file system`、`engine.realm: Read-only file system`（SDK 自己的 cache 写不进去） |

也就是说：**Dart/Flutter 侧的验证是"能跑但要放宽沙箱"**（`external/frb-spike/run-spike*.sh`
就是这么跑的：它们把 Flutter SDK 的目录一起放开）；而**Rust 侧的验证在当前沙箱里正常**。
本报告的所有实测都只用 Rust 侧 + 只读命令。

---

## 2. 目标形态

### 2.1 "若干连接方式"落到三种形态

用户说的"若干项连接方式"实际上有三种，配置里应当能分别表达：

| 形态 | 系统层握手 | 典型用途 |
| :--- | :--- | :--- |
| **本机·启动** | 启动 `kb_core` 子进程 → 读 stdio 通知 → 连 IPC | 桌面端独占使用，最省事；进程随 App 生命周期 |
| **本机·附着** | 直接连已在跑的 `kb_core` 的 IPC（扫运行时目录） | 服务端已经由别人（rproxy / 手工）拉起时 |
| **远程·TCP** | 连 `kb_core_rproxy` 的 `host:port` | 跨机 / 跨容器；网关在对面启动 `kb_core` |

> "本机·附着"是**必须**有的一种，不是锦上添花：`kb_core` 启动时会清掉运行时目录里
> **全部** `kb-*.ipc`（`rendezvous_::clear_stale_name_files_` 的注释明确写了
> "一个运行时目录只应当挂一个 `kb_core`；两个实例会互相清名字"）。桌面端如果无脑再起
> 一个，会把已在跑的那个的名字文件删掉。

### 2.2 组件图（目标态）

```text
kb_admin_desktop（Flutter）
   │ 启动时：读配置 → 得到若干连接方式 → 按 default / 顺序尝试
   │ FRB
   ▼
客户端 Rust 侧（rust_lib_kb_admin_desktop，独立 workspace）
   │ 只暴露窄接口：connect(profile) / hello / list_workspaces / list_sessions
   ├── 本机·启动 ──► kb_core_starter::launch() ──► kb-core 子进程
   │                      └─► kb_svc_servo_ipc::Client::connect(runtime_dir)
   ├── 本机·附着 ──► kb_svc_servo_ipc::Client::connect(runtime_dir)
   └── 远程·TCP  ──► 新的 TCP 客户端 crate ──TCP──► kb_core_rproxy ──IPC──► kb-core
   │
   └── 配置解析（新）：serde_json 读客户端自己的配置文件
```

### 2.3 配置文件形态（**已拍板：TOML**，见 §9.1）

格式在 2026-09-18 拍板为 **TOML**（原 JSON 提案作废），解析与校验都放在**客户端 Rust 侧**
（决策 2：界面逻辑以外一律 Rust）。位置与语义见 §3.1。示例：

```toml
# kb_admin_desktop 的连接配置。启动时读取；不存在时由首次运行引导界面生成（见 §9.1）。
version = 1

# 缺省用哪一项；连不上时按声明顺序继续试其余项。
default = "本机"

[[connections]]
name = "本机"
kind = "local-launch"          # 起一个本机 kb_core 并连它的 IPC
kb_core = "/home/me/.local/bin/kb-core"
runtime_dir = "/run/user/1000/llm_kb"
storage_dir = "/run/user/1000/llm_kb/storage"
handshake_timeout_millis = 5000

[[connections]]
name = "已在本机跑着的"
kind = "local-attach"          # 只连已经在跑的 kb_core
runtime_dir = "/run/user/1000/llm_kb"

[[connections]]
name = "实验室"
kind = "tcp"                   # 经 kb_core_rproxy 走 TCP
address = "192.168.1.5:8788"
connect_timeout_millis = 3000
request_timeout_millis = 10000
```

`kind` 用**三个显式取值**而不是"`local` + `autostart` 布尔"：布尔读起来要猜，
而三种形态的行为差别很大（起不起进程、要不要 `kb_core` 路径、连 IPC 还是 TCP）。

### 2.4 职责划分

| 关注点 | 归属 |
| :--- | :--- |
| 配置文件的格式与解析 | **建议**：客户端 Rust 侧（可用 `cargo test` 覆盖）；Dart 只传路径、拿结果 |
| 启动 `kb_core` 子进程 | 新 crate `kb_core_starter`（rproxy 与客户端共用） |
| 本机 IPC 连接 | `kb_svc_servo_ipc::Client`（已有，不改） |
| TCP 帧编解码 | 从 rproxy 提取成共享模块，rproxy 与 TCP 客户端共用一份 |
| 远程 TCP 连接 | 新的 TCP 客户端 crate（rproxy 与客户端共用协议、各有各的端） |
| 界面状态（当前选中项等） | 仍在 Dart / `shared_preferences` |

---

## 3. 逐项可行性分析

### 3.1 客户端配置文件

**可行，且没有技术难点**，但要拍板四件事。

**(a) 谁解析。** 两条路都通：

| 方案 | 优点 | 缺点 |
| :--- | :--- | :--- |
| **A. Rust 解析（推荐）** | 与"连接方式"紧挨着；`cargo test` 能直接覆盖；格式只有一份定义 | 客户端 Rust crate 要加 `serde` / `serde_json`（`serde_json` 已是主 workspace 的常用依赖） |
| B. Dart 解析后经 FRB 传参 | Dart 文件 API 现成；配置可被 UI 直接编辑 | 格式变成 Dart 侧定义，Rust 无法独立验证；连接参数要跨 FRB 边界多传一份 |

方案 A 下，Dart 需要做的只是"把默认路径或显式路径交给 Rust"，之后
`connect` 的入参就是一个 profile 名字或一份扁平结构——FRB 处理扁平结构没有问题。

**(b) 文件放哪。** 桌面端要跨平台，候选：

```text
Linux   $XDG_CONFIG_HOME/kb_admin_desktop/config.json   （缺省 ~/.config/...）
macOS   ~/Library/Application Support/kb_admin_desktop/config.json
Windows %APPDATA%\kb_admin_desktop\config.json
```

两种实现：Rust 里手写十几行环境变量推导（**不引入新依赖**，且便于测试），
或引入 `dirs` / `directories` crate。建议前者，并把默认路径做成一个纯函数以单测覆盖。
另外留一个显式覆盖：`--config <路径>` 等价的入口（FRB 参数或环境变量
`KB_ADMIN_DESKTOP_CONFIG`），便于开发与排错时指向临时配置。

**(c) "若干连接方式"怎么用。** 需要明确的语义，建议：

1. 配置里可以有多项；`default` 指定缺省项；
2. 启动时先试 `default`；**失败则按声明顺序继续试其余项**（这正是"检查自己有哪些
   连接方式"的落地）；
3. 尝试结果（哪一项连上了、各自失败原因）交给界面，供用户手工切换；
4. 用户在界面上选的项写回 `shared_preferences`（**界面状态**，不是配置文件本身）。

每次尝试都要有短超时，"本机·启动"另外还要限制"等 stdio 通知"的时长（见 §3.2），
否则一个坏配置会把 App 启动卡住。

**(d) 配置文件 vs 界面设置的关系。** 本轮建议**只读**：文件是唯一事实来源，
设置面板以后再加"编辑连接"（那会引入写回、并发编辑等新问题）。这条要显式写进范围。

### 3.2 提取 `kb_core_starter`

**可行，是本次最"机械"的一件事。** `launch_.rs` 是自包含的：只用
`std::process` / `std::io` / `serde_json` / `thiserror` / `log`，不碰 rproxy 的
`frame_` / `ring_` / compio。

建议的搬迁与形状：

```text
kb_plugins/crates/kb_core_starter/     ← 新 crate（lib）
  src/lib.rs        LaunchSpec / Launched / LaunchError / launch() / default_kb_core_path()
```

| 项 | 建议 |
| :--- | :--- |
| 放置 | `kb_plugins/crates/kb_core_starter`：1749 号记录 §7.5 已把"启动器"归到插件区（"以后还会有别的 launcher"） |
| rproxy 改动 | 删 `src/launch_.rs`；`main.rs` 改 `use kb_core_starter::{launch, LaunchSpec, Launched}`；`Cargo.toml` 加 path 依赖 |
| 依赖整理 | 提取后 **`serde_json` 在 rproxy 里再无使用者**（只有 `launch_` 用它），应一并删掉；`thiserror` 要留（`ring_.rs` 在用） |
| 是否连 IPC 一起管 | **建议不管**。starter 只做"起进程 + 读通知"，连接仍由调用方 `Client::connect` 负责。这样 starter 不依赖 `kb_svc_servo_ipc` / `ipc-channel`，依赖面最小。若日后确实多处重复"起完就连"，再加一个高层 `start_and_connect()` 不迟 |

**两条应当顺手补的健壮性问题**（都是 rproxy 现有代码里就存在的）：

1. **等 stdio 通知没有超时**。`reader.read_line()` 会一直等下去；子进程若在打通知前
   卡住，父进程就永久挂起。桌面端把它放启动路径上，这个风险必须收掉——加一个
   `handshake_timeout`，超时报 `LaunchError`。
2. **`default_kb_core_path()` 只会看"与本可执行文件同目录"**。这对 rproxy 成立
   （`cargo build` 把两个 bin 放进同一个 `target/debug`），但**对 Flutter App 不成立**：
   `current_exe()` 拿到的是 Flutter runner（`.../bundle/kb_admin_desktop`），而
   `kb-core` 在开发期位于**主 workspace 的** `target/debug/`，发布期需要随包安装。
   因此桌面端**必须**靠配置里的 `kb_core` 路径，`default_kb_core_path()` 只能算兜底。
   建议把"猜路径"抽成接受一个基准目录的纯函数 `kb_core_beside(dir)`，便于测试与复用。

**测试**（`AGENTS.md` 第 2、10 条）：

- **单元**：把"解析那一行 JSON 通知"抽成纯函数 `parse_notice_(&str) -> Result<PathBuf, …>`，
  直接测合法 / 缺字段 / 非法 JSON 三种输入。现在的实现是内联在 `launch()` 里的，抽出来
  才可测。
- **集成（不依赖真 `kb-core`）**：造一个"假 kb_core"（shell 脚本或 `tests/fixtures`
  下的小程序），行为是"打一行合法通知然后睡"或"什么都不打直接退"，用它覆盖
  spawn / 通知解析 / 提前退出 / 超时四条路径。这条能绕开"跨包拿不到
  `CARGO_BIN_EXE_kb-core`"的限制（1749 号记录 §3.3 提过这个坑）。
- **端到端（真 `kb-core`）**：`CARGO_BIN_EXE_kb-core` 只对**定义了该 bin 的那个包**的
  集成测试可见，所以"真启动真连接"的用例应当放在 `kb_core/tests/` 下
  （`kb_core` 加 `kb_core_starter` 作 dev-dependency；两者无循环依赖）。

### 3.3 `kb_core_rproxy` 改成"真正的代理"

提取之后 rproxy 的行为**一行都不用变**：它仍然是"起 kb_core（改成调用 starter）→
连 IPC → 监听 TCP → 一次服务一个远程客户端"。所谓"真正的代理"就是**把'起进程'
这件事交出去**，让 rproxy 只剩"转发"。

顺带值得记一笔的两处现状（本轮不必改，但要在文档里写清）：

- rproxy 在**第一个远程客户端连上来时**自己先做一次 `Hello`（1749 §7.2 的决定）。
  远程客户端按协议还会再发一次 `Hello`，于是同一条链路上发生两次握手。
  今天无害（第二次照样被转发并应答），但等事件/状态推送落地后，可以考虑去掉
  rproxy 自己那次，改成"首个业务请求失败即判定上游不可用"。
- rproxy 的 `request_id` 来自远程客户端，而上游只有**一条** `Client` 连接、
  其 `pending_` 表按 `request_id` 索引。当前"一次只服务一个远程客户端"所以不会撞；
  **一旦 rproxy 支持并发客户端，不同 TCP 连接的同名 `request_id` 就会互相顶掉**。
  这是一条要在并发改造时一起处理的约束。

### 3.4 远程 TCP 客户端（本轮真正的新增件）

**可行**，但需要新写一个 crate。建议形状：

```text
kb_plugins/crates/kb_core_rproxy_client/     ← 新 crate（lib）
  src/frame_.rs   纯编解码：encode_request / decode_reply / decode_any（种类 0/1/2）
  src/client_.rs  TcpClient：连接、超时、request_id、按 request_id 等应答
```

| 关注点 | 建议 |
| :--- | :--- |
| **帧编解码复用** | 把 rproxy 的 `frame_.rs` 拆成两半：**纯字节编解码**（无 IO、无 compio）搬进共享 crate；rproxy 侧保留一个几行的 compio `write_frame_` 包装。**不要**让共享 crate 依赖 compio（那会把 compio 拖进桌面端的原生库）。这样"帧格式只有一份定义"，不会两边漂移 |
| 客户端实现 | 首选与 `kb_svc_servo_ipc::Client` **同构**的做法：一条专职读线程 + `request_id → oneshot` 表，对外仍是"运行时无关的 future"（`gen_mcf2` 展开、可取消）。这样 FRB 侧、取消语义、`AGENTS.md` 第 4 条都与现有客户端一致 |
| 顺序假设 | rproxy 现在是**严格一问一答**（收到一帧→转发→写回→再读下一帧），所以即使客户端只允许"同时一个在途请求"也够用。先用 `Mutex<连接>` 的简单实现即可，把"多请求并发"留到 rproxy 支持并发之后 |
| 种类 2（事件） | 现在服务端不推事件，但**读循环必须按种类分派**而不是"读到的下一帧就是应答"——否则事件一上线，客户端就会把事件帧当应答解码。`probe.rs` 现在正是"非 1 就报错"的写法，**不要**照抄进正式客户端 |
| 超时 | TCP 连接超时与单次请求超时都要可配（配置里有），否则对面挂住 = App 挂住。8 MiB 单帧上限也要在客户端侧执行（对端可以谎报长度） |
| 错误分层 | 传输失败走 `RpcError::Transport(<本 crate 错误类型>)`，业务失败仍是 `RpcError::Business` —— 与 `abs_kb_svc` README §5 第 7 条一致 |
| 测试 | 用**假服务端**（进程内 `TcpListener`，按帧格式回应答/事件/坏帧）覆盖：正常一问一答、应答 request_id 对不上、帧长度非法、连接中断、事件帧混入。真端到端可以直接用现成的 `kb_core_rproxy` + 真 `kb-core`（§7.3 已实跑过一次） |

**另一种选择**（供比较，不推荐本轮做）：桌面端**永远只说 TCP**，本机模式也改成
"启动本地 `kb_core_rproxy`（绑 `127.0.0.1`）再连它"。好处是客户端只有一种传输、
不必把 `ipc-channel` 编进 App 的原生库；代价是多一个常驻进程、且与用户描述的
"本地的直接启动 `kb_core`"不符。**记录在此，等本机路径真的在某个平台上出问题时再回头考虑。**

### 3.5 客户端统一抽象与 FRB 桥接

**能一起做，但要克制。**

**(a) 两条传输要不要统一到同一组 trait。** `abs_kb_svc` 的按域 trait 是给"面向 trait
编程"设计的，理论上可以让 `TcpClient` 也实现 `TrHandshake` / `TrWorkspaceService` /
`TrSessionService`，再让桌面端持有一个 `enum Connection { Local(Client), Remote(TcpClient) }`
并实现同一组 trait。好处是上层完全对称；代价是：

- `TrKbEndpoint::Error` 要合并成一个新错误枚举（`Ipc(ServoIpcError) | Tcp(TcpError)`）；
- `enum` 的每个方法都要走一遍 `gen_mcf2` 宏 + 关联类型填写（现有 `client_.rs` 里
  七个方法每个都要配套 `#[gen_may_cancel_future]`），样板量不小。

**建议**：先不追求"一个 enum 实现全部 trait"，而是让桌面端 Rust 侧暴露一组
**窄接口**（`hello` / `list_workspaces` / `list_sessions` / `connect`），内部 `match`
分派到两条传输。等 `Ask` / 事件流落地、需要统一的取消与订阅语义时，再收敛到 trait 层。

**(b) FRB 的形状（有实测依据）。** `external/frb-spike/` 里已经用一个**副本**工程
实测过 `flutter_rust_bridge 2.13.0` 直接扫描 `abs_kb_svc::v1::desktop` 的效果：

- `rust_input: crate::api,abs_kb_svc::v1::desktop` + 用 `rust_preamble` 补 `abs_llm`
  的传递类型后，**`cargo check` 通过、Dart `analyze` 无告警**；
- 但生成出来的 Dart 形态很两极：`WorkspaceList` / `SessionList` / `ServerState` /
  `DirEntry` 等是**普通 class**，而 `Workspace` / `SessionSummary` / `AskRequest` /
  `Request` / `Reply` / 全部标识类型共 **27 个**是 **opaque**（`RustOpaqueInterface`）。
  opaque 类型仍会生成字段访问器（受控实验里 `String get name` 是有的），所以**不是
  不能用**；代价是每次读字段都过一次 FFI，而且 Dart 侧不能自由构造，只能让 Rust 造。
- 退化的根因是"**直接持有**一个 opaque 类型字段的结构体会跟着 opaque，包在
  `Vec<>` 里的则不受影响"（spike4 已证实这条规则）；至于**跨 crate** 时
  `WorkspaceId` 为何一开始就 opaque，spike5 想验证但脚本取错了输出路径，
  **结论未定**（不影响本报告的判断，但要做 FRB 落地时值得先花半小时把它钉死）。

⇒ **建议**：给 FRB 看的 API **不要直接回 `Workspace` / `SessionSummary` 这些协议类型**，
而是回客户端自己定义的**扁平 DTO**（字段只有 `String` / `i64` / `List<...>` /
简单枚举），在 Rust 侧做一次 `From<Workspace>` 映射。这样：

- Dart 侧拿到的是可直接构造、可直接比较的普通类，界面代码不必依赖 FRB 的 opaque 语义；
- 协议类型怎么演化（加字段、改嵌套）都不会直接冲击 Dart 侧；
- 代价是多一层 ~20 行的映射代码。

**(c) 阻塞与线程。** `Client::connect` 是**同步阻塞**（读名字文件 + 重试，缺省最多 5 秒），
`kb_core_starter::launch` 也是阻塞的。FRB 的异步函数跑在它自己的执行器上，
**不能在 poll 里内联这两个调用**。建议客户端 Rust 侧做一个"连接管理器"：
在专职线程里做 launch/connect，FRB 侧只 await channel。这与
`kb_svc_servo_ipc::Client` 已有的"路由线程 + oneshot"是同一种结构，风格一致。

**(d) 取消。** 本轮的四个动作（握手 + 两个列表 + 连接）都很快，可以先不做取消；
但 `AGENTS.md` 第 4 条要求"实现不得假定调用者不会取消"，而 `Client` 的 future
本身就支持 `.may_cancel_with(token)`。等 `Ask` 落地时再决定"取消令牌怎么过 FRB 边界"。

### 3.6 生命周期、并发与一致性

这是本轮**最容易踩坑**的部分，三条现状约束都要在配置语义与文档里体现：

1. **`kb_core` 一次只服务一个客户端**（`serve_.rs` 的 accept 循环是顺序的），
   rproxy 也**一次只服务一个远程客户端**。因此：
   - 如果 rproxy 正握着 `kb_core` 的唯一 accept 槽，"本机·附着"会连不上并重试到超时；
   - 桌面端最稳的用法是"本机·启动"，自己起一个独占的 `kb_core`。
2. **一个运行时目录只应挂一个 `kb_core`**（启动时清 `kb-*.ipc`）。所以：
   - 桌面端自起的实例必须用**自己的运行时目录**，不能与 rproxy 的默认目录
     （`$XDG_RUNTIME_DIR/llm_kb`）重合；
   - 配置里的 `runtime_dir` 要显式，不要靠默认值。
3. **存储目录要共享，但两个 `kb_core` 同时写同一个存储目录没有跨进程协调**。
   `Store` 的写是"临时文件 + rename"，**不会读到半个 JSON**，但"读-改-写"之间
   没有锁，两个实例并发改同一对象可能丢更新。建议 MVP 明确约定
   "**同一存储目录同时只应有一个 `kb_core`**"，并在文档里写出来。

**子进程归属**：桌面端起 `kb_core` 时，`Launched` 的 `Drop` 会 `kill` 子进程——
"App 关了就关掉自己起的服务端"是正确的。但 App 被强杀时 `Drop` 不会跑，会留下孤儿
进程。建议启动时先做一次"**附着探测**"：配置的 `runtime_dir` 下若有**内容非空**的
`kb-*.ipc`，先尝试连它；连上了就用（不为孤儿问题新增机制），连不上才走启动。
（更彻底的方案是让 `kb_core` 写 pid 文件或加实例锁——那是 `kb_core` 的改动，另议。）

### 3.7 安全

远程方式**没有鉴权、没有 TLS**（1749 §7.5 已明确接受为临时状态）。因此：

- 远程连接项里的地址应当被当作"受信网络内"的前提；
- 客户端配置里**不放任何凭据**（现在也确实没有可放的东西）；
- 一旦要跨不受信网络，必须先做鉴权，且这是 `kb_core_rproxy` 侧的工作，
  不在本轮的客户端改造里。

---

## 4. 风险与未决项

| # | 风险 / 未决 | 影响 | 建议 |
| :--- | :--- | :--- | :--- |
| 1 | `kb-core` 可执行文件在发布包里怎么定位 | 本机·启动可能找不到二进制 | 配置里的 `kb_core` 是**必需**项；发布打包（Linux bundle 的 `lib/` 安装规则等）另列一项 |
| 2 | 桌面端 Rust 侧依赖 `ipc-channel` 的平台支持 | 若将来要跑 Android/iOS，IPC 客户端可能不可用 | 本轮目标平台是桌面；在文档里写明"本机 IPC 只在桌面平台保证" |
| 3 | FRB opaque 退化的根因未定（spike5 未完成） | 影响 DTO 设计的自由度 | 采用"窄接口 + 自有扁平 DTO"即可绕开；真要修 FRB 配置时再补做 spike5 |
| 4 | 两个 `kb_core` 共写一个存储目录会丢更新 | 数据一致性 | MVP 约定"同时只有一个"；将来给 `Store` 加文件锁或让所有客户端都走同一个 `kb_core` |
| 5 | 客户端 Rust crate 是独立 workspace，`Cargo.lock` 被 `.gitignore` 忽略 | 两条依赖解析路径可能漂移出不同版本 | 已实测可编译；若要可复现，考虑让客户端 crate 并入主 workspace（见 §6 待拍板） |
| 6 | rproxy 并发化后的 `request_id` 撞车 | 应答串线 | 写入 rproxy 的模块文档；并发改造时一并解决 |
| 7 | stdio 通知超时缺失（现状） | App 启动可能被卡死 | 提取 `kb_core_starter` 时一并补上 |
| 8 | 事件推送与 `Event::Ready` 未实现 | 界面拿不到 `plugin_online` | 本轮范围外；做好种类 2 的分派即可 |

---

## 5. 建议的落地顺序

每一阶段都**独立可验证**，且把"能在当前沙箱验证的 Rust 部分"排在前面。

| 阶段 | 内容 | 验证手段 |
| :--- | :--- | :--- |
| **0** | 拍板 §6 的对外约定（配置格式、crate 名与放置、对外接口） | 讨论 |
| **1** | 提取 `kb_core_starter`（含通知解析纯函数 + 超时 + 测试）；rproxy 改用它 | `cargo test -p kb_core_starter`；假 kb_core 覆盖四条路径；`cargo test --workspace`；重跑 §7.3 的端到端 |
| **2** | 提取帧编解码成共享模块；新增 TCP 客户端 crate（假服务端测试） | `cargo test -p <新 crate>`；用真 rproxy + 真 kb-core 跑一次 probe 式端到端 |
| **3** | 客户端配置模块（解析 + 默认路径推导 + 尝试顺序） | `cargo test`（纯函数：路径推导、坏配置、缺字段） |
| **4** | 客户端 Rust 侧窄接口 + 连接管理器（Dart 传入选中项） | 客户端 workspace 里 `cargo test/check`；用 `external/feasibility-spike/` 那种独立消费工程做跨 workspace 编译验证 |
| **5** | FRB 生成 + Dart 侧接线（连接选择、工作区/会话列表落到界面） | 需要放宽沙箱：`flutter analyze` / `flutter test`；界面验收 |
| **6** | 文档收尾：`kb_core_rproxy/README.md`、`kb_admin_desktop/README.md`、`llm_kb-20260917-1655.md` §2 的落位表 | 人工评审 |

---

## 6. 需要拍板的对外约定（`AGENTS.md` 第 1 条）

> 第 1 条已于 2026-09-18 拍板（TOML），第 2 条里的 `kb_core_starter` 名字与放置也已随
> 本次提取落地——详见 §9.1。下面的原文保留，便于对照"当初为什么列成待定"。

1. ~~**客户端配置文件的格式与位置**（§2.3 的 JSON 提案、平台目录规则、`kind` 的三个取值、
   `default` + 顺序回退的语义、配置只读还是可被界面写回）~~ → **已拍板：TOML；
   启动时读或创建；"只读"这条被"首次运行要能生成"取代**；
2. **新 crate 的名字与放置**：~~`kb_core_starter`（`kb_plugins/crates/`）~~ →
   **已落地在 `kb_svc/crates/`**（服务侧，不是插件，见 §9.2）；
   剩下的：TCP 客户端 crate 的名字（`kb_core_rproxy_client`？）与它和帧编解码的关系
   （一个 crate 两个模块 vs 两个 crate）；
3. **帧编解码的共享方式**：把 rproxy 的 `frame_.rs` 拆成纯编解码 + IO 包装，前者进共享 crate；
   **同时**决定 rproxy 的 TCP 帧格式从此是否算对外约定（远程客户端要照着它实现，实际上已经是了）；
4. **客户端 Rust crate 是否并入主 workspace**：现在它是独立 workspace（空 `[workspace]`），
   依赖面变大之后（`abs_kb_svc` / `kb_svc_servo_ipc` / `kb_core_starter` / TCP 客户端 / 配置），
   "独立"带来的隔离收益在变小，而"Cargo.lock 不统一、测试要单独跑"的成本在变大。
   两条都可行（§7.2 已证明独立也可编译），但应明确选一条；
5. **是否给 `kb_core` 加 lib target**：`kb_core_rproxy` 与桌面端都只能靠"启动二进制"，
   没有进程内嵌入的可能；若加 lib target，测试与嵌入都会容易很多。这属于 `kb_core` 的对外形态变更。

---

## 7. 附录：本轮实测证据

环境：`cargo 1.100.0-nightly` / `rustc 1.100.0-nightly`；
`CARGO_HOME=$PWD/external/cargo-home`（`~/.cargo` 在本机只读，见 1655 号记录 §4）。

### 7.1 基线

```console
$ CARGO_HOME=$PWD/external/cargo-home cargo check --workspace
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 2m 39s   # 全绿

$ CARGO_HOME=$PWD/external/cargo-home cargo test -p kb_core
running 27 tests
test result: ok. 27 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

### 7.2 跨 workspace 的 path 依赖（本次新做的探针）

`external/feasibility-spike/ext-consumer/`：一个**独立 workspace** 的 crate，
用 path 依赖引用主 workspace 的 `abs_kb_svc` + `kb_svc_servo_ipc`，并真调用
`Client::connect` / `hello` / `list_workspaces` / `list_sessions`：

```console
$ cd external/feasibility-spike/ext-consumer && CARGO_HOME=…/external/cargo-home cargo check
    Checking abs_kb_svc v0.1.0 (/root/projects/me.noli/llm_kb/kb_svc/crates/abs_kb_svc)
    Checking kb_svc_servo_ipc v0.1.0 (/root/projects/me.noli/llm_kb/kb_svc/crates/kb_svc_servo_ipc)
    Checking ext-consumer v0.0.0 (/root/projects/me.noli/llm_kb/external/feasibility-spike/ext-consumer)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1m 09s   # 通过
```

结论：`[lints] workspace = true`、`[dependencies] x = { workspace = true }` 这些
**workspace 继承字段在跨 workspace 消费时能正确解析**；`kb_core_starter` 将来被
客户端按 path 引用，机制上是同一件事。（`external/frb-spike/` 里还有一份更强的证据：
客户端 rust 工程**副本**同时依赖 `abs_kb_svc` + `abs_llm` + `kb_svc_servo_ipc`，
`cargo check` 通过、`flutter analyze` 无告警。）

### 7.3 远程路径端到端（本次实跑）

```console
$ ./target/debug/kb-core-rproxy --listen 127.0.0.1:8899 --kb-core ./target/debug/kb-core \
    --runtime-dir /tmp/kb-feas/run --storage-dir /tmp/kb-feas/data
WARN  kb_core_rproxy] 本网关没有鉴权与 TLS：…
INFO  kb_core::serve_] kb_core v0.1.0（协议 v1）
INFO  kb_core::serve_] IPC 端点文件: /tmp/kb-feas/run/kb-20260918-fa25a789….ipc
INFO  kb_core_rproxy::launch_] kb_core 已启动 (pid=6)，端点名字文件: …/kb-20260918-fa25a789….ipc
INFO  kb_core_rproxy] 正在监听 127.0.0.1:8899（上游 kb_core pid=Some(6)…）
INFO  kb_core_rproxy] 远程客户端已连接: 127.0.0.1:36250
INFO  kb_core_rproxy] 应用层握手成功：服务端版本 0.1.0，协议 v1

$ ./target/debug/examples/probe 127.0.0.1:8899
已连上 127.0.0.1:8899
握手 OK：服务端版本 0.1.0，协议 v1
已建立工作区: probe 建的工作区 (w-8c3f5f9aa09e43d5b64eca6b0770209c)
共 1 个工作区
  w-8c3f5f9aa09e43d5b64eca6b0770209c	probe 建的工作区	/tmp/probe
```

这条链路上"启动子进程 + stdio 系统层握手 + IPC 连接 + TCP 帧 + 应用层握手 + 业务请求"
全部真实发生过一遍。**本机方式与本条只差"谁在前面发起连接"**：rproxy 是"先起后连再转发"，
桌面端是"先起后连直接用"。

### 7.4 stdio 系统层握手（本次实跑）

```console
$ timeout 3 ./target/debug/kb-core --handshake-prompt stdio \
    --runtime-dir /tmp/kb-stdio/run --storage-dir /tmp/kb-stdio/data >out.txt 2>err.txt
$ cat out.txt
{"event":"ipc_ready","ipc_name_file":"/tmp/kb-stdio/run/kb-20260918-d2920c0ea4574633abbb3851de70cff5.ipc","pid":6,"protocol_version":1}
$ wc -c < out.txt
136
```

stdout 上**正好一行**，与 `launch_` 的解析逻辑对得上。

---

## 8. 相关文档

- 1749 号记录 §7：两层握手、rproxy 的职责、放置、安全前提——本报告全部沿用；
- 1341 号记录：客户端通信需求与协议数据来源；
- `kb_svc/crates/abs_kb_svc_v1_desktop/src/handshake_.rs`：两个层面握手的完整说明；
- `kb_svc/crates/kb_svc_servo_ipc/src/lib.rs`：本机 IPC 的引导（rendezvous）机制；
- `kb_plugins/crates/kb_core_rproxy/README.md`：TCP 帧格式与当前能力边界。

---

## 9. 决策记录与实施进展（2026-09-18 11:15）

### 9.1 本轮拍板的四条

| # | 决策 | 落位 / 影响 |
| :--- | :--- | :--- |
| 1 | **客户端配置文件用 TOML** | §2.3 的 JSON 提案作废，示例已改成 TOML；解析用 `toml` crate |
| 2 | **`kb_admin_desktop` 里界面逻辑以外一律用 Rust 实现；有条件时独立成可复用 crate** | 配置解析、连接管理、TCP 客户端、启动 `kb_core` 全部落在 Rust 侧；Dart 只做界面与状态。本次的 `kb_core_starter` 就是这条的第一个产物 |
| 3 | **启动时读或创建配置文件；若为新建，要提供界面让用户选择或填写如何连 `kb_core`**（重要技术决策） | 客户端多出"首次运行"这条状态机：文件不存在 → 引导界面 → 按用户输入生成文件。§3.1(c) 的"default + 顺序回退"仍然成立，但要加一条"首次生成" |
| 4 | **等 stdio 通知的取消/超时逻辑放进 `kb_core_starter`，做成与运行时无关的异步库，等多久由调用者决定** | 本次已落地（§9.2）。`kb_core_starter` **不自带定时器**：它把等待做成可取消 future，由调用方用 `abs_cancel` 令牌表达"多久之后不等了" |
| 5 | **`kb_core` 的 stdio 就绪通知纳入 `abs_kb_svc`；两个层面的握手都算公开协议** | 新增 `abs_kb_svc::v1::desktop::{IpcReadyNotice, HandshakeNoticeKind}`；`kb_core` 用它序列化、`kb_core_starter` 用它解析。**修订** 1749 §7.1 的"系统层格式不进协议" |

第 5 条的来龙去脉值得记一笔，因为它**推翻了 1749 §7.1 已经拍过的决定**：

- 1749 §7.1 的原话是"系统层握手的消息格式**不进 `abs_kb_svc`**"，
  `abs_kb_svc_v1_desktop/README.md` 与 `handshake_.rs` 的模块文档也都是这个口径；
- 但那条通知事实上是**两个进程之间的约定**：`kb_core::serve_` 用
  `serde_json::json!` 现拼，`kb_core_starter` 按字符串键现抠，**没有任何一处
  能挡住单边改名**——真正的失败模式是"跑起来发现启动不了"；
- 修订后的分界是**"消息在协议里，机制在传输实现里"**：选哪种内核端点、端点放哪、
  失败怎么重试仍由传输实现决定；`kb_svc_servo_ipc` 那条"扫运行时目录"的路径
  压根不经过这条通知，所以"换传输不必改协议"这个初衷没有被牺牲。

1749 §7.1 已就地加了修订标注（保留了原文与推翻理由）。

决策 3 还留了三个实现前要定的细节（都不影响本报告的结论）：

- "创建"意味着客户端要**写**配置文件，这与 §3.1(d) 原先"本轮只读"的建议相反，以本次拍板为准；
- 首次生成时文件里放什么（内置一份指向同目录 `kb-core` 的 `local-launch`？还是留空等用户填）、
  以及"用户什么都没填就退出"时下次是否再问；
- 写回要**原子**（临时文件 + rename），与 `kb_core` 存储层的约定保持一致。

### 9.2 本次已完成：提取 `kb_core_starter`

代码落位（放在 `kb_plugins/crates/`，与 §3.2 / §3.3 的建议一致）：

| 位置 | 内容 |
| :--- | :--- |
| `kb_svc/crates/kb_core_starter/` | 新 crate（lib）。公开面：`LaunchSpec` / `Launched` / `LaunchError` / `start()` / `default_kb_core_path()` / `kb_core_beside()`；私有模块 `error_` / `launch_` / `notice_` |
| `src/launch_.rs` | `#[gen_mcf2::gen_may_cancel_future(Start, pub)]` 展开出 `StartAsync`：`.await` 为不可取消路径，`.may_cancel_with(token).await` 为可取消路径 |
| `src/notice_.rs` | 解析协议类型 `abs_kb_svc::v1::desktop::IpcReadyNotice`（只取 `ipc_name_file`）；纯函数，可单测 |
| `kb_svc/crates/abs_kb_svc_v1_desktop/src/handshake_.rs` | 新增系统层握手消息：`IpcReadyNotice` + `HandshakeNoticeKind`（`event` 线上取值 `"ipc_ready"`），含 JSON 线格式与 postcard 往返测试 |
| `kb_svc/crates/kb_core/src/serve_.rs` | 打通知改成用 `IpcReadyNotice` 序列化（不再是 `serde_json::json!` 现拼） |
| `tests/launch.rs` | 集成测试：假 `kb_core` 脚本覆盖 正常 / 提前退出 / 坏通知 / 可执行文件不可用 / 已取消不起进程 / **等待中取消并结束子进程** / `kb_core_beside` |
| `kb_plugins/crates/kb_core_rproxy/` | 删 `src/launch_.rs`（-163 行）；`main.rs` 改用 `kb_core_starter::start(&spec).await`；`Cargo.toml` 去掉 `serde_json`（只有旧 `launch_` 用它），保留 `thiserror`（`ring_` 在用） |
| 根 `Cargo.toml` | 把 `kb_svc/crates/kb_core_starter` 加进成员（**服务侧**那一组，不是插件组） |

> **放置修正（2026-09-18 11:40）**：§3.2 与 §6 第 2 条原建议放在
> `kb_plugins/crates/`（沿用 1749 §7.5"启动器算插件"的说法），实际落地改为
> **`kb_svc/crates/kb_core_starter`**：它是 `kb_core` 的配套启动逻辑、属于**服务侧基础设施**，
> 而 `kb_core_rproxy` 那种"对外提供服务的网关"才算插件。§3.2 的其余建议（依赖面最小、
> 不管 IPC 连接、超时交给调用方）都照原样落地。

关键形状（对应决策 4）：

```text
start(&spec) ──► StartAsync（与运行时无关的 future）
  第 1 次 poll ：Command::spawn（短系统调用）+ 起一条专职线程读 stdout 那一行
  之后        ：只轮询 oneshot 完成量与取消令牌 —— 执行器线程不参与等待
  收场        ：成功              → Launched（Drop 时结束子进程）
                取消 / 出错 / future 被丢弃 → ChildGuard_ 结束子进程
```

- **不挑运行时**：没有 tokio / compio 依赖；集成测试用 `futures_lite::block_on` 驱动它；
- **不自带定时器**：`abs_cancel` v0.2 只提供"永不取消"与"一出生就取消"两种令牌，没有超时令牌，
  所以"多久算超时"只能由调用方造令牌决定——这正是决策 4 要的形状；将来若要给调用方一个
  现成的超时令牌，那是 `abs_cancel` 或调用方的事，不改变本 crate 的接口；
- **子进程不会变成孤儿**：取消、读通知失败、future 被直接丢弃三条路径共用同一个 `ChildGuard_`；
- 顺手补的两处：`default_kb_core_path()` 现在委托给新的 `kb_core_beside(dir)`
  （桌面端要自己决定拿哪个目录当基准，见 §3.2），并在 Windows 下找 `kb-core.exe`。

### 9.3 验证

- `cargo test -p kb_core_starter`：2 单元 + 7 集成 + 3 文档测试**全绿**；
- `cargo test -p abs_kb_svc`：新增的系统层通知类型带 JSON 线格式测试与 postcard
  往返测试（后者守的是本模块"不用内部标签 / 不用 `skip_serializing_if`"那两条约定）；
- 反脆弱：集成测试连跑 60 次、以及 4 路并发 × 20 次，**0 失败**。
  过程中确实抓到一个 flake：并行测试里"某个线程正在写脚本"与"另一个线程 fork"重叠时，
  exec 会拿到 `ETXTBSY`（Text file busy）——已用一把 `SPAWN_LOCK_` 把"写脚本 + 起进程"
  串起来消掉，理由写在测试文件里（这是内核的 fd 继承行为，与生产代码无关）；
- `cargo clippy -p kb_core_starter -p kb_core_rproxy --all-targets`：这两个 crate 无告警；
- **端到端复验**：改用 starter 之后的 `kb-core-rproxy` + `examples/probe` 仍然握手成功、
  建/列工作区正常（§7.3 的命令，端口改 8902）；
- 已知**与本轮无关**的红：`cargo test --workspace` 里 `kb_rig_llm_v1_adapt` 与
  `kb_rig_llm_v1_agent` 各有 1 项失败，断言的是 `abs_llm::Capabilities` 的 serde 表示
  （它们期望 JSON 对象，而 `abs_llm` 现在把它序列化成位标志整数）。这两个 crate 与
  `abs_llm` 本次一行未动，属于既有状态，另行处理。

### 9.4 仍然待定

§6 的第 3–5 条还没拍板（帧编解码的共享方式、客户端 rust crate 是否并入主 workspace、
是否给 `kb_core` 加 lib target）。下一步做"远程 TCP 客户端 + 客户端配置"时，第 3、4 条会先撞上来。

---

## 10. 协议 crate 拆分（2026-09-18 12:20）

### 10.1 拍板的三条

| # | 决策 | 落位 |
| :--- | :--- | :--- |
| 1 | `abs_kb_svc` 里的 `v1::desktop` 拆成独立 crate | **`kb_svc/crates/abs_kb_svc_v1_desktop`** |
| 2 | 两个层面握手里**系统层**的消息拆成独立 crate | **`kb_svc/crates/abs_kb_core_handshake`**（你写的 `abs_kv_core_handshake` 按笔误处理，落地为 `kb`） |
| 3 | `abs_kb_svc` **保留**，但只做**聚合** | `pub mod v1 { pub use abs_kb_svc_v1_desktop as desktop; }`——一行类型都不定义 |

范围上有一个刻意的取舍：**只把系统层拆出去，应用层握手与业务数据留在
`abs_kb_svc_v1_desktop`**。原因是依赖方向——`ServerState` 带着服务列表
（`ServiceSummary` / `ServiceId`），把它搬进"握手" crate 就得把业务类型一起搬过去，
否则两个 crate 互相依赖。所以"握手 crate"= 那条只依赖 `serde` 的
`IpcReadyNotice`。

### 10.2 为什么系统层要单独一个 crate

一句话：**只想启动并找到 `kb_core` 的调用方不该依赖整套业务协议**。

`kb_core_starter` 正是这种调用方——它起进程、读一行通知就结束了。
在拆分之前它必须依赖 `abs_kb_svc`，于是为了四个字段的通知，连带把工作区 / 会话 /
服务那整套类型以及背后的 `abs_llm` 拖进依赖树（桌面端还要把这一切编进原生库）。
拆开之后它的依赖只剩 `abs_kb_core_handshake`（`serde`）+ `abs_cancel` + `serde_json`。

### 10.3 落位与兼容

```text
abs_kb_svc                      ← 聚合层（只有一条 pub use 别名）
└── v1::desktop  ──别名──►  abs_kb_svc_v1_desktop
                              ├── 应用层握手（ClientInfo / ServerInfo / PROTOCOL_VERSION / ServerState）
                              ├── 业务数据（15 请求 / 11 应答 / 9 事件）+ 按域 RPC trait
                              └── re-export ──► abs_kb_core_handshake
                                                  └── 系统层握手（IpcReadyNotice / HandshakeNoticeKind）
```

- **`abs_kb_svc::v1::desktop::X` 这条路径保持不变**（含刚加的
  `IpcReadyNotice` / `HandshakeNoticeKind`），所以 `kb_core`、
  `kb_svc_servo_ipc`、`kb_core_rproxy` 与既有测试**一行都没改**——
  这正是保留聚合层的目的；
- 聚合用"别名整个 crate"而不是逐个 `pub use`：按 `AGENTS.md` 第 5 条不用通配符，
  同时"这份清单"仍然只在新 crate 的根文件里列举一次，不会出现两份要同步的清单；
- `abs_kb_core_handshake` 的**唯一**调用方改动是 `kb_core_starter`：直接依赖它；
- 移动用 `git mv` 完成，`git diff` 会显示为改名 + 路径重写。

### 10.4 新增文档

| 文件 | 内容 |
| :--- | :--- |
| `kb_svc/crates/abs_kb_svc/README.md` | 聚合层：聚合出什么、为什么保留、**什么时候不必经过它** |
| `kb_svc/crates/abs_kb_svc_v1_desktop/README.md` | 原来的 `abs_kb_svc/README.md` 整体搬过来（路径与层级章节按新布局改写） |
| `kb_svc/crates/abs_kb_core_handshake/README.md` | 系统层：属于哪一层、为什么独立、谁发谁收、承诺什么 |
| 根 `README.md` §2 / §3 / §6 | crate 分工表加入三个协议 crate 与 `kb_core_starter`；请求路径图标注两个层次的出处 |

### 10.5 验证

- `cargo test -p abs_kb_core_handshake -p abs_kb_svc_v1_desktop -p abs_kb_svc -p kb_core -p kb_core_starter`：
  **全绿**（系统层 2 单元；桌面协议 23 单元 + 6 契约；聚合层 1 文档测试；`kb_core` 27；starter 2 + 7 + 3）；
- `cargo check --workspace --all-targets`：无错误；
- 拆分**没有改任何线格式**：`IpcReadyNotice` 的 JSON 与 postcard 测试原样通过。

> 与本轮无关的既有告警（不是这次拆分引入的，根 `Cargo.toml` 现在没有
> `[workspace.lints.*]`，所以它们重新显形了）：`abs_art-bridge` / `tokio` 是"声明了但
> 没有成员使用的 workspace 依赖"；`kb_rig_llm_v1_agent` 有 4 个未使用依赖；
> `abs_llm` 的 `try_trait_v2` 声明了但没用到。这些是清单层面的清理，另行处理。
