# llm_kb

`llm_kb` 是个人知识库应用，其中的知识内容主要来自 LLM 服务生成的内容以及用户个人编辑整理。  

`llm_kb` 本身是一个分布式应用，它的主要功能——维护一个以 Turso 为载体的知识数据库，进行增删查改等操作，并对外提供搜索、检索等功能。
其他扩展的功能都将以插件的形式存在，目前规划了如下的插件功能：
- 与 LLM 进行通信，并将 LLM 生成的数据存进知识库
- 知识库的编辑和管理界面

## 项目架构

### kb_svc 目录

这是存放知识库的基本骨架，其中包括

| 子项目 | 描述 |
| :---:  | :--- |
| `kb_core` | 知识库的主进程，主要功能是围绕一个 Turso 数据库提供增删查改和模糊搜索、向量搜索，以及插件管理 |
| `abs_llm` | 对各类 LLM 的服务进行提供抽象和统一的接口 |
| `abs_kb_svc` | `kb_core` 中对各类插件的抽象和调用接口规范 |
| `kb_svc_salvo` | 使用 `Salvo` 框架以及 http3 协议来作为 `kb_core` 的第一个版本实现，这也是第一个版本中各个插件与主进程的通信方式 |

### kb_plugins 目录

这是存放知识库应用各种基本插件的目录，其中包括

| 子项目 | 描述 |
| :--- | :--- |
| `kb_rig_llm_v1_agent` | 使用 `rig` 实现与 LLM 对话的插件 |
| `kb_rig_llm_v1_adapt` | 用于将 `rig` 中实现的 LLM 上下文相关的对象，转换为符合 `abs_llm` （v1） 定义的对象 |

### kb_clients 目录

这是构建知识库客户端应用的目录，目前只提供全功能的客户端 `kb_admin_desktop`。
未来将提供同样基于 flutter 实现的移动端版本。

| 子项目 | 描述 |
| :--- | :--- |
| `kb_admin_desktop` | 全功能的知识库编辑、管理，桌面客户端。|

## 构建环境

工具链由仓库根的 `rust-toolchain.toml` 锁定为 **nightly**：`abs_llm` 使用了
`#![feature(try_trait_v2)]`（见 `kb_svc/crates/abs_llm/src/lib.rs`），stable 编译不过。
`rustup` 会自动按该文件准备工具链与 `rustfmt` / `clippy` 组件，无需手工 `rustup default`。

客户端 `kb_admin_desktop` 里的 flutter_rust_bridge 子项目由 Cargokit 驱动，
它不会读取 `rust-toolchain.toml`（`rustup run` 会覆盖工具链文件），因此另有一份
`kb_clients/kb_admin_desktop/rust/cargokit.yaml` 把工具链对齐到同一条通道。
