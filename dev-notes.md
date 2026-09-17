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

## 1. 当前进度

| 能力 | 状态 | 说明 |
| :--- | :---: | :--- |
| 双监听器（TCP + Unix domain socket） | ✅ | 同一份路由表，见 `kb_svc_salvo::server` |
| UDS 上的 WebSocket 长连接 | ✅ | PoC 已验证，见 `kb_svc_salvo::poc` |
| 仿 DSH 聊天界面 + 增量渲染 | ✅ | `kb_svc_salvo/src/web/assets/` |
| LLM 服务与 API key 配置（文件 + 界面） | ✅ | `kb_svc_salvo::settings` |
| 浏览器 ↔ 插件两段转发 | ✅ | `kb_svc_salvo::hub` / `wire` / `plugin` / `web_ws` |
| 启动编排与优雅退出 | ✅ | `kb_svc_salvo::launch`（`kb_core` 唯一入口） |
| `kb_rig_llm_v1_agent`（rig 直连 LLM） | 🚧 | 由原 `kb_rig_llm` 改名；本阶段实现 |
| `kb_rig_llm_v1_adapt`（rig → `abs_llm::v1`） | 🚧 | 本阶段新增 |
| 浏览器侧帧格式统一为 `abs_llm::v1` | 🚧 | 本阶段改造 `wire.rs` |
| 检索 / 知识库 / Turso | ❌ | 尚未开始 |

**验收现状**：`cargo test --workspace` 43 项全绿；界面、设置接口、双监听器、
优雅退出均已实机跑通。

**已知限制**：API key 明文落盘；无鉴权、无 TLS；没有对话历史（刷新即清空）；
TCP 上也能访问 `/ws/plugin`。详见 `kb_svc_salvo::launch` 与 `kb_svc_salvo::settings`
的模块文档、以及 `kb_core/README.md`。

## 2. rig 接入与格式统一的决策（已确认）

这一组决策决定了下一阶段的实现方向，实施前已经确认，逐条记录如下。

> **⚠️ §2.1 已被取代（2026-09-17）**：团队已决定把进程间通信由 socket/HTTP 改为共享内存方向的 IPC，
> 并要求 `abs_kb_svc` 承担"所有 IPC 语义所需的数据类型与信道抽象，以及一个运行时无关的异步 RPC 业务接口"。
> 因此"`abs_kb_svc` 保持不动、插件协议留在 `kb_svc_salvo`"不再成立；`kb_svc_salvo` 本身也在废弃之列。
>
> 后续演进（同日）：共享内存方案首选候选 iceoryx2 经实测后被**放弃**（它要求业务类型改写为定长
> POD 或加一层镜像类型转换），改为研究 `servo/ipc-channel`。相关文档：
> - 选型与决策：[`dev-notes/abs_kb_svc-20260917-1254.md`](dev-notes/abs_kb_svc-20260917-1254.md)（**以此文为准**）
> - iceoryx2 可行性研究（已转为背景资料）：[`dev-notes/kb_svc_iceoryx2-20260917-1203.md`](dev-notes/kb_svc_iceoryx2-20260917-1203.md)
>
> `abs_kb_svc` 的定位与接口形状见其 [`README.md`](kb_svc/crates/abs_kb_svc/README.md)。
> §2.2 / §2.5 关于"rig 数据在哪一侧翻译"的讨论仍有效，但受新方案影响需要重新确认。

### 2.1 不在 `abs_kb_svc` 里抽象插件协议（已被取代，见上方说明）

插件与本次试验的通信协议**先定义在 `kb_svc_salvo` 内部**，不放进抽象层 `abs_kb_svc`。
等协议在真实使用中稳定下来之后，再考虑上提为抽象。

因此在本阶段：

- `abs_kb_svc` 保持不动（仍是空的 `lib.rs`）；
- 插件协议的帧定义留在 `kb_svc_salvo`（现有 `wire.rs` 一脉）；
- 只有真正与 LLM 语义相关的类型才通过 `abs_llm::v1` 表达。

### 2.2 插件拆成两个 crate：`agent` 与 `adapt`

| crate | 位置 | 职责 |
| :--- | :--- | :--- |
| `kb_rig_llm_v1_agent` | `kb_plugins/crates/kb_rig_llm_v1_agent` | 由原 `kb_rig_llm` 改名而来；用 rig 直接与**任意 rig 支持的 LLM 服务商**通信；自己保留完整对话上下文 |
| `kb_rig_llm_v1_adapt` | `kb_plugins/crates/kb_rig_llm_v1_adapt` | 把 rig 的原始数据转换成 `abs_llm::v1` 的数据；**具体载荷与线上表示主要由这里决定** |

