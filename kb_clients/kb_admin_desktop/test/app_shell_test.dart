// 三栏框架的 widget 测试。
//
// 覆盖本次要交付的两条动态效果：
//   1. 工作区列表（左侧栏）可折叠成 56px 图标轨道，再展开回来；
//   2. 工作区文件浏览器（右侧栏）默认隐藏，由对话区列头的按钮调出与收起。
//
// 侧边栏的展开内容在折叠后**仍然挂在树上**（只是被淡出并忽略指针），这样
// 滑动过程中它不会重排——所以这里用「不透明度 + 轨道宽度」判断形态，
// 而不是用 `find.text` 的存在性。

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kb_admin_desktop/src/app.dart';
import 'package:kb_admin_desktop/src/services/local_store.dart';
import 'package:kb_admin_desktop/src/state/app_controller.dart';
import 'package:kb_admin_desktop/src/theme/dsw_tokens.dart';
import 'package:kb_admin_desktop/src/widgets/app_shell.dart';
import 'package:kb_admin_desktop/src/widgets/files/workspace_file_panel.dart';
import 'package:kb_admin_desktop/src/widgets/settings/settings_dialog.dart';
import 'package:kb_admin_desktop/src/widgets/sidebar/sidebar_panel.dart';
import 'package:shared_preferences/shared_preferences.dart';

/// 造一个跑在内存里的应用状态。
Future<AppController> _controller() async {
  SharedPreferences.setMockInitialValues(<String, Object>{});
  final LocalStore store = await LocalStore.open();
  return AppController(store, snapshot: store.load());
}

/// 把测试窗口固定到宽屏尺寸（宽于侧边栏自动折叠阈值 1024）。
void _useWideWindow(WidgetTester tester, {double width = 1400}) {
  tester.view.physicalSize = Size(width, 900);
  tester.view.devicePixelRatio = 1.0;
  addTearDown(tester.view.reset);
}

/// 等一次 300ms 的列宽动画走完。
Future<void> _settleColumns(WidgetTester tester) async {
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 400));
}

/// 读取某个 [Opacity] 的当前值。
double _opacityOf(WidgetTester tester, Key key) =>
    tester.widget<Opacity>(find.byKey(key)).opacity;

