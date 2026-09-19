# kb_admin_desktop

知识库的桌面客户端（Flutter）。**通过 `kb_core` 的协议**（本机 IPC 或经
`kb_core_rproxy` 的 TCP）提供工作区管理、提问与文件浏览。

> 早期版本走 `kb_svc_salvo` 的 HTTP / WebSocket；那套通道已废弃，现在是
> `abs_kb_svc_v1_desktop` 的协议 + `kb_svc_servo_ipc` 的传输。

## 当前进度

| 能力 | 状态 | 说明 |
| :--- | :---: | :--- |
| 三栏框架（工作区列表 / 对话 / 文件浏览器） | ✅ | 对齐 DSH v0.1.5-rc 的布局与动效，见下节 |
| 侧边栏折叠成图标轨道 | ✅ | 300ms 滑动 + 交叉淡入淡出；窄于 1024px 自动折叠 |
| 文件浏览器默认隐藏、按钮调出 | ✅ | 列头右上角的开关；面板从右边缘滑入，见 `widgets/app_shell.dart` |
| **连接 `kb_core`：首次运行选连接方式 → 连上 → 看工作区 / 会话** | ✅ | 左上角是「主机名 + 切换连接」按钮；工作区与会话来自 `kb_core`，见下节 |
| **工作区 / 会话的增删**（在 `kb_core` 所在主机上执行） | ✅ | 列表右上角 `+` 新建工作区；工作区行 / 会话行悬停出现新建、改名与删除；见下节 |
| **会话正文 + 提问** | ✅ | 选中会话读 `GetSession` 渲染到对话区；提问发给 `kb_core`（当前是临时模拟的 LLM，把问题逆序输出并落盘） |
| **工作区 / 会话改名** | ✅ | 行悬停的改名按钮；走 `RenameWorkspace` / `RenameSession`，改动落在 `kb_core` 的记录里 |
| **窗口最小尺寸 800×520** | ✅ | 三个平台的 runner 各自设一次（Flutter 没有对应的 Dart API）；数值与 `DswLayout.minWindowWidth/Height` 一致 |
| 设置面板配置 LLM 服务与 API key | ✅ | 本地持久化；字段与服务端 `/api/settings*` 对齐 |
| 工作区 / 会话 / 消息 | 🚧 | 连上 `kb_core` 后列表与正文都以服务端为准；未连接时整体退回本地 |
| 文件浏览器内容 | ❌ | 只有骨架与空状态，等 `kb_core` 提供目录接口 |
| Markdown 渲染 | ❌ | 目前只切分 ``` 围栏为代码块，与网页端 `app.js` 的处理一致 |

## 连接 kb_core（当前已打通的体验）

```text
启动
  ├─ 读客户端自己的连接配置（TOML，见 kb_client_config）
  │    ├─ 没有配置 → 弹「连接 kb_core」：选 / 填一条连接方式 → 写回配置 → 连
  │    └─ 有配置   → 自动连缺省那条
  ├─ 系统层握手：起本机 kb_core / 附着到本机 kb_core / 连远程网关
  ├─ 应用层握手：Request::Hello → Reply::Hello（版本对不上会被明确拒绝）
  └─ 连上之后：侧边栏列出 kb_core 上的工作区；展开一个工作区时按需拉它的会话