依赖方向：

```text
kb_rig_llm_v1_agent  ──(原样 rig 数据)──►  UDS  ──►  kb_svc_salvo
                                                          │
                                                          └─ 调用 kb_rig_llm_v1_adapt
                                                             （rig 数据 → abs_llm::v1）
                                                          │
                                      浏览器 ◄──(abs_llm::v1 形状的帧)──┘
```

即：**转换发生在 `kb_svc_salvo` 进程内**，而不是插件进程内。插件只负责「说话」，
`kb_svc_salvo` 负责「翻译」，浏览器只看到 `abs_llm::v1` 的词汇。

### 2.3 对话上下文归 agent 所有，但数据原样上报

`kb_rig_llm_v1_agent` **自己保留整个对话上下文**（多轮对话由它维护），
同时把**原样的 rig 数据**传送给 `kb_svc_salvo`。

这样做的好处是：agent 侧可以直接复用 rig 的多轮能力，而 `kb_svc_salvo` 不需要
理解 rig 的上下文结构，只需要把原始数据交给 `kb_rig_llm_v1_adapt` 转换。

### 2.4 浏览器侧的通信格式统一为 `abs_llm::v1`

`kb_svc_salvo` 与「客户端」（目前就是它自己提供的网页端）之间的帧格式，
统一采用 `abs_llm::v1` 的词汇：`role`、`LogicOutput`（answer / reasoning /
function_call / dynamic_search_call / static_search_call）、`FinishReason`、
`TrUsage` 的 `input_tokens` / `output_tokens` / `total_tokens`、`Capabilities`。

因此 `wire.rs` 中浏览器侧的那一半需要按这套词汇重写（插件侧的那一半保持原样透传）。

### 2.5 `abs_llm` 临时引入 serde，但具体表示仍主要由 adapt 决定

**这是一个临时决策**：为了先把逻辑跑通，`abs_llm` 引入 `serde` 依赖，
并给其中少量纯数据 `enum` / `struct`（例如 `Role`、`Capabilities`、
`LogicOutput`、`FinishReason`）派生 `Serialize` / `Deserialize`。
`abs_llm` 仍然保持 `no_std`，serde 使用 `default-features = false`。

之所以说「具体 serde 逻辑大部分仍在 `kb_rig_llm_v1_adapt`」：`abs_llm`
里绝大多数内容本来就是 `trait` 和与 wire 无关的抽象类型，真正需要上线、
需要处理缺省字段、rename、tag、原始 rig payload 的仍是 adapt 的具体载荷
类型（`TextDelta`、`ToolCall`、`Usage`、`AdaptedEvent` 等）。

当前边界是：

- `abs_llm` 只负责让跨进程复用的纯语义类型具备基本的 serde 能力；
- `kb_rig_llm_v1_adapt` 负责 rig 原始数据 → `abs_llm::v1` 的语义映射，
  以及这些具体载荷在线上长什么样。

等逻辑稳定后，再重新评估 serde 是否应该收回 adapt，或进一步下沉/上提。

## 3. 代码即文档：设计与决策落位

原来写在本文件里的设计与决策，已经随实现迁移到对应代码的模块文档与 README 中。
需要查设计时按下表去找，不要再回到本文件：

| 主题 | 落位 |
| :--- | :--- |
| 进程分工、`kb_svc_salvo` 只做库、启动编排只在 `launch` | `kb_svc_salvo/src/launch.rs` 模块文档；`kb_core/src/main.rs` 模块文档 |
| 双监听器、一份路由表、监听地址与权限 | `kb_svc_salvo/src/server.rs` 模块文档 |
| UDS 上跑 WebSocket 的验证与结论 | `kb_svc_salvo/src/poc.rs` 模块文档 + `tests/poc_uds_websocket.rs` |
| socket 文件名（日期 + UUID）、权限、清理 | `kb_svc_salvo/src/plugin_socket.rs` 模块文档 |
| 两段线协议的全部帧与字段 | `kb_svc_salvo/src/wire.rs` 模块文档 |
| 会话状态、广播、错误类别 | `kb_svc_salvo/src/hub.rs` 模块文档 |
| 用户配置的路径、格式、注释保留、安全取舍 | `kb_svc_salvo/src/settings.rs` 模块文档 |
| HTTP 路由表 | `kb_svc_salvo/src/web.rs` 模块文档 |
| 前端资源目录、内嵌与 `--assets-dir` 覆盖 | `kb_svc_salvo/src/assets.rs` 模块文档 |
| 仿 DSH 的观感取舍与增量渲染策略 | `web/assets/app.css` 与 `web/assets/app.js` 顶部注释 |
| 依赖分层（哪些放 workspace、哪些放各自 crate） | 根 `Cargo.toml` 与各 crate `Cargo.toml` 的注释 |
| 命令行参数、环境变量、实测命令与预期结果 | `kb_core/README.md` |

