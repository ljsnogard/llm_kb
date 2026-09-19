# `kb_admin_desktop`：窗口最小尺寸 + 新建会话的草稿行

- 日期：2026-09-19 14:35
- 状态：**本轮实施记录**（两个小修复）。
- 范围：
  1. 窗口有一个**最小尺寸**，拖动缩放时不能比它更小；
  2. 修「新建会话」反直觉的体验：点完按钮界面必须立刻有变化。
- 前置：[`kb_admin_desktop-20260919-1237.md`](kb_admin_desktop-20260919-1237.md)：
  草稿会话模型（本轮把它补成"界面上看得见的草稿"）。

---

## 1. 窗口最小尺寸：800 × 520

### 1.1 数值从哪来

由布局反推，写在 [`DswLayout`](../kb_clients/kb_admin_desktop/lib/src/theme/dsw_tokens.dart) 里：

| 维度 | 取法 |
| :--- | :--- |
| 宽 | 折叠轨道 56（`sidebarCollapsed`）+ 中间区最小 400（`centerMin`）+ 右侧栏最小 300（`rightbarMin`）= 756，**取 800** 留余量 |
| 高 | 列头 76 + 输入区约 64 + 若干行消息，**取 520** |

窗口窄于 `sidebarAutoCollapse`（1024）时侧边栏本来就会自动折叠，所以宽度这一项按
"折叠轨道"算就是最紧的那种形态——800 还顺带保证"打开右侧文件面板时中间区仍有 400"。

`test/column_geometry_test.dart` 里加了一条断言守着这个关系
（`minWindowWidth ≥ sidebarCollapsed + centerMin + rightbarMin`）。

### 1.2 为什么必须写进原生 runner

Flutter 没有"设置窗口最小尺寸"的 Dart API——窗口归操作系统，缩放时的下界由窗口
管理器执行。所以三个平台的 runner 各自设一次（**原生读不到 Dart 常量，这三处是
硬编码的同一个数值**）：

| 平台 | 位置 | 写法 |
| :--- | :--- | :--- |
| Linux | `linux/runner/my_application.cc` | `gtk_window_set_geometry_hints(..., GDK_HINT_MIN_SIZE)` |
| Windows | `windows/runner/win32_window.cpp` | `WM_GETMINMAXINFO` 里写 `ptMinTrackSize`（按 `FlutterDesktopGetDpiForHWND` 换算物理像素） |
| macOS | `macos/Runner/MainFlutterWindow.swift` | `self.minSize = NSSize(width: 800, height: 520)` |

另一条路是引入 `window_manager` 之类的插件；**没有采用**：它要多一个依赖 + 插件注册，
而这三个 runner 本来就在仓库里。

### 1.3 验证

本机只有 Linux 能构建/运行。除了 `flutter build linux --debug` 通过之外，还用
`xprop` 直接读了窗口的 `WM_NORMAL_HINTS`（这是 GTK 交给窗口管理器执行的那份声明）：

```console
$ xprop -id 0x200003 WM_NORMAL_HINTS
WM_NORMAL_HINTS(WM_SIZE_HINTS):
        program specified minimum size: 800 by 520
        program specified base size: 800 by 520
        window gravity: NorthWest
```

Windows / macOS 两处只做了 API 层面的核对（`FlutterDesktopGetDpiForHWND` 在
`flutter_windows.h` 里确实存在；`NSSize` 是 `NSWindow.minSize` 的标准用法），
**没有实机编译**——本机没有那两个工具链。

---

## 2. 新建会话：草稿行要立刻可见

### 2.1 问题

上一轮把「新会话」做成了客户端草稿（名字由 `kb_core` 从第一个问题推导，空会话不落盘），
但**草稿只影响对话区**：点完按钮侧边栏一行都不多，用户不知道发生了什么，得先盲打
一个问题才看到会话冒出来。用户的口径是：

> 客户端新建会话时并不真正向 `kb_core` 发请求，而是先在左侧栏**具体的工作区下**建造
> 一个名为「新会话」的会话（内部状态为未同步，即没有会话 ID），等用户输入第一个问题
> 并发送之后，才真正向 `kb_core` 新建会话并发送问题。

### 2.2 修法

`ConnectionController` 增加一个集合，记录**哪些工作区里有草稿**：

```text
Set<String> _draftWorkspaces        // 每个工作区最多一个草稿
startDraftSession(workspaceId)      // 点「新会话」/ 工作区行的 + → 加入并选中
selectDraftSession(workspaceId)     // 点那条草稿行 → 只是把界面切过去
discardDraftSession(workspaceId)    // 放弃：纯客户端操作，不发任何请求
```

侧边栏在对应工作区下渲染一条 `_DraftSessionRow`：

- 标题「新会话」，右侧灰字「**未同步**」；悬停时出现「放弃这个新会话」（一个 ×）；
- 点它 = 选中这个草稿（对话区标题也随之变成「新会话」）；
- **不受工作区折叠影响**：它代表"正在进行的工作"，收起服务端会话列表时也留在原地，
  这样无论是点顶部的「新会话」还是工作区行上的 `+`，都立刻看得到；
- 首次提问成功后从 `_draftWorkspaces` 移除，那一行换成真正的会话行（标题就是问题）。

失效规则与上一轮一致：断开 / 重连清空草稿；删掉工作区时连带它的草稿；刷新工作区列表
**不**清草稿（草稿是客户端状态），只把挂靠工作区已经消失的那些剔除。

---

## 3. 验证

| 项 | 命令 | 结果 |
| :--- | :--- | :--- |
| Dart 静态检查 | `flutter analyze` | **No issues found** |
| Dart 测试 | `flutter test` | **51 项全绿**（新增：草稿行可见性、放弃草稿不发请求、最小窗口尺寸与三栏下界的关系） |
| Linux 构建 | `flutter build linux --debug` | 通过（native runner 的 geometry hints 编译进来，`nm -D` 能看到 `gtk_window_set_geometry_hints`） |
| 最小尺寸 | `xprop -id <win> WM_NORMAL_HINTS` | `program specified minimum size: 800 by 520` |
| 真 GUI | `xvfb-run` 跑真 App 连真 `kb_core` | 正常启动、连接、列出工作区与会话（runner 改动没有影响启动） |

Rust 侧本轮**一行未改**，因此没有重跑 `cargo test`。

---

## 4. 没做 / 注意

1. **Windows / macOS 的最小尺寸没有实机验证**（本机没有工具链），改动是标准 API 用法，
   但第一次在那些平台构建时要留意；
2. 草稿目前每个工作区最多一个，也没有"草稿跨重启恢复"——它是纯内存状态；
3. 工作区折叠时草稿行仍然显示，这是刻意的（见 §2.2）；如果以后觉得碍眼，可以改成
   "草稿所在工作区强制展开"。

---

## 5. 相关文档

- [`kb_admin_desktop-20260919-1237.md`](kb_admin_desktop-20260919-1237.md)：
  改名、会话命名、"空会话不落盘"与草稿模型；
- `kb_clients/kb_admin_desktop/lib/src/theme/dsw_tokens.dart`：`minWindowWidth` / `minWindowHeight`；
- `kb_clients/kb_admin_desktop/lib/src/state/connection_controller.dart`：草稿集合的四个入口。
