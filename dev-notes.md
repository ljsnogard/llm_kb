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
| `kb_rig_llm` 插件进程 | ❌ | 下一个任务；现在提问会提示「插件未连接」 |
| 检索 / 知识库 / Turso | ❌ | 尚未开始 |

**验收现状**：`cargo test --workspace` 43 项全绿；界面、设置接口、双监听器、
优雅退出均已实机跑通。

**已知限制**：API key 明文落盘；无鉴权、无 TLS；没有对话历史（刷新即清空）；
TCP 上也能访问 `/ws/plugin`。详见 `kb_svc_salvo::launch` 与 `kb_svc_salvo::settings`
的模块文档、以及 `kb_core/README.md`。

## 2. 代码即文档：设计与决策落位

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

## 3. 尚未决策的事项

这些需要团队拍板后才能动代码（按 `AGENTS.md` 第 1 条，涉及公开约定的变更须先确认）：

1. **`abs_kb_svc` 的定位**：UDS 插件协议应定义在 `abs_kb_svc`（抽象层）还是
   `kb_svc_salvo`（实现层）？放在抽象层能让 `kb_rig_llm` 不依赖具体实现，
   但抽象层要引入线协议依赖。
2. **rig 适配 crate 的命名与导出**：放在 `kb_plugins/crates/` 下的新 crate 叫什么？
   导出的是「rig 类型 → `abs_llm::v1`」的转换器，还是直接导出实现了 `TrLlmService`
   的 provider？
3. **「rig 数据原样透传」的边界**：rig 的流式事件与 `abs_llm::v1::LlmRespEvent`
   并非一一对应，落到 `wire` 协议时是保留 rig 的原始 JSON，还是先做最小投影？
4. **对话上下文的归属**：`TrConversation` 由服务端持有，还是每轮把上下文整体下发给插件？
5. **多会话**：当前是单会话内存态。协议里的 `turn_id` 是否需要升级为
   `session_id` + `turn_id`？
6. **`kb_core` 与 `kb_svc_salvo` 的最终拆分边界**：现在两者「视为一体」，
   将来拆开时哪些模块留在服务库、哪些上移到核心进程？
7. **`abs_llm` 抽象层是否要引入 `serde`**：`kb_svc_salvo` 自己定义了线协议类型
   （`wire.rs`）。若将来把协议放进 `abs_kb_svc`，抽象层的依赖面会随之扩大。

## 4. 剩余工作

| 阶段 | 内容 | 估时 |
| :---: | :--- | :---: |
| 4 | `PluginSupervisor`：spawn `kb_rig_llm`、注入 socket 路径、退出监测与退避重启 | 0.5 天 |
| 5 | `kb_rig_llm` + rig 适配 crate：UDS 客户端 + `abs_llm::v1` 实现 | 2 天 |
| 6 | `ServerHandle::stop_graceful` 替换当前的「等 3 秒或 abort」 | 0.5 天 |
| 7 | 浏览器端到端测试（Playwright）：流式渲染、设置面板、取消、重连 | 1 天 |
| 8 | 其余验收项：插件断线自动重启并提示、取消的端到端时效 | 0.5 天 |

## 5. 环境提示

本机 `~/.cargo/registry` 与 `~/.cargo/git` 处于只读文件系统，`cargo fetch` 无法写入
缓存，会以 `Read-only file system (os error 30)` 失败。绕行方式：

```bash
cp -r ~/.cargo/{registry,git} /tmp/cargo-home/   # 复制到可写的 CARGO_HOME
cp ~/.cargo/config.toml /tmp/cargo-home/
CARGO_HOME=/tmp/cargo-home cargo test --workspace
```

这属于本机沙箱限制，不是项目配置问题；正常开发机上无需这样处理。