## 4. 尚未决策的事项

这些需要团队拍板后才能动代码（按 `AGENTS.md` 第 1 条，涉及公开约定的变更须先确认）：

1. ~~插件协议放抽象层还是实现层~~ → ✅ 见 §2.1：先留在 `kb_svc_salvo`。
2. ~~rig 适配 crate 的命名与导出~~ → ✅ 见 §2.2：命名定为 `kb_rig_llm_v1_adapt`，
   职责是「rig 数据 → `abs_llm::v1`」。
3. ~~「原样透传」的边界~~ → ✅ 见 §2.3 / §2.5：插件侧原样传，转换在
   `kb_svc_salvo` 内、由 `kb_rig_llm_v1_adapt` 落地具体表示。
4. ~~对话上下文的归属~~ → ✅ 见 §2.3：由 `kb_rig_llm_v1_agent` 自己保留。
5. ~~`abs_llm` 是否引入 serde~~ → ✅ 见 §2.5：临时引入；只为纯语义类型派生，
   具体线上表示仍优先留在 `kb_rig_llm_v1_adapt`，后续再优化。
6. **多会话**：当前是单会话内存态。协议里的 `turn_id` 是否需要升级为
   `session_id` + `turn_id`？既然上下文现在归 agent 所有，这一步需要和
   agent 侧的会话管理一起设计。
7. **`kb_core` 与 `kb_svc_salvo` 的最终拆分边界**：现在两者「视为一体」，
   将来拆开时哪些模块留在服务库、哪些上移到核心进程？
8. **多 provider 的配置形态**：`settings.rs` 目前用 `provider` 字符串 +
   `base_url` + `model` 描述服务。rig 各 provider 的构造参数不同（有的要
   `base_url`，有的要 region），是否需要 per-provider 的配置结构？

## 5. 剩余工作

| 阶段 | 内容 | 估时 |
| :---: | :--- | :---: |
| 4 | `kb_rig_llm_v1_agent`：rig 直连 provider、自持上下文、原样上报 | 1.5 天 |
| 5 | `kb_rig_llm_v1_adapt`：rig 数据 → `abs_llm::v1`，并落地具体载荷的 serde 表示 | 1 天 |
| 6 | `kb_svc_salvo`：接入 adapt、浏览器帧格式改为 `abs_llm::v1` | 1 天 |
| 7 | `PluginSupervisor`：spawn agent、注入 socket 路径、退出监测与退避重启 | 0.5 天 |
| 8 | `ServerHandle::stop_graceful` 替换当前的「等 3 秒或 abort」 | 0.5 天 |
| 9 | 浏览器端到端测试（Playwright）：流式渲染、设置面板、取消、重连 | 1 天 |
| 10 | 其余验收项：插件断线自动重启并提示、取消的端到端时效 | 0.5 天 |

## 6. 环境提示

本机 `~/.cargo/registry` 与 `~/.cargo/git` 处于只读文件系统，`cargo fetch` 无法写入
缓存，会以 `Read-only file system (os error 30)` 失败。

**临时绕行（推荐）**：把缓存复制到 `/tmp` 下可写的 `CARGO_HOME`：

```bash
cp -r ~/.cargo/{registry,git} /tmp/cargo-home/   # 复制到可写的 CARGO_HOME
cp ~/.cargo/config.toml /tmp/cargo-home/
CARGO_HOME=/tmp/cargo-home cargo test --workspace
```

**工作区内的临时缓存（可选）**：如果希望绕行缓存跨会话保留、并且不落在系统 `/tmp`
里，也可以把它放到项目根目录的 `external/` 下：

```bash
mkdir -p external/cargo-home
cp -r ~/.cargo/{registry,git} external/cargo-home/
cp ~/.cargo/config.toml external/cargo-home/
CARGO_HOME="$PWD/external/cargo-home" cargo test --workspace
```

`external/cargo-home` 只承载本机沙箱需要的 Cargo 缓存，不作为项目代码提交；
`.gitignore` 中已按 `external/cargo-home/` 忽略它。

这属于本机沙箱限制，不是项目配置问题；正常开发机上无需这样处理。
