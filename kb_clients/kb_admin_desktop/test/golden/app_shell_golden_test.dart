// 视觉基线（golden）测试。
//
// 这些基线依赖本机安装的字体，因此在 `dart_test.yaml` 里打了 `golden` 标签并默认
// 跳过。需要更新或查看时显式运行：
//
// ```bash
// flutter test --run-skipped --tags golden --update-goldens test/golden
// ```
//
// 生成的 PNG 在 `test/golden/goldens/` 下，可以直接当作「三栏布局长什么样」的
// 参考图看，也可以用来做视觉回归。
//
// 注意 `flutter_test` 自带的默认字体把所有字形都画成实心方块：中文会走
// `fontFamilyFallback` 用上系统字体，但 ASCII 仍是方块。为了让基线可读，
// 这里把主题的正文族显式指向加载进来的那款中文字体。

@Tags(<String>['golden'])
library;

import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kb_admin_desktop/src/models/chat_turn.dart';
import 'package:kb_admin_desktop/src/models/llm_service.dart';
import 'package:kb_admin_desktop/src/services/local_store.dart';
import 'package:kb_admin_desktop/src/state/app_controller.dart';
import 'package:kb_admin_desktop/src/theme/app_theme.dart';
import 'package:kb_admin_desktop/src/widgets/app_shell.dart';
import 'package:kb_admin_desktop/src/widgets/settings/settings_dialog.dart';
import 'package:shared_preferences/shared_preferences.dart';

/// 基线渲染使用的中文字体族名。
const String _fontFamily = 'Source Han Sans SC';

/// 系统中文字体路径；不存在时基线里的汉字会是方块。
const String _fontPath =
    '/usr/share/fonts/opentype/source-han-cjk/SourceHanSansSC-Regular.otf';

/// 把系统中文字体注册成 [_fontFamily]。
Future<void> _loadFonts() async {
  final File file = File(_fontPath);
  if (!file.existsSync()) {
    return;
  }
  final Uint8List bytes = await file.readAsBytes();
  final FontLoader loader = FontLoader(_fontFamily)
    ..addFont(Future<ByteData>.value(ByteData.view(bytes.buffer)));
  await loader.load();
}

/// 在 DSH 主题基础上把正文字体指向已加载的中文字体。
ThemeData _theme(Brightness brightness) {
  final ThemeData base = buildDswTheme(brightness);
  return base.copyWith(
    textTheme: base.textTheme.apply(
      fontFamily: _fontFamily,
      fontFamilyFallback: DswTypography.sansFallback,
    ),
  );
}

/// 把三栏框架挂到一个与正式应用同构的最小外壳里。
Widget _appHost(AppController controller) {
  return AnimatedBuilder(
    animation: controller,
    builder: (BuildContext context, Widget? _) {
      return MaterialApp(
        debugShowCheckedModeBanner: false,
        theme: _theme(Brightness.light),
        darkTheme: _theme(Brightness.dark),
        themeMode: controller.themeMode,
        home: Scaffold(body: AppShell(controller: controller)),
      );
    },
  );
}

/// 造一个已经装了内容的应用状态。
Future<AppController> _seededController() async {
  SharedPreferences.setMockInitialValues(<String, Object>{});
  final LocalStore store = await LocalStore.open();
  final AppController controller = AppController(store, snapshot: store.load());

  controller.upsertService(
    const LlmService(
      id: 'deepseek',
      provider: 'deepseek',
      model: 'deepseek-chat',
      baseUrl: 'https://api.deepseek.com',
      apiKey: 'sk-demo',
    ),
  );
  controller.addWorkspace(name: 'llm_kb 笔记', path: '/home/me/notes/llm_kb');
  controller.addWorkspace(name: '会议记录', path: '/home/me/notes/meetings');
  controller.startSession();
  controller.appendTurn(ChatTurn.user('帮我总结一下知识库检索的两种方式。'));
  controller.appendTurn(
    ChatTurn(
      id: 'demo-1',
      role: ChatRole.assistant,
      reasoning: '先区分模糊搜索与向量搜索的适用场景，再给出选择建议。',
      text:
          '知识库目前规划了两种检索方式：\n\n'
          '1. **模糊搜索**：基于 Turso 的全文检索，适合关键词明确的问题；\n'
          '2. **向量搜索**：把查询与切片分别向量化后比较距离，适合语义相近但\n'
          '   用词不同的情况。\n\n'
          '```rust\n'
          'let hits = kb.search(&query, SearchMode::Hybrid).await?;\n'
          '```\n\n'
          '实际使用中通常先向量召回、再用关键词重排。',
      usage: const TokenUsage(
        inputTokens: 128,
        outputTokens: 256,
        totalTokens: 384,
      ),
    ),
  );
  return controller;
}

