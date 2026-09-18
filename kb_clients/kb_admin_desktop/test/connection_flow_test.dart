// 连接 kb_core 这条链路的测试。
//
// 用一份**[假的 KbClientApi]** 驱动，因此完全不需要初始化原生库
// （`RustLib.init()`）——这正是把界面收在 `KbClientApi` 后面的目的。
//
// 覆盖两件事：
//   1. 状态机：首次运行 / 自动连接 / 列工作区 / 按需拉会话 / 失败 / 断开；
//   2. 界面：连上之后侧边栏显示服务端的工作区与会话；首次运行的对话框能保存并连接。

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kb_admin_desktop/src/app.dart';
import 'package:kb_admin_desktop/src/services/kb_client_api.dart';
import 'package:kb_admin_desktop/src/services/local_store.dart';
import 'package:kb_admin_desktop/src/state/app_controller.dart';
import 'package:kb_admin_desktop/src/state/connection_controller.dart';
import 'package:kb_admin_desktop/src/widgets/connection/connection_dialog.dart';
import 'package:kb_admin_desktop/src/widgets/sidebar/server_workspace_list.dart';
import 'package:shared_preferences/shared_preferences.dart';

/// 一条假的工作区。
WorkspaceView _workspace(String id, String name) => WorkspaceView(
  id: id,
  name: name,
  path: '/tmp/$id',
);

/// 一条假的会话。
SessionView _session(String id, String workspaceId, String title) => SessionView(
  id: id,
  workspaceId: workspaceId,
  title: title,
  updatedAtMillis: 1760000000000,
  turnCount: 3,
);

/// 一条假的连接方式。
ConnectionView _profile(String name) => ConnectionView(
  name: name,
  kind: 'local-launch',
  summary: '启动本机 kb-core 并连 /run/user/1000/llm_kb',
  kbCore: '/usr/local/bin/kb-core',
  runtimeDir: '/run/user/1000/llm_kb',
  storageDir: '/run/user/1000/llm_kb/storage',
  address: '',
);

/// 假的原生接口。
class _FakeApi implements KbClientApi {
  _FakeApi({
    this.configExists = true,
    this.profiles = const <ConnectionView>[],
    this.connectOk = true,
    this.connectError = '',
  });

  bool configExists;
  List<ConnectionView> profiles;
  bool connectOk;
  String connectError;

  /// 记录调用，便于断言"界面确实走了这条路"。
  final List<String> calls = <String>[];
  final List<ConnectionView> savedConnections = <ConnectionView>[];

  @override
  Future<String> configFilePath() async => '/tmp/fake/config.toml';

  @override
  Future<ConfigView> loadConfig() async {
    calls.add('loadConfig');
    return ConfigView(
      path: '/tmp/fake/config.toml',
      exists: configExists,
      defaultName: profiles.isEmpty ? '' : profiles.first.name,
      connections: profiles,
      error: '',
    );
  }

  @override
  Future<String> saveConfig({
    required String defaultName,
    required List<ConnectionView> connections,
  }) async {
    calls.add('saveConfig:$defaultName');
    profiles = connections;
    configExists = true;
    savedConnections
      ..clear()
      ..addAll(connections);
    return '';
  }

  @override
  Future<List<String>> connectionKinds() async =>
      <String>['local-launch', 'local-attach', 'tcp'];

  @override
  Future<String> connectionKindDescription(String kind) async =>
      '说明:$kind';

  @override
  Future<ConnectionView> suggestLocalConnection(String name) async {
    calls.add('suggest:$name');
    return _profile(name);
  }

  @override
  Future<ConnectReport> connect(ConnectionView profile) async {
    calls.add('connect:${profile.name}');
    if (!connectOk) {
      return ConnectReport(
        ok: false,
        profileName: profile.name,
        serverVersion: '',
        protocolVersion: 0,
        isLocal: false,
        launchedPid: 0,
        error: connectError,
      );
    }
    return ConnectReport(
      ok: true,
      profileName: profile.name,
      serverVersion: '9.9.9',
      protocolVersion: 1,
      isLocal: profile.kind != 'tcp',
      launchedPid: profile.kind == 'local-launch' ? 4242 : 0,
      error: '',
    );
  }

