// 应用根组件。
//
// 只做三件事：把 [AppController] 挂到组件树上、按主题模式装配 [MaterialApp]、
// 把三栏框架塞进 `Scaffold`。DSH 没有全局顶栏，所以这里也没有 `AppBar`——
// 每一栏自己负责自己的头部。

import 'package:flutter/material.dart';

import 'state/app_controller.dart';
import 'state/connection_controller.dart';
import 'theme/app_theme.dart';
import 'widgets/app_shell.dart';
import 'widgets/connection/connection_dialog.dart';

/// 知识库管理客户端。
class KbAdminApp extends StatelessWidget {
  /// 构造应用。
  const KbAdminApp({super.key, required this.controller, this.connection});

  /// 应用状态。
  final AppController controller;

  /// 与 `kb_core` 的连接状态；为 `null` 时不接连接（widget 测试用）。
  final ConnectionController? connection;

  @override
  Widget build(BuildContext context) {
    // 整个应用挂在一个 [AnimatedBuilder] 上：状态里的任何变化（布局、工作区、
    // 消息、主题）都会重建三栏。对当前这个体量足够简单可靠；等接入流式回答后，
    // 应当把高频变化的会话内容收窄到各自的倾听者。
    return AnimatedBuilder(
      animation: controller,
      builder: (BuildContext context, Widget? _) {
        return MaterialApp(
          title: 'llm_kb',
          debugShowCheckedModeBanner: false,
          theme: buildDswTheme(Brightness.light),
          darkTheme: buildDswTheme(Brightness.dark),
          themeMode: controller.themeMode,
          // 没有 AppBar：DSH 也没有全局顶栏，每一栏自己负责自己的头部。
          home: Scaffold(
            body: _FirstRunGate(
              connection: connection,
              child: AppShell(controller: controller, connection: connection),
            ),
          ),
        );
      },
    );
  }
}

/// 「首次运行」闸门：配置不存在时，等第一帧过去之后弹出连接对话框。
///
/// 放在组件树里而不是 `main` 里，是因为它需要一个已经挂好的 [BuildContext]
/// 才能弹对话框；用 `addPostFrameCallback` 保证不在 build 期间改动状态。
class _FirstRunGate extends StatefulWidget {
  const _FirstRunGate({required this.child, this.connection});

  final Widget child;
  final ConnectionController? connection;

  @override
  State<_FirstRunGate> createState() => _FirstRunGateState();
}

class _FirstRunGateState extends State<_FirstRunGate> {
  /// 本次会话是否已经弹过一次，避免用户取消之后又被反复弹出来打扰。
  bool _asked = false;

  @override
  void initState() {
    super.initState();
    widget.connection?.addListener(_maybeAsk);
  }

  @override
  void didUpdateWidget(_FirstRunGate oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.connection != widget.connection) {
      oldWidget.connection?.removeListener(_maybeAsk);
      widget.connection?.addListener(_maybeAsk);
    }
  }

  @override
  void dispose() {
    widget.connection?.removeListener(_maybeAsk);
    super.dispose();
  }

  void _maybeAsk() {
    final ConnectionController? connection = widget.connection;
    if (_asked || connection == null || !connection.firstRun) {
      return;
    }
    _asked = true;
    WidgetsBinding.instance.addPostFrameCallback((_) async {
      if (!mounted) {
        return;
      }
      await showConnectionDialog(context, connection, firstRun: true);
      connection.dismissFirstRun();
    });
  }

  @override
  Widget build(BuildContext context) => widget.child;
}
