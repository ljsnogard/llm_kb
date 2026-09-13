// 设置面板的行为测试。
//
// 重点验证一条从服务端沿用过来的契约：编辑一个已有服务时，API key 输入框会
// 回填遮蔽值 `••••••••`；此时保存**不应该**把遮蔽串写进去，而应当保留原来的 key
// （对应 `kb_svc_salvo/src/web.rs` L208-218 的处理）。

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kb_admin_desktop/src/models/llm_service.dart';
import 'package:kb_admin_desktop/src/services/local_store.dart';
import 'package:kb_admin_desktop/src/state/app_controller.dart';
import 'package:kb_admin_desktop/src/theme/app_theme.dart';
import 'package:kb_admin_desktop/src/widgets/settings/settings_dialog.dart';
import 'package:shared_preferences/shared_preferences.dart';

/// 以一个可控的状态渲染设置面板。
///
/// 窗口开得足够高，让整张表单（包括「保存」按钮）落在可视区内——默认的
/// 800×600 会让按钮滚出屏幕，`tap()` 就打不中。
Future<void> _pumpDialog(WidgetTester tester, AppController controller) async {
  tester.view.physicalSize = const Size(1400, 1200);
  tester.view.devicePixelRatio = 1.0;
  addTearDown(tester.view.reset);

  await tester.pumpWidget(
    MaterialApp(
      theme: buildDswTheme(Brightness.dark),
      home: Scaffold(
        body: SettingsDialog(controller: controller),
      ),
    ),
  );
  await tester.pump();
}

/// 造一个跑在内存里的应用状态。
Future<AppController> _controller() async {
  SharedPreferences.setMockInitialValues(<String, Object>{});
  final LocalStore store = await LocalStore.open();
  return AppController(store, snapshot: store.load());
}

/// 对话框内部的输入框（顺序：标识 / provider / 模型 / Base URL / API key）。
Finder _fields() => find.descendant(
  of: find.byType(SettingsDialog),
  matching: find.byType(TextField),
);

void main() {
  /// 测试保存一个全新的服务。
  /// - 手段：填满五个字段后点「保存」。
  /// - 判断：控制器里出现这条服务，key 原样保存，并自动成为生效服务。
  testWidgets('保存新服务并自动生效', (WidgetTester tester) async {
    final AppController controller = await _controller();
    addTearDown(controller.dispose);
    await _pumpDialog(tester, controller);

    await tester.enterText(_fields().at(0), 'deepseek');
    await tester.enterText(_fields().at(1), 'deepseek');
    await tester.enterText(_fields().at(2), 'deepseek-chat');
    await tester.enterText(_fields().at(3), 'https://api.deepseek.com');
    await tester.enterText(_fields().at(4), 'sk-real');
    await tester.tap(find.text('保存'));
    await tester.pump();

    expect(controller.services.single.apiKey, 'sk-real');
    expect(controller.activeServiceId, 'deepseek');
    expect(find.text('deepseek · deepseek-chat · https://api.deepseek.com'), findsOneWidget);
    expect(find.text('当前'), findsOneWidget);
  });

  /// 测试「缺少 API key」标记。
  /// - 手段：只填标识与 provider 后保存。
  /// - 判断：条目上出现「缺少 API key」标签，且没有生效服务。
  testWidgets('未填 key 的服务带缺少标记且不生效', (WidgetTester tester) async {
    final AppController controller = await _controller();
    addTearDown(controller.dispose);
    await _pumpDialog(tester, controller);

    await tester.enterText(_fields().at(0), 'deepseek');
    await tester.enterText(_fields().at(1), 'deepseek');
    await tester.enterText(_fields().at(2), 'deepseek-chat');
    await tester.tap(find.text('保存'));
    await tester.pump();

    expect(find.text('缺少 API key'), findsOneWidget);
    expect(controller.activeServiceId, isNull);
  });

  /// 测试必填校验。
  /// - 手段：不填服务标识直接保存。
  /// - 判断：状态行提示「服务标识不能为空」，且没有写入任何服务。
  testWidgets('缺少服务标识时拒绝保存', (WidgetTester tester) async {
    final AppController controller = await _controller();
    addTearDown(controller.dispose);
    await _pumpDialog(tester, controller);

    await tester.enterText(_fields().at(1), 'deepseek');
    await tester.tap(find.text('保存'));
    await tester.pump();

    expect(find.text('服务标识不能为空'), findsOneWidget);
    expect(controller.services, isEmpty);
  });

  /// 测试编辑服务时遮蔽值不会覆盖真实 key。
  /// - 手段：先保存一个带 key 的服务；点「编辑」把表单回填进遮蔽值；只改模型名
  ///   再保存。
  /// - 判断：key 仍然是原来那一个，模型名已更新。
  testWidgets('编辑服务时保留原有 API key', (WidgetTester tester) async {
    final AppController controller = await _controller();
    addTearDown(controller.dispose);

    controller.upsertService(
      const LlmService(
        id: 'deepseek',
        provider: 'deepseek',
        model: 'deepseek-chat',
        apiKey: 'sk-original',
      ),
    );

    await _pumpDialog(tester, controller);
    await tester.tap(find.text('编辑'));
    await tester.pump();

    // 回填的是遮蔽串，不是真实 key。
    expect(
      tester.widget<TextField>(_fields().at(4)).controller?.text,
      kMaskedApiKey,
    );

    await tester.enterText(_fields().at(2), 'deepseek-reasoner');
    await tester.tap(find.text('保存'));
    await tester.pump();

    expect(controller.services.single.model, 'deepseek-reasoner');
    expect(controller.services.single.apiKey, 'sk-original');
  });

  /// 测试「使用」按钮切换生效服务。
  /// - 手段：先放入两个带 key 的服务（第一个自动生效），再点第二个的「使用」。
  /// - 判断：生效标识变成第二个服务。
  testWidgets('使用按钮切换生效服务', (WidgetTester tester) async {
    final AppController controller = await _controller();
    addTearDown(controller.dispose);

    controller.upsertService(
      const LlmService(
        id: 'a',
        provider: 'deepseek',
        model: 'deepseek-chat',
        apiKey: 'sk-a',
      ),
    );
    controller.upsertService(
      const LlmService(id: 'b', provider: 'openai', model: 'gpt-4o', apiKey: 'sk-b'),
    );

    await _pumpDialog(tester, controller);
    expect(controller.activeServiceId, 'a');

    // 两个服务各有一个「使用」，第二个是 `b` 的。
    await tester.tap(find.text('使用').at(1));
    await tester.pump();

    expect(controller.activeServiceId, 'b');
  });

  /// 测试删除服务。
  /// - 手段：放入两个服务后点其中一个的「删除」。
  /// - 判断：控制器里只剩一个。
  testWidgets('删除按钮移除服务', (WidgetTester tester) async {
    final AppController controller = await _controller();
    addTearDown(controller.dispose);

    controller.upsertService(
      const LlmService(
        id: 'a',
        provider: 'deepseek',
        model: 'deepseek-chat',
        apiKey: 'sk-a',
      ),
    );
    controller.upsertService(
      const LlmService(id: 'b', provider: 'openai', model: 'gpt-4o', apiKey: 'sk-b'),
    );

    await _pumpDialog(tester, controller);
    await tester.tap(find.text('删除').first);
    await tester.pump();

    expect(controller.services.single.id, 'b');
  });
}
