# kb_admin_desktop

知识库的桌面客户端（Flutter）。通过 HTTP / WebSocket 与 `kb_core` / `kb_svc_salvo`
通信，提供工作区管理、提问与文件浏览。

## 当前进度

| 能力 | 状态 | 说明 |
| :--- | :---: | :--- |
| 三栏框架（工作区列表 / 对话 / 文件浏览器） | ✅ | 对齐 DSH v0.1.5-rc 的布局与动效，见下节 |
| 侧边栏折叠成图标轨道 | ✅ | 300ms 滑动 + 交叉淡入淡出；窄于 1024px 自动折叠 |
| 文件浏览器默认隐藏、按钮调出 | ✅ | 列头右上角的开关；面板从右边缘滑入，见 `widgets/app_shell.dart` |
| 设置面板配置 LLM 服务与 API key | ✅ | 本地持久化；字段与服务端 `/api/settings*` 对齐 |
| 工作区 / 会话 / 消息 | 🚧 | 本地内存 + `shared_preferences`；服务端还没有对应接口 |
| 文件浏览器内容 | ❌ | 只有骨架与空状态，等 `kb_core` 提供目录接口 |
| HTTP / WebSocket 通道 | ❌ | 尚未接入 `/api/settings*` 与 `/ws/chat` |
| Markdown 渲染 | ❌ | 目前只切分 ``` 围栏为代码块，与网页端 `app.js` 的处理一致 |

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
flutter test                        # 29 项；golden 组默认跳过
flutter run -d linux
```

### 视觉基线

`test/golden/` 下有四张参考图（深色默认态、折叠侧边栏 + 打开文件面板、浅色主题、
设置面板）。它们依赖本机安装的中文字体，因此默认跳过：

```bash
flutter test --run-skipped --tags golden --update-goldens test/golden
```

### Rust 侧的工具链与构建配置

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