```

界面上的位置：

- **左上角是「主机名」按钮**：名字就是连接配置里的 `name`（客户端自己起的花名，
  与握手协议无关）。点开列出所有已配置的连接，选一条就切过去（旧的连接被替换）；
  最后一项「管理连接方式…」打开连接对话框。折叠成图标轨道时是一个 `dns` 图标，
  颜色反映状态（灰 / 黄 / 绿 / 红）。
- **工作区列表**：连上之后显示的是 `kb_core` 的数据；每个工作区展开时会去拉
  它的会话，拉过就缓存；列表右上角是**新建工作区**与**刷新**两个按钮。
  工作区行悬停时出现「工作区改名 / 在此工作区新建会话 / 删除工作区」；会话行
  悬停时出现「会话改名 / 删除会话」。删除工作区会先确认（服务端级联删掉它的会话）。
- **会话正文**：点一条会话，右侧对话区从 `GetSession` 读回它的全部消息并渲染；
  在下方输入框提问会走 `Ask`——`kb_core` 现在的回答是**把问题逆序输出**
  （临时模拟的 LLM），一问一答都会落盘，所以**重新连线仍然看得到**。
- **「新会话」是一个草稿**：点它（或工作区行上的 `+`）**立刻在那个工作区下多出一条
  「新会话」行**，右侧标着「未同步」——它还没有会话 ID，只在客户端存在。用户发出第一个
  问题时，客户端才把那条消息作为会话的首条内容提交给 `kb_core`，会话名由 `kb_core`
  从**问题的前若干字**推导，草稿行随之换成真正的会话行。悬停草稿行可以「放弃这个新会话」
  （纯客户端操作）；空会话因此永远不会落盘。会话行悬停的改名按钮可以随时改标题
  （留空则交回服务端重新推导）。
- **目录是 `kb_core` 所在主机上的路径**：客户端只把「名字 + 路径」提交给
  服务端，不碰自己这边的文件系统。
- 断开/未连接时，列表与对话区退回**本地**那一套（`AppController` 里的工作区），
  这样没有服务端也能起界面。

### 手动跑一遍

```bash
# 1. 起一个 kb_core（或者干脆让客户端自己起：连接方式选 local-launch）
cargo run -p kb_core -- --runtime-dir /tmp/kb/run --storage-dir /tmp/kb/data

# 2. 起客户端，首次运行会弹「连接 kb_core」
flutter run -d linux
#    - 「连接已在跑的本机 kb_core」→ 运行时目录填 /tmp/kb/run
#    - 或「启动一个本机 kb_core」→ 填 kb-core 路径 + 上面两个目录
# 3. 连上之后，直接在列表右上角的 + 里新建工作区与会话
```

配置文件在平台约定目录下（Linux：`~/.config/kb_admin_desktop/config.toml`），
也可以显式指定：

```bash
KB_ADMIN_DESKTOP_CONFIG=/tmp/kb-admin.toml flutter run -d linux
```

它的内容就是 `kb_client_config` 的 TOML（三种 `kind` 见
[`kb_client_conn_mgr`](../crates/kb_client_conn_mgr/README.md)）。

> **当前的边界**：工作区与会话的**增 / 删 / 改 / 查**、**会话正文**与**提问**都已
> 打通（`AddWorkspace` / `RemoveWorkspace` / `RenameWorkspace` / `ListWorkspaces`、
> `CreateSession` / `RemoveSession` / `RenameSession` / `ListSessions` /
> `GetSession`、`Ask`）。还差两块：
>
> - **流式生成**：现在是同步一问一答（`TrGeneration::ask` 回整份会话），
>   换成流式形状属于公开协议变更，见
>   [`dev-notes/kb_admin_desktop-20260918-1740.md`](../../dev-notes/kb_admin_desktop-20260918-1740.md) §2.1；
> - **真正的多连接**：左上角可以**切换**活动连接，但同一时刻仍只有一条；
>   "同时保持多条、各自保留缓存"还没做。
>
> 工作区"改路径"也没做——本轮只做了改名，见
> [`dev-notes/kb_admin_desktop-20260919-1237.md`](../../dev-notes/kb_admin_desktop-20260919-1237.md) §5。

## Rust 侧

界面之外的逻辑一律在 Rust 侧（`rust/`，flutter_rust_bridge），并通过
`lib/src/rust/` 暴露给 Dart。当前已就绪的是**连接**这一块：

```text
lib/src/rust/api/kb.dart       ← 生成的 Dart 接口（全部 Future<...>）
        │
rust/src/api/kb.rs             ← 扁平视图 + 持有已连上的客户端
        │
