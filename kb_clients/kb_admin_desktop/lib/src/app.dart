// 应用根组件。
//
// 只做三件事：把 [AppController] 挂到组件树上、按主题模式装配 [MaterialApp]、
// 把三栏框架塞进 `Scaffold`。DSH 没有全局顶栏，所以这里也没有 `AppBar`——
// 每一栏自己负责自己的头部。

import 'package:flutter/material.dart';

import 'state/app_controller.dart';
import 'theme/app_theme.dart';
import 'widgets/app_shell.dart';

/// 知识库管理客户端。
class KbAdminApp extends StatelessWidget {
  /// 构造应用。
  const KbAdminApp({super.key, required this.controller});

  /// 应用状态。
  final AppController controller;

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
          home: Scaffold(body: AppShell(controller: controller)),
        );
      },
    );
  }
}