  @override
  Future<void> disconnect() async => calls.add('disconnect');

  @override
  Future<WorkspacesReport> listWorkspaces() async {
    calls.add('listWorkspaces');
    return WorkspacesReport(
      ok: true,
      workspaces: <WorkspaceView>[
        _workspace('w-1', '笔记'),
        _workspace('w-2', '资料'),
      ],
      error: '',
    );
  }

  @override
  Future<SessionsReport> listSessions(String workspaceId) async {
    calls.add('listSessions:$workspaceId');
    return SessionsReport(
      ok: true,
      sessions: <SessionView>[
        _session('s-$workspaceId', workspaceId, '会话@$workspaceId'),
      ],
      error: '',
    );
  }
}

/// 跑在内存里的应用状态（与 `app_shell_test.dart` 同一套做法）。
Future<AppController> _appController() async {
  SharedPreferences.setMockInitialValues(<String, Object>{});
  final LocalStore store = await LocalStore.open();
  return AppController(store, snapshot: store.load());
}

void main() {
  /// 测试"配置不存在"会走到首次运行，而不是直接报错。
  ///
  /// - 手段：假接口说 `exists == false`，跑一次 `initialize()`。
  /// - 判断：`firstRun` 为真、状态仍是 `idle`、没有错误——
  ///   界面据此弹「连接 kb_core」对话框。
  test('没有配置文件时进入首次运行', () async {
    final _FakeApi api = _FakeApi(configExists: false);
    final ConnectionController connection = ConnectionController(api);

    await connection.initialize();

    expect(connection.initialized, isTrue);
    expect(connection.firstRun, isTrue);
    expect(connection.phase, ConnectionPhase.idle);
    expect(connection.error, isEmpty);
    expect(connection.profiles, isEmpty);
  });

  /// 测试有配置时自动连上缺省那条，并把工作区拉下来。
  ///
  /// - 手段：假接口给两条连接方式（第一条是缺省），连接与列工作区都成功。
  /// - 判断：状态是 `connected`；服务端身份是 `9.9.9`；工作区两条且名字对得上；
  ///   调用记录里能看到 `connect:` 与 `listWorkspaces`（顺序也校验）。
  test('有配置时自动连接并列出工作区', () async {
    final _FakeApi api = _FakeApi(
      profiles: <ConnectionView>[_profile('本机'), _profile('实验室')],
    );
    final ConnectionController connection = ConnectionController(api);

    await connection.initialize();

    expect(connection.connected, isTrue);
    expect(connection.profileName, '本机');
    expect(connection.serverVersion, '9.9.9');
    expect(connection.protocolVersion, 1);
    expect(connection.isLocal, isTrue);
    expect(connection.launchedPid, 4242);
    expect(
      connection.workspaces.map((WorkspaceView w) => w.name),
      <String>['笔记', '资料'],
    );
    expect(connection.selectedWorkspaceId, 'w-1');
    expect(
      api.calls,
      <String>['loadConfig', 'connect:本机', 'listWorkspaces'],
    );
  });

  /// 测试"选中工作区 → 按需拉会话"，且同一个工作区不会重复拉。
  ///
  /// - 手段：连上之后 `selectWorkspace('w-2')` 两次。
  /// - 判断：`sessionsOf` 拿到那条会话；假接口里 `listSessions:w-2` **只被调用一次**
  ///   （第二次命中缓存）。
  test('选中工作区时按需拉会话且不重复拉', () async {
    final _FakeApi api = _FakeApi(profiles: <ConnectionView>[_profile('本机')]);
    final ConnectionController connection = ConnectionController(api);
    await connection.initialize();

    await connection.selectWorkspace('w-2');
    await connection.selectWorkspace('w-2');

    expect(connection.selectedWorkspaceId, 'w-2');
    expect(
      connection.sessionsOf('w-2').map((SessionView s) => s.title),
      <String>['会话@w-2'],
    );
    expect(
      api.calls.where((String call) => call == 'listSessions:w-2').length,
      1,
    );
  });

  /// 测试连接失败会把原因留在状态里，而不是抛出去。
  ///
  /// - 手段：假接口的 `connect` 返回 `ok == false` 且带一句错误。
  /// - 判断：状态是 `failed`、`connected` 为假、`error` 就是那句话，
  ///   工作区列表为空。
  test('连接失败时状态是 failed 且带错误说明', () async {
    final _FakeApi api = _FakeApi(
      profiles: <ConnectionView>[_profile('本机')],
      connectOk: false,
      connectError: '本机 IPC 传输失败: 等待服务端端点超时（5000 ms）',
    );
    final ConnectionController connection = ConnectionController(api);

    await connection.initialize();

    expect(connection.phase, ConnectionPhase.failed);
    expect(connection.connected, isFalse);
    expect(connection.error, contains('等待服务端端点超时'));
    expect(connection.workspaces, isEmpty);
  });

  /// 测试断开之后列表被清空。
  ///
  /// - 手段：连上并列出工作区之后 `disconnect()`。
  /// - 判断：状态回到 `idle`、工作区与已拉到的会话都没了、假接口收到 `disconnect`。
  test('断开后清空工作区与会话', () async {
    final _FakeApi api = _FakeApi(profiles: <ConnectionView>[_profile('本机')]);
    final ConnectionController connection = ConnectionController(api);
    await connection.initialize();
    await connection.selectWorkspace('w-1');

    await connection.disconnect();

    expect(connection.phase, ConnectionPhase.idle);
    expect(connection.workspaces, isEmpty);
    expect(connection.selectedWorkspaceId, isNull);
    expect(api.calls, contains('disconnect'));
  });

  /// 测试首次运行的对话框：预填本机连接，点「保存并连接」会写配置并连上。
  ///
  /// - 手段：把对话框单独放进一个最小 App 里（配置不存在），等预填完成，
  ///   点「保存并连接」。
  /// - 判断：对话框自己关掉；假接口收到 `suggest:本机`、`saveConfig:本机`、
  ///   `connect:本机`；控制器进入 `connected`。
  testWidgets('首次运行对话框保存并连接', (WidgetTester tester) async {
    final _FakeApi api = _FakeApi(configExists: false);
    final ConnectionController connection = ConnectionController(api);

    await tester.pumpWidget(
      MaterialApp(
        home: Builder(
          builder: (BuildContext context) => TextButton(
            onPressed: () => showConnectionDialog(
              context,
              connection,
              firstRun: true,
            ),
            child: const Text('open'),
          ),
        ),
      ),
    );

    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();
    expect(find.text('连接 kb_core'), findsOneWidget);

    await tester.tap(find.text('保存并连接'));
    await tester.pumpAndSettle();

    expect(api.calls, contains('suggest:本机'));
    expect(api.calls, contains('saveConfig:本机'));
    expect(api.calls, contains('connect:本机'));
    expect(connection.connected, isTrue);
    expect(find.text('连接 kb_core'), findsNothing);
  });

  /// 测试连上之后，服务端的工作区与会话出现在侧边栏里。
  ///
  /// - 手段：用假接口连上，把 `KbAdminApp` 渲染出来（宽屏），等会话拉完。
  /// - 判断：状态条显示连接方式与服务端版本；两个工作区名都在；
  ///   第一个工作区展开着，所以它的会话标题也在。
  testWidgets('连上之后侧边栏显示服务端的工作区与会话', (WidgetTester tester) async {
    tester.view.physicalSize = const Size(1400, 900);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);

    final _FakeApi api = _FakeApi(profiles: <ConnectionView>[_profile('本机')]);
    final ConnectionController connection = ConnectionController(api);
    await connection.initialize();

    final AppController app = await _appController();
    await tester.pumpWidget(
      KbAdminApp(controller: app, connection: connection),
    );
    await tester.pumpAndSettle();

    // 状态条：连接方式 + 服务端版本。
    expect(find.textContaining('kb_core 9.9.9'), findsOneWidget);
    // 服务端的工作区。
    expect(find.text('笔记'), findsOneWidget);
    expect(find.text('资料'), findsOneWidget);
    // 展开着的第一个工作区，会话已经拉下来了。
    expect(find.text('会话@w-1'), findsOneWidget);
    expect(find.byType(ServerWorkspaceList), findsOneWidget);
  });
}