void main() {
  /// 测试三栏框架的初始形态。
  /// - 手段：在 1400×900 窗口下渲染应用，等待一帧。
  /// - 判断：左侧栏轨道宽 280、展开内容完全可见、轨道内容完全透明；
  ///   右侧文件面板的正文不可见。
  testWidgets('初始只渲染左侧栏与中间对话区', (WidgetTester tester) async {
    _useWideWindow(tester);
    await tester.pumpWidget(KbAdminApp(controller: await _controller()));
    await tester.pump();

    expect(tester.getSize(find.byKey(AppShell.sidebarTrackKey)).width, 280);
    expect(_opacityOf(tester, SidebarPanel.wideKey), 1.0);
    expect(_opacityOf(tester, SidebarPanel.railKey), 0.0);

    expect(find.text('新会话'), findsOneWidget);
    expect(find.text('设置'), findsOneWidget);
    expect(find.textContaining('目录为空'), findsNothing);
    expect(tester.getSize(find.byKey(AppShell.fileTrackKey)).width, 0);
  });

  /// 测试工作区列表可以折叠成图标轨道并展开回来。
  /// - 手段：点击列头上的「折叠侧边栏」，等动画走完后读取轨道宽度与两层内容
  ///   的不透明度；再点击轨道里的「展开侧边栏」重复一次。
  /// - 判断：折叠后宽度为 [DswLayout.sidebarCollapsed]（56）、展开内容透明、
  ///   轨道内容不透明；展开后回到 280 且不透明度对调。
  testWidgets('工作区列表可折叠成轨道并展开', (WidgetTester tester) async {
    _useWideWindow(tester);
    await tester.pumpWidget(KbAdminApp(controller: await _controller()));
    await tester.pump();

    await tester.tap(find.byTooltip('折叠侧边栏'));
    await _settleColumns(tester);

    expect(
      tester.getSize(find.byKey(AppShell.sidebarTrackKey)).width,
      DswLayout.sidebarCollapsed,
    );
    expect(_opacityOf(tester, SidebarPanel.wideKey), 0.0);
    expect(_opacityOf(tester, SidebarPanel.railKey), 1.0);

    await tester.tap(find.byTooltip('展开侧边栏'));
    await _settleColumns(tester);

    expect(tester.getSize(find.byKey(AppShell.sidebarTrackKey)).width, 280);
    expect(_opacityOf(tester, SidebarPanel.wideKey), 1.0);
    expect(_opacityOf(tester, SidebarPanel.railKey), 0.0);
  });

  /// 测试文件浏览器默认隐藏、可由按钮调出、再收起。
  /// - 手段：先建一个工作区（让面板有内容可显示），再依次点击列头右上角的
  ///   开关按钮，每次等动画走完，读取轨道宽度与面板左边缘的位置。
  /// - 判断：初始轨道宽 0 且面板整体被平移到视口右边缘之外；调出后轨道宽等于
  ///   45% 视口、面板左边缘正好落在对话区的右边缘上；再点一次回到初始状态。
  ///
  /// 面板在收起时**仍然挂在树上**（这是刻意设计：滑动过程中内容不重排），
  /// 因此这里用几何位置而不是 `find.text` 判断显隐。
  testWidgets('文件浏览器默认隐藏并可按钮调出', (WidgetTester tester) async {
    _useWideWindow(tester);
    final AppController controller = await _controller();
    addTearDown(controller.dispose);

    await tester.pumpWidget(KbAdminApp(controller: controller));
    await tester.pump();

    controller.addWorkspace(name: '测试工作区', path: '/home/me/notes');
    await tester.pump();

    const double viewport = 1400;
    const double panelWidth = viewport * DswLayout.rightbarDefaultRatio;

    expect(find.byTooltip('显示工作区文件'), findsOneWidget);
    expect(tester.getSize(find.byKey(AppShell.fileTrackKey)).width, 0);
    expect(
      tester.getTopLeft(find.byType(WorkspaceFilePanel)).dx,
      viewport,
      reason: '收起时面板应当整体位于视口右边缘之外',
    );

    await tester.tap(find.byTooltip('显示工作区文件'));
    await _settleColumns(tester);

    expect(find.textContaining('目录为空'), findsOneWidget);
    expect(find.text('notes'), findsOneWidget);
    expect(find.byTooltip('隐藏工作区文件'), findsOneWidget);
    expect(
      tester.getSize(find.byKey(AppShell.fileTrackKey)).width,
      panelWidth,
    );
    expect(
      tester.getTopLeft(find.byType(WorkspaceFilePanel)).dx,
      viewport - panelWidth,
      reason: '展开后面板左边缘应当与对话区右边缘重合',
    );

    await tester.tap(find.byTooltip('隐藏工作区文件'));
    await _settleColumns(tester);

    expect(find.byTooltip('显示工作区文件'), findsOneWidget);
    expect(tester.getSize(find.byKey(AppShell.fileTrackKey)).width, 0);
    expect(tester.getTopLeft(find.byType(WorkspaceFilePanel)).dx, viewport);
  });

  /// 测试发送一条消息会写入会话并渲染出用户气泡。
  /// - 手段：先建一个工作区与会话，在输入框里输入文字后按 Enter。
  /// - 判断：会话里多出「用户消息 + 说明性助手消息」两条，界面上出现用户气泡
  ///   与「对话通道尚未接入」的提示。
  testWidgets('Enter 发送消息并渲染用户气泡', (WidgetTester tester) async {
    _useWideWindow(tester);
    final AppController controller = await _controller();
    addTearDown(controller.dispose);

    await tester.pumpWidget(KbAdminApp(controller: controller));
    await tester.pump();

    controller.addWorkspace(name: '测试工作区', path: '/tmp/notes');
    controller.startSession();
    await tester.pump();

    await tester.tap(find.byType(TextField));
    await tester.pump();
    await tester.enterText(find.byType(TextField), '你好');
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await _settleColumns(tester);

    expect(controller.activeSession?.turns.length, 2);
    expect(controller.activeSession?.turns.first.text, '你好');
    // 「你好」会同时出现在侧边栏会话标题与面包屑里，所以这里只要求至少一处。
    expect(find.text('你好'), findsWidgets);
    expect(find.textContaining('对话通道尚未接入'), findsOneWidget);

    // 工作区落盘有 500ms 防抖，必须让它跑完，否则测试结束时会报「仍有挂起的 Timer」。
    await tester.pump(const Duration(milliseconds: 600));
  });

  /// 测试「设置」按钮打开的面板可以配置 LLM 服务与 API key。
  /// - 手段：点击左下角「设置」，在对话框表单里填写服务标识 / provider /
  ///   模型 / API key，然后点「保存」。
  /// - 判断：保存后服务条目出现，带上「当前」标记与 provider · model 摘要，
  ///   且控制器里真的存下了这条服务与它的 key。
  testWidgets('设置面板可保存 LLM 服务与 API key', (WidgetTester tester) async {
    _useWideWindow(tester);
    final AppController controller = await _controller();
    addTearDown(controller.dispose);

    await tester.pumpWidget(KbAdminApp(controller: controller));
    await tester.pump();

    await tester.tap(find.text('设置'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 200));

    expect(find.text('新增服务'), findsOneWidget);

    // 只取对话框内部的输入框——后面的对话区里也有一个 TextField。
    final Finder fields = find.descendant(
      of: find.byType(SettingsDialog),
      matching: find.byType(TextField),
    );
    expect(fields, findsNWidgets(5));

    await tester.enterText(fields.at(0), 'deepseek');
    await tester.enterText(fields.at(1), 'deepseek');
    await tester.enterText(fields.at(2), 'deepseek-chat');
    await tester.enterText(fields.at(4), 'sk-test-key');
    await tester.pump();

    await tester.tap(find.text('保存'));
    await tester.pump();

    expect(controller.services.length, 1);
    expect(controller.services.first.apiKey, 'sk-test-key');
    expect(controller.activeServiceId, 'deepseek');
    expect(find.text('当前'), findsOneWidget);
    expect(find.text('deepseek · deepseek-chat'), findsOneWidget);
  });
}