kb_clients/crates/kb_client_config   ← 连接配置（TOML、路径、首次生成）
kb_clients/crates/kb_client_conn_mgr ← 连接管理器（本机启动 / 本机附着 / 远程 TCP）
```

Dart 侧拿到的接口（都是 `Future`，因为 FRB 会把普通函数放到自己的工作线程池上）：

| 函数 | 用途 |
| :--- | :--- |
| `configFilePath()` / `loadConfig()` / `saveConfig(...)` | 读 / 写客户端自己的连接配置；`exists == false` 表示**首次运行** |
| `connectionKinds()` / `connectionKindDescription(kind)` | 界面填下拉框用的三种连接方式与说明 |
| `suggestedLocalConnection(name)` | 首次运行时预填的"启动本机 kb_core" |
| `connectTo(profile)` / `disconnect()` / `connectionState()` | 连接、断开、看当前连的是谁 |
| `listWorkspaces()` / `listSessions(workspaceId)` | 连接之后的查询 |
| `addWorkspace(name, path)` / `removeWorkspace(workspaceId)` | 新建 / 删除工作区；`path` 是 **kb_core 所在主机上**的目录 |
| `renameWorkspace(workspaceId, name)` | 工作区改名（只改展示名，磁盘目录不动） |
| `createSession(workspaceId, title, turnId, question)` / `removeSession(workspaceId, sessionId)` | 新建 / 删除会话；`turnId` 非空时把 `question` 作为首条消息一起提交（名字由服务端从问题推导） |
| `renameSession(workspaceId, sessionId, title)` | 会话改名；`title` 只有空白时由服务端重新推导 |
| `getSession(workspaceId, sessionId)` | 读取会话正文（摘要 + 全部消息） |
| `ask(workspaceId, sessionId, turnId, question)` | 提问；返回提问之后的会话内容（当前是模拟 LLM 的逆序回答） |

细节与设计取舍见 [`kb_client_conn_mgr`](../crates/kb_client_conn_mgr/README.md) 与
`dev-notes/` 下 `kb_admin_desktop-*` 的几份记录。

## 重新生成 FRB 绑定（改了 Rust 侧之后）

`lib/src/rust/` 下的东西**都是生成的**，不要手改。只要 Rust 侧的 **FFI 可见面**
变了，就必须重新生成；生成的产物跟源码一起提交。

### 什么时候要重新生成

| 改动 | 要不要重新生成 |
| :--- | :--- |
| `rust/src/api/**` 里增删函数、改函数签名、改返回类型 | ✅ 要 |
| 增删 / 改 `rust/src/api/**` 里的 `struct` / `enum` 的**字段**（FFI 会镜像它们） | ✅ 要 |
| 改 `flutter_rust_bridge.yaml`（`rust_input` / `dart_output` 等） | ✅ 要 |
| 只改函数体、内部逻辑；改 `kb_client_conn_mgr` 等依赖 crate 的实现 | ❌ 不要（FFI 面没变） |
| 改依赖 crate 的**代码**，但 `crate::api` 里的签名照旧 | ❌ 不要 |
| 把依赖 crate **改名**，但 `crate::api` 里的符号路径照旧 | ❌ 不要（渲染出来的 Dart 名从函数名来） |
| 给 `rust/Cargo.toml` 加依赖（只要没改 API 形状） | ❌ 不要 |

判断标准只有一条：**`lib/src/rust/api/*.dart` 里公开的名字/字段会不会变**。
拿不准就跑一次生成，看 `git diff` 是否为空。

### 前置条件

1. `flutter` / `dart` 在 `PATH` 上（`flutter --version` 能正常输出）；
2. `flutter_rust_bridge_codegen` 已安装，且**版本与 `rust/Cargo.toml` 里钉的
   `flutter_rust_bridge` 一致**（当前两边都是 `2.13.0`）：

   ```bash
   flutter_rust_bridge_codegen --version     # 期望 2.13.0
   grep '^flutter_rust_bridge' rust/Cargo.toml
   ```

   不一致时先对齐：`cargo install flutter_rust_bridge_codegen --version 2.13.0 --locked`；
3. 生成过程会调用 `flutter`（用来做 Dart 侧格式化与版本探测），所以它**要能写自己的
   SDK cache**。

### 步骤

```bash
# 1. 进 Flutter 工程根（flutter_rust_bridge.yaml 所在处）
cd kb_clients/kb_admin_desktop

# 2. 确认路径与（本机沙箱才需要的）CARGO_HOME
export PATH="/root/.cargo/bin:/root/develop/flutter/bin:$PATH"
export CARGO_HOME="$PWD/../../external/cargo-home"   # 本机沙箱：~/.cargo 只读

# 3. 生成（读 flutter_rust_bridge.yaml，不需要额外参数）
flutter_rust_bridge_codegen generate

# 4. Rust 侧编译
(cd rust && cargo check)

# 5. Dart 侧静态检查
flutter analyze
```

`flutter_rust_bridge.yaml` 的内容决定了它会看什么、写哪里：

```yaml
rust_input: crate::api          # 只扫这个模块；不要写整个 crate，原因见下
rust_root: rust/
dart_output: lib/src/rust
```

`rust_input` **只列 `crate::api`** 是有意的：一旦把协议 crate（如
`abs_kb_svc_v1_desktop`）也列进来，FRB 会把那里的类型也镜像一遍，其中**直接持有
跨 crate 类型的结构体会退化成 opaque 句柄**（实测 27 个）。所以 FFI 面只用本地的
扁平结构体，业务类型在 `rust/src/api/kb.rs` 里就地翻平。

### 生成会动哪些文件

| 文件 | 说明 |
| :--- | :--- |
| `rust/src/frb_generated.rs` | 整体重写 |
| `lib/src/rust/frb_generated.dart` / `.io.dart` / `.web.dart` | 整体重写 |
| `lib/src/rust/api/<模块>.dart` | 按 `crate::api` 的子模块逐个生成 |

> ⚠️ **删掉一个 api 模块时，它对应的 Dart 文件不会被自动删除**（`generate` 只写，
> 不清理）。要手动删：`rm lib/src/rust/api/<旧模块>.dart`，否则 `flutter analyze` 会
> 抱着一堆引用已删符号的代码报错。`lib/src/rust/third_party/` 同理（本项目不用它）。

### 生成完检查什么

1. `git status`：应当只看到上面那几张表里的文件 + 你改的 Rust 源码，**没有**意外文件；
2. `git diff lib/src/rust/api/<模块>.dart`：新函数/字段是不是都在，名字是不是你期望的
   （FRB 会把 `snake_case` 转成 `camelCase`，把 `ConnectionView` 直接当类名）；
3. `cargo check`（在 `rust/`）通过；
4. `flutter analyze` 无 issue；
5. 如果这次改的是**界面要用**的接口，顺手跑一下集成测试：
   `flutter test integration_test -d linux`（需要 clang/cmake/ninja/GTK 与显示服务，
   本机沙箱里用 `xvfb-run`）。

### 常见故障

| 现象 | 原因 / 处理 |
| :--- | :--- |
| `Error: Dart/Flutter toolchain not available` | `flutter` 不在 `PATH`，或 SDK cache 不可写（见下） |
| `Read-only file system`，指向 `flutter/bin/cache/engine.stamp.tmp` / `engine.realm` | SDK 目录只读。本机沙箱要把 Flutter SDK 的路径一起放开再跑 |
| `cargo check` 报 `cannot find \`xxx\` in \`api\`` | 生成的 `frb_generated.rs` 还是旧的：重新生成一次 |
| `error: lifetime bound not satisfied`，位置在 `frb_generated.rs` 的 `wrap_async` | FRB **2.13** 为 `async fn` 生成的代码在当前 nightly 上编译不过（rustc HRTB 限制）。把 `crate::api` 里的异步函数写成**同步函数 + `block_on`**（现在的做法，理由见 `rust/src/api/kb.rs` 的模块文档）；Dart 侧仍是 `Future` |
| `flutter analyze` 报引用了已删的函数 | 旧的 `lib/src/rust/api/*.dart` 没删干净，见上面的 ⚠️ |
| 生成的 Dart 里某个类型成了 opaque（`implements RustOpaqueInterface`） | 那个结构体直接持有了协议 crate 的类型；在 `crate::api` 里加一层扁平视图再映射 |

### 依赖 crate 变了但 FFI 面没变时

`kb_client_config` / `kb_client_conn_mgr`（以及它们背后的 `kb_svc` 协议 crate）都是用
path 依赖进来的：**改它们的实现不需要重新生成绑定**，但客户端 rust 侧要重新编译一次：

```bash
(cd rust && cargo check)     # 或 flutter run / flutter build
```

如果改的是它们的**公开 API 形状**（函数签名、结构体字段），先按上面的判断标准确认
FFI 面是否真的没变——变了就重新生成。

## 三栏框架

```text
┌──────────┬────────────────────────────┬──────────────┐
│ 工作区列表 │ 对话区                      │ 文件浏览器     │
│ 新会话    │ 面包屑 + 消息列表 + 输入框    │ 默认隐藏       │
│ 设置(左下) │                            │ 按钮调出       │
└──────────┴────────────────────────────┴──────────────┘
```

尺寸与动效取自 DSH 的 `ui-layout` / `ui-sidebar` / `ui-sidebar-right`，常量集中在
`lib/src/theme/dsw_tokens.dart`：

| 常量 | 值 | 出处 |
| :--- | :--- | :--- |
| 侧边栏默认 / 最小 / 最大 | 280 / 264 / 420 | `columns.ts` `SIDEBAR_*` |
| 侧边栏折叠轨道 | 56 | `SIDEBAR_COLLAPSED` |
| 自动折叠阈值 | 1024 | `SIDEBAR_AUTO_COLLAPSE` |
| 右侧栏最小 / 默认 / 最大 | 300 / 45% 视口 / 70% 视口 | `RIGHTBAR_*` |
| 中间区最小宽度 | 400 | `CENTER_MIN` |
| 列宽动画 | 300ms，`cubic-bezier(0.4, 0, 0.2, 1)` | `--ds-transition-duration-slow` / `--ds-ease-in-out` |

配色同样是 DSH 的两级令牌（`--dsw-static-*` → `--dsw-alias-*`）移植，
见 `lib/src/theme/dsw_tokens.dart` 与 `app_theme.dart`。

## 目录结构

```text
lib/
├── main.dart                     入口：读本地快照 → 组装 controller → runApp
└── src/
    ├── app.dart                  MaterialApp + 主题装配
    ├── models/                   chat_turn / chat_session / workspace / llm_service
    ├── services/local_store.dart shared_preferences 读写
    ├── state/app_controller.dart 布局、数据、配置的唯一状态源
    ├── theme/                    DSH 令牌与 ThemeData
    └── widgets/
        ├── app_shell.dart        三栏框架、列宽动画、拖拽分栏
        ├── common/               DSH 风格控件与自绘图标
        ├── sidebar/              左侧栏 + 工作区列表
        ├── conversation/         对话区 + 消息 + 输入框
        ├── files/                右侧文件面板
        └── settings/             设置面板
```

## 开发

```bash
flutter pub get
flutter analyze
flutter test                        # 51 项；golden 组默认跳过
flutter run -d linux
```

### 视觉基线

`test/golden/` 下有四张参考图（深色默认态、折叠侧边栏 + 打开文件面板、浅色主题、
设置面板）。它们依赖本机安装的中文字体，因此默认跳过：

```bash
flutter test --run-skipped --tags golden --update-goldens test/golden
```

## Rust 侧的工具链与构建配置

客户端自带一个 flutter_rust_bridge 的 Rust 子项目（`rust/`），它有两处需要留意：

1. **`rust/Cargo.toml` 里显式声明了空的 `[workspace]`。** 该 crate 位于仓库根
   workspace 的目录树内，却不想成为它的成员；不声明的话 cargo 会报
   「current package believes it's in a workspace when it's not」。

2. **`rust/cargokit.yaml` 把工具链钉到 nightly，这是必须的。** 仓库根的
   `rust-toolchain.toml` 把整个项目锁到 nightly（`abs_llm` 需要
   `#![feature(try_trait_v2)]`），但 Cargokit **看不到**它——Cargokit 固定用
   `rustup run <通道> cargo build` 调用，而 `rustup run` 会覆盖同目录树里的
   `rust-toolchain.toml`。不配置的话它走默认的 `stable`，于是 `flutter build linux`
   会在任何 nightly-only 的 rustflag（例如本机全局 `-Zpolonius=next`）上直接失败。
   Cargokit 的 `enum Toolchain` 只认 `stable` / `beta` / `nightly`，所以这里只能
   按通道对齐，写不了 `nightly-YYYY-MM-DD`。