/// 把测试窗口固定到 1440×900。
void _useWideWindow(WidgetTester tester) {
  tester.view.physicalSize = const Size(1440, 900);
  tester.view.devicePixelRatio = 1.0;
  addTearDown(tester.view.reset);
}

/// 等所有动画稳定，并放掉工作区落盘的 500ms 防抖定时器。
Future<void> _settle(WidgetTester tester) async {
  await tester.pumpAndSettle();
  await tester.pump(const Duration(milliseconds: 600));
}

void main() {
  setUpAll(_loadFonts);

  /// 生成「默认状态」的视觉基线：左侧栏展开、右侧文件面板隐藏。
  /// - 手段：渲染整个三栏框架并等动画稳定。
  /// - 判断：与 `goldens/app_shell_default.png` 一致。
  testWidgets('默认三栏状态', (WidgetTester tester) async {
    _useWideWindow(tester);
    final AppController controller = await _seededController();
    addTearDown(controller.dispose);

    await tester.pumpWidget(_appHost(controller));
    await _settle(tester);

    await expectLater(
      find.byType(AppShell),
      matchesGoldenFile('goldens/app_shell_default.png'),
    );
  });

  /// 生成「侧边栏折叠 + 文件面板展开」的视觉基线。
  /// - 手段：折叠侧边栏并调出文件面板，等动画走完。
  /// - 判断：与 `goldens/app_shell_panels.png` 一致。
  testWidgets('折叠侧边栏并打开文件面板', (WidgetTester tester) async {
    _useWideWindow(tester);
    final AppController controller = await _seededController();
    addTearDown(controller.dispose);

    await tester.pumpWidget(_appHost(controller));
    await tester.pumpAndSettle();

    controller.toggleSidebar(1440);
    controller.toggleFilePanel(1440);
    await _settle(tester);

    await expectLater(
      find.byType(AppShell),
      matchesGoldenFile('goldens/app_shell_panels.png'),
    );
  });

  /// 生成浅色主题下的视觉基线。
  /// - 手段：把主题切到浅色后渲染。
  /// - 判断：与 `goldens/app_shell_light.png` 一致。
  testWidgets('浅色主题', (WidgetTester tester) async {
    _useWideWindow(tester);
    final AppController controller = await _seededController();
    addTearDown(controller.dispose);
    controller.setThemeMode(ThemeMode.light);

    await tester.pumpWidget(_appHost(controller));
    await _settle(tester);

    await expectLater(
      find.byType(AppShell),
      matchesGoldenFile('goldens/app_shell_light.png'),
    );
  });

  /// 生成设置面板的视觉基线。
  /// - 手段：直接渲染 [SettingsDialog]，状态里已有一条服务。
  /// - 判断：与 `goldens/settings_dialog.png` 一致。
  testWidgets('设置面板', (WidgetTester tester) async {
    tester.view.physicalSize = const Size(1000, 900);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);

    final AppController controller = await _seededController();
    addTearDown(controller.dispose);

    await tester.pumpWidget(
      MaterialApp(
        theme: _theme(Brightness.dark),
        home: Scaffold(body: SettingsDialog(controller: controller)),
      ),
    );
    await _settle(tester);

    await expectLater(
      find.byType(SettingsDialog),
      matchesGoldenFile('goldens/settings_dialog.png'),
    );
  });
}
