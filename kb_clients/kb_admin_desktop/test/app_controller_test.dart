// 应用状态（[AppController]）的行为测试。
//
// 这里只覆盖「有明确约定、容易被改坏」的几条：窄视口自动折叠、面板打开时让位、
// 会话标题推导、主题切换、以及服务的新增 / 删除 / 生效切换。

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kb_admin_desktop/src/models/chat_turn.dart';
import 'package:kb_admin_desktop/src/models/llm_service.dart';
import 'package:kb_admin_desktop/src/services/local_store.dart';
import 'package:kb_admin_desktop/src/state/app_controller.dart';
import 'package:kb_admin_desktop/src/theme/dsw_tokens.dart';
import 'package:shared_preferences/shared_preferences.dart';

/// 造一个跑在内存里的应用状态。
Future<AppController> _controller() async {
  SharedPreferences.setMockInitialValues(<String, Object>{});
  final LocalStore store = await LocalStore.open();
  return AppController(store, snapshot: store.load());
}

void main() {
  /// 测试窄视口下侧边栏会自动折叠成轨道。
  /// - 手段：分别用 1400（宽于阈值 1024）与 800（窄于阈值）询问折叠状态。
  /// - 判断：宽视口不折叠；窄视口折叠，且此时宽度偏好为 0。
  test('窄于 1024 时侧边栏自动折叠', () async {
    final AppController controller = await _controller();
    addTearDown(controller.dispose);

    expect(controller.sidebarCollapsedFor(1400), isFalse);
    expect(controller.sidebarPreferenceFor(1400), DswLayout.sidebarDefault);

    expect(controller.sidebarCollapsedFor(800), isTrue);
    expect(controller.sidebarPreferenceFor(800), 0);
  });

  /// 测试窄视口下用户仍能手动展开侧边栏。
  /// - 手段：在 800 宽度下调用 toggleSidebar（此时是折叠态）。
  /// - 判断：折叠状态翻转为 false，宽度偏好回到用户偏好值。
  test('窄视口下可以手动把侧边栏展开', () async {
    final AppController controller = await _controller();
    addTearDown(controller.dispose);

    controller.toggleSidebar(800);

    expect(controller.sidebarCollapsedFor(800), isFalse);
    expect(controller.sidebarPreferenceFor(800), DswLayout.sidebarDefault);
    // 宽视口不受这次手动展开影响，仍按自己的偏好走。
    expect(controller.sidebarCollapsedFor(1400), isFalse);
  });

  /// 测试窄视口下打开文件面板会让侧边栏让位。
  /// - 手段：先在 800 宽度下展开侧边栏，再打开文件面板。
  /// - 判断：文件面板为展开态，同时侧边栏重新折叠成轨道。
  test('打开文件面板时窄视口的侧边栏让位', () async {
    final AppController controller = await _controller();
    addTearDown(controller.dispose);

    controller.toggleSidebar(800);
    expect(controller.sidebarCollapsedFor(800), isFalse);

    controller.toggleFilePanel(800);

    expect(controller.filePanelShown, isTrue);
    expect(controller.sidebarCollapsedFor(800), isTrue);
  });

  /// 测试侧边栏宽度拖拽会被夹到合法区间。
  /// - 手段：分别拖到 100 与 9999。
  /// - 判断：分别被夹到 264 与 420。
  test('侧边栏宽度拖拽被夹到 [264, 420]', () async {
    final AppController controller = await _controller();
    addTearDown(controller.dispose);

    controller.setSidebarWidth(100);
    expect(controller.sidebarWidth, DswLayout.sidebarMin);

    controller.setSidebarWidth(9999);
    expect(controller.sidebarWidth, DswLayout.sidebarMax);
  });

  /// 测试会话标题取自首条用户消息。
  /// - 手段：新建会话后追加一条较长的用户消息。
  /// - 判断：标题被截断成 18 个字符加省略号。
  test('会话标题由首条用户消息推导', () async {
    final AppController controller = await _controller();
    addTearDown(controller.dispose);

    controller.addWorkspace(name: '笔记', path: '/tmp/notes');
    controller.startSession();
    expect(controller.activeSession?.title, '新会话');

    controller.appendTurn(ChatTurn.user('这是一条特别长的用户提问用来验证标题截断'));

    final String title = controller.activeSession?.title ?? '';
    expect(title.endsWith('…'), isTrue);
    expect(title.length, 19);
  });

  /// 测试服务的新增、生效切换与删除。
  /// - 手段：依次加入两个带 key 的服务，切换生效服务，再删除正在生效的那个。
  /// - 判断：首个服务自动成为生效服务；切换后生效标识更新；删除后回落到剩下的
  ///   那个可用服务；再删光后生效标识变成 null。
  test('服务增删与生效服务回落', () async {
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
    expect(controller.activeServiceId, 'a');

    controller.upsertService(
      const LlmService(
        id: 'b',
        provider: 'openai',
        model: 'gpt-4o',
        apiKey: 'sk-b',
      ),
    );
    controller.setActiveService('b');
    expect(controller.activeServiceId, 'b');

    controller.removeService('b');
    expect(controller.activeServiceId, 'a');

    controller.removeService('a');
    expect(controller.activeServiceId, isNull);
    expect(controller.hasUsableService, isFalse);
  });

  /// 测试没有 key 的服务不会自动成为生效服务。
  /// - 手段：只加入一个 apiKey 为空的服务。
  /// - 判断：生效标识仍为 null，且 hasUsableService 为 false。
  test('缺少 API key 的服务不会自动生效', () async {
    final AppController controller = await _controller();
    addTearDown(controller.dispose);

    controller.upsertService(
      const LlmService(id: 'a', provider: 'deepseek', model: 'deepseek-chat'),
    );

    expect(controller.activeServiceId, isNull);
    expect(controller.hasUsableService, isFalse);
  });

  /// 测试主题在深色与浅色之间切换。
  /// - 手段：默认（深色）下以 Brightness.dark 调用 toggleTheme。
  /// - 判断：主题模式变成浅色；再以 Brightness.light 调用一次又变回深色。
  test('主题在深色与浅色之间切换', () async {
    final AppController controller = await _controller();
    addTearDown(controller.dispose);

    expect(controller.themeMode, ThemeMode.dark);

    controller.toggleTheme(Brightness.dark);
    expect(controller.themeMode, ThemeMode.light);

    controller.toggleTheme(Brightness.light);
    expect(controller.themeMode, ThemeMode.dark);
  });

  /// 测试删除工作区后选中项会回到剩下的第一个。
  /// - 手段：建两个工作区并选中第二个，然后删掉它。
  /// - 判断：当前工作区变成剩下的那一个。
  test('删除工作区后选中项回落', () async {
    final AppController controller = await _controller();
    addTearDown(controller.dispose);

    controller.addWorkspace(name: '甲', path: '/tmp/a');
    final String first = controller.activeWorkspaceId!;
    controller.addWorkspace(name: '乙', path: '/tmp/b');
    final String second = controller.activeWorkspaceId!;

    expect(second, isNot(first));

    controller.removeWorkspace(second);
    expect(controller.activeWorkspaceId, first);
  });
}
