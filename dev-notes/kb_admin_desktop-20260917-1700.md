# `kb_admin_desktop`：本机能否只靠 Flutter SDK 做 GUI 测试（调查）

- 日期：2026-09-17 17:00
- 状态：**调查结论**（只读调查，未改动任何仓库文件）
- 问题：在本机上，是否有可能**仅依靠 Flutter SDK** 完成 GUI 相关的测试？

---

## 1. 结论

**不能。** 本机完全没有安装 Flutter / Dart SDK，`flutter` 与 `dart` 命令都是
退出码 127，因此**现在连最基础的 widget 测试都跑不起来**。
而且即便补上 SDK，集成测试还缺 Linux 桌面构建链与显示服务两层。

---

## 2. 证据（命令 → 观察到的结果）

| 检查项 | 命令 | 结果 |
| :--- | :--- | :--- |
| SDK 存在性 | `which flutter dart` | 无输出，退出码 1 |
| SDK 可用性 | `flutter --version` / `dart --version` / `flutter doctor -v` / `flutter devices` / `flutter test` | 全部 `No such file or directory`，退出码 127 |
| 全盘搜索 | `find / -maxdepth 4 -type f \( -name flutter -o -name dart \)` | 无结果 |
| 目录搜索 | `find / -maxdepth 6 -type d -iname 'flutter*'` | 只有 `/root/.oh-my-zsh/plugins/flutter`（zsh 补全插件，不是 SDK） |
| 包管理 | `dpkg -l \| grep -i flutter`；`snap` | 无；`snap` 不存在；`/opt` 为空 |
| 显示服务 | `echo $DISPLAY` / `$WAYLAND_DISPLAY` | 均为空 |
| 无头显示 | `which xvfb-run Xvfb` | 未安装 |
| X11 / GPU | `ls /tmp/.X11-unix`；`ls /dev/dri` | 都不存在 |
| 构建链 | `which clang cmake ninja pkg-config` | 全部 NOT FOUND（`gcc` / `g++` / `make` 存在） |
| GTK | `pkg-config --exists gtk+-3.0` | `pkg-config` 本身就没有，无法确认（大概率未装） |
| 工程状态 | `pubspec.lock` / `.dart_tool/` | lock 存在（已入库）；`.dart_tool/` 不存在 ⇒ 从未 `pub get` |

环境补充：Debian 13（trixie）；`/` 与 `/root` 对本会话**只读**，
只有工作区与 `/tmp`（3.8 G tmpfs）可写；网络可达（pub.dev:443、
storage.googleapis.com:443 均可连）；`apt` 候选齐全
（clang 1:19.0-63、cmake 3.31.6-2、ninja-build 1.12.1-1、pkg-config 1.8.1-4、
libgtk-3-dev 3.24.49-3、xvfb 2:21.1.16）。

工程内容：`pubspec.yaml` 依赖 `flutter`、`cupertino_icons`、
`shared_preferences`、`flutter_rust_bridge 2.13.0` 以及 path 依赖
`rust_lib_kb_admin_desktop`（由 `rust_builder` 里的 cargokit 构建）；
dev 依赖 `flutter_test`、`integration_test`、`flutter_lints`。
`test/` 下有 4 个 widget 测试与 `golden/`（被 `dart_test.yaml` 的 `golden` tag
默认跳过）；`integration_test/simple_test.dart` 会调用 `RustLib.init()`。

---

## 3. 分层可行性

| 层次 | 本机现状 | 说明 |
| :--- | :--- | :--- |
| **widget 测试（`flutter test`）** | ❌ 现在不能（缺 SDK）；补齐 SDK 后**可以** | 它走 `flutter_tester`，**不需要显示服务、不需要 GTK/X11**。现有 4 个测试都不调用 `RustLib.init()`，所以也不依赖 Rust 侧。 |
| **golden 测试** | ❌（同上前提之外还缺字体） | 被 `dart_test.yaml` 的 `golden` tag 默认跳过；且它依赖的中文字体 `/usr/share/fonts/opentype/source-han-cjk/SourceHanSansSC-Regular.otf` 本机不存在（`/usr/share/fonts` 下只有 truetype）。 |
| **集成测试（`flutter test integration_test -d linux`）** | ❌ 现在不能；补 SDK 后仍缺两层 | (a) 缺 clang / cmake / ninja / pkg-config / libgtk-3-dev；(b) 无 X11/Wayland，需要 `xvfb-run`；(c) `RustLib.init()` 要经 cargokit 编出 Rust cdylib，同样依赖上面那套构建链。软件渲染 + 无 `/dev/dri`，与真机渲染有差异。 |
| **真机 / 模拟器** | ❌ 不可行 | 无 Android/iOS 设备、无 GPU、无显示服务。 |

---

## 4. 最小补齐路径

1. **Flutter SDK（含 Dart）**——最关键的缺口。需要 `flutter_linux_*.tar.xz`
   解压到可写目录（约 3 GB+）。注意本机 `/` 与 `/root` 只读、`/tmp` 是 3.8 G
   的非持久 tmpfs，因此**建议由宿主机或更高权限侧装到持久路径**并加入 `PATH`，
   而不是在这个会话里解压。
2. **Linux 桌面构建链**：`apt-get install clang cmake ninja-build pkg-config libgtk-3-dev`
   （要写 `/`，本会话的沙箱拒绝）。
3. **无头显示**：`apt-get install xvfb`，然后
   `xvfb-run -a flutter test integration_test -d linux`。
4. （可选）golden 测试需要中文字体 Source Han Sans SC，或从仓库外提供字体文件。

第 1–3 步都需要沙箱之外的权限，因此**这次调查没有产生任何仓库改动**，
也没有执行 `flutter pub get`（`.dart_tool/` 与 `pubspec.lock` 都保持原样）。
