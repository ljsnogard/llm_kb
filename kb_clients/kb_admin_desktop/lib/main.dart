// kb_admin_desktop 的入口。
//
// 应用结构与 DSH v0.1.5-rc 对齐（左侧工作区列表 + 新会话、中间对话、右侧工作区
// 文件浏览器），是 `kb_core` / `kb_svc_salvo` 的桌面客户端。
//
// 启动顺序：
//
// 1. `LocalStore.open()` 读出上一次的界面偏好、LLM 服务与工作区；
// 2. `AppController` 用这份快照构造；
// 3. `RustLib.init()` 初始化 flutter_rust_bridge（工作区文件浏览等本地能力
//    将来会用到；失败不阻塞界面）。
//
// HTTP / WebSocket 通道（`kb_svc_salvo` 的 `/api/settings*` 与 `/ws/chat`）
// 尚未接入，属于下一阶段。

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:kb_admin_desktop/src/app.dart';
import 'package:kb_admin_desktop/src/rust/frb_generated.dart';
import 'package:kb_admin_desktop/src/services/local_store.dart';
import 'package:kb_admin_desktop/src/state/app_controller.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();

  final LocalStore store = await LocalStore.open();
  final AppController controller = AppController(
    store,
    snapshot: store.load(),
  );

  // flutter_rust_bridge 的原生库初始化。当前界面阶段用不到它，因此失败也
  // 不影响启动；下一阶段做本地文件浏览时会成为必需。
  unawaited(
    RustLib.init().catchError((Object error) {
      debugPrint('RustLib 初始化失败（当前阶段可忽略）: $error');
    }),
  );

  runApp(KbAdminApp(controller: controller));
}
