// 端到端冒烟测试：在真实设备上把整个应用拉起来，确认三栏框架能正常渲染，
// 并且「调出右侧文件面板」这个动态效果确实生效。
//
// 与 `test/` 下的 widget 测试不同，这里会真正初始化 flutter_rust_bridge
// 的原生库，因此能同时验证原生依赖是否可用。

import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:kb_admin_desktop/src/app.dart';
import 'package:kb_admin_desktop/src/rust/frb_generated.dart';
import 'package:kb_admin_desktop/src/services/local_store.dart';
import 'package:kb_admin_desktop/src/state/app_controller.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  setUpAll(() async => RustLib.init());

  testWidgets('三栏框架可以启动并切换文件面板', (WidgetTester tester) async {
    final LocalStore store = await LocalStore.open();
    final AppController controller = AppController(
      store,
      snapshot: store.load(),
    );

    await tester.pumpWidget(KbAdminApp(controller: controller));
    await tester.pump();

    // 侧边栏底部的设置按钮始终存在。
    expect(find.text('设置'), findsWidgets);

    // 默认不显示文件面板；点开列头右上角的按钮后应当出现。
    expect(find.textContaining('目录为空'), findsNothing);
    await tester.tap(find.byTooltip('显示工作区文件'));
    await tester.pump(const Duration(milliseconds: 600));
    expect(find.textContaining('目录为空'), findsOneWidget);
  });
}
