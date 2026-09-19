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
import 'package:kb_admin_desktop/src/widgets/common/dsw_icons.dart';
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
///
/// 它维护一份**可变的**服务端状态（工作区列表 + 每个工作区的会话），这样
/// "增删之后列表确实变了"可以被断言；纯查询类用例拿到的仍是预置的那两条。
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

  /// 服务端状态。
  final List<WorkspaceView> workspaces = <WorkspaceView>[
    _workspace('w-1', '笔记'),
    _workspace('w-2', '资料'),
  ];
  final Map<String, List<SessionView>> sessions = <String, List<SessionView>>{
    'w-1': <SessionView>[_session('s-w-1', 'w-1', '会话@w-1')],
    'w-2': <SessionView>[_session('s-w-2', 'w-2', '会话@w-2')],
  };

  /// 每个会话的正文（`getSession` / `ask` 读写它）。
  final Map<String, SessionDetailReport> details =
      <String, SessionDetailReport>{};

  /// 给新建出来的对象发一个可预期的标识。
  int _sequence = 0;

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
      workspaces: List<WorkspaceView>.of(workspaces),
      error: '',
    );
  }

  @override
  Future<SessionsReport> listSessions(String workspaceId) async {
    calls.add('listSessions:$workspaceId');
    return SessionsReport(
      ok: true,
      sessions: List<SessionView>.of(
        sessions[workspaceId] ?? const <SessionView>[],
      ),
      error: '',
    );
  }

  @override
  Future<WorkspaceReport> addWorkspace({
    required String name,
    required String path,
  }) async {
    calls.add('addWorkspace:$name');
    final WorkspaceView workspace = WorkspaceView(
      id: 'w-new-${_sequence++}',
      name: name,
      path: path,
    );
    workspaces.add(workspace);
    return WorkspaceReport(ok: true, workspace: workspace, error: '');
  }

  @override
  Future<OpReport> removeWorkspace(String workspaceId) async {
    calls.add('removeWorkspace:$workspaceId');
    workspaces.removeWhere((WorkspaceView item) => item.id == workspaceId);
    sessions.remove(workspaceId);
    return const OpReport(ok: true, error: '');
  }

  @override
  Future<WorkspaceReport> renameWorkspace({
    required String workspaceId,
    required String name,
  }) async {
    calls.add('renameWorkspace:$workspaceId');
    final int index = workspaces.indexWhere(
      (WorkspaceView item) => item.id == workspaceId,
    );
    final WorkspaceView renamed = WorkspaceView(
      id: workspaceId,
      name: name.trim(),
      path: index >= 0 ? workspaces[index].path : '',
    );
    if (index >= 0) {
      workspaces[index] = renamed;
    }
    return WorkspaceReport(ok: true, workspace: renamed, error: '');
  }

  @override
  Future<SessionReport> createSession({
    required String workspaceId,
    required String title,
    required String turnId,
    required String question,
  }) async {
    calls.add('createSession:$workspaceId');
    final bool withQuestion = turnId.trim().isNotEmpty;
    // 与 kb_core 同口径：没有显式标题时，从第一个问题推导。
    final String resolvedTitle = title.trim().isNotEmpty
        ? title.trim()
        : (withQuestion ? question : '新会话');
    final SessionView session = SessionView(
      id: 's-new-${_sequence++}',
      workspaceId: workspaceId,
      title: resolvedTitle,
      updatedAtMillis: 1760000000000,
      turnCount: withQuestion ? 1 : 0,
    );
    sessions
        .putIfAbsent(workspaceId, () => <SessionView>[])
        .insert(0, session);
    if (withQuestion) {
      details[session.id] = SessionDetailReport(
        ok: true,
        session: session,
        turns: <TurnView>[_turn(turnId, 'user', question)],
        error: '',
      );
    }
    return SessionReport(ok: true, session: session, error: '');
  }

  @override
  Future<OpReport> removeSession({
    required String workspaceId,
    required String sessionId,
  }) async {
    calls.add('removeSession:$workspaceId:$sessionId');
    sessions[workspaceId]?.removeWhere(
      (SessionView item) => item.id == sessionId,
    );
    return const OpReport(ok: true, error: '');
  }

  @override
  Future<SessionReport> renameSession({
    required String workspaceId,
    required String sessionId,
    required String title,
  }) async {
    calls.add('renameSession:$sessionId');
    final String resolved = title.trim().isNotEmpty
        ? title.trim()
        : _deriveTitle_(sessionId);
    final List<SessionView>? list = sessions[workspaceId];
    final int index =
        list?.indexWhere((SessionView item) => item.id == sessionId) ?? -1;
    final SessionView renamed = list != null && index >= 0
        ? SessionView(
            id: list[index].id,
            workspaceId: list[index].workspaceId,
            title: resolved,
            updatedAtMillis: list[index].updatedAtMillis,
            turnCount: list[index].turnCount,
          )
        : _session(sessionId, workspaceId, resolved);
    if (list != null && index >= 0) {
      list[index] = renamed;
    }
    final SessionDetailReport? detail = details[sessionId];
    if (detail != null) {
      details[sessionId] = SessionDetailReport(
        ok: true,
        session: renamed,
        turns: detail.turns,
        error: '',
      );
    }
    return SessionReport(ok: true, session: renamed, error: '');
  }

  @override
  Future<SessionDetailReport> getSession({
    required String workspaceId,
    required String sessionId,
  }) async {
    calls.add('getSession:$sessionId');
    final SessionDetailReport detail =
        details[sessionId] ?? _detailFor_(workspaceId, sessionId);
    details[sessionId] = detail;
    return detail;
  }

  @override
  Future<SessionDetailReport> ask({
    required String workspaceId,
    required String sessionId,
    required String turnId,
    required String question,
  }) async {
    calls.add('ask:$sessionId');
    // 与 kb_core 里的临时模拟 LLM 同口径：回答是问题的逆序（按字符），
    // 且回答标识由用户回合标识确定性推导。
    final String answer = String.fromCharCodes(
      question.runes.toList().reversed,
    );
    final String answerId = '$turnId-a';
    final SessionDetailReport current =
        details[sessionId] ?? _detailFor_(workspaceId, sessionId);

    // 幂等：已经有这一轮回答就原样返回（草稿流程会先建会话带上问题，再 Ask）。
    if (current.turns.any((TurnView turn) => turn.id == answerId)) {
      return current;
    }
    final bool hasUser = current.turns.any(
      (TurnView turn) => turn.id == turnId,
    );
    final List<TurnView> turns = <TurnView>[
      ...current.turns,
      if (!hasUser) _turn(turnId, 'user', question),
      _turn(answerId, 'assistant', answer),
    ];
    final SessionDetailReport next = SessionDetailReport(
      ok: true,
      session: SessionView(
        id: sessionId,
        workspaceId: workspaceId,
        title: current.session.title,
        updatedAtMillis: 1760000001000,
        turnCount: turns.length,
      ),
      turns: turns,
      error: '',
    );
    details[sessionId] = next;

    // 会话摘要里的 `turn_count` 也跟着变，列表下次读到的应当是新的。
    final List<SessionView>? list = sessions[workspaceId];
    final int index = list?.indexWhere(
          (SessionView item) => item.id == sessionId,
        ) ??
        -1;
    if (list != null && index >= 0) {
      list[index] = next.session;
    }
    return next;
  }

  /// 空白改名时的退路：取首条用户消息（与 `kb_core` 的推导口径一致）。
  String _deriveTitle_(String sessionId) {
    for (final TurnView turn
        in details[sessionId]?.turns ?? const <TurnView>[]) {
      if (turn.role == 'user' && turn.text.trim().isNotEmpty) {
        return turn.text.trim();
      }
    }
    return '新会话';
  }

  /// 没拉过正文时给一个空会话。
  SessionDetailReport _detailFor_(String workspaceId, String sessionId) {
    SessionView? summary;
    for (final SessionView item
        in sessions[workspaceId] ?? const <SessionView>[]) {
      if (item.id == sessionId) {
        summary = item;
        break;
      }
    }
    return SessionDetailReport(
      ok: true,
      session: summary ?? _session(sessionId, workspaceId, '会话'),
      turns: const <TurnView>[],
      error: '',
    );
  }
}

/// 造一条服务端消息。
TurnView _turn(String id, String role, String text) => TurnView(
  id: id,
  role: role,
  text: text,
  reasoning: '',
  state: 'done',
  notice: '',
  noticeIsError: false,
);

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
    // 服务端的工作区。名字在侧边栏与面包屑里各出现一次，所以限定在侧边栏里找。
    final Finder sidebar = find.byType(ServerWorkspaceList);
    expect(
      find.descendant(of: sidebar, matching: find.text('笔记')),
      findsOneWidget,
    );
    expect(
      find.descendant(of: sidebar, matching: find.text('资料')),
      findsOneWidget,
    );
    // 展开着的第一个工作区，会话已经拉下来了。
    expect(
      find.descendant(of: sidebar, matching: find.text('会话@w-1')),
      findsOneWidget,
    );
    expect(sidebar, findsOneWidget);
  });

  /// 测试新建工作区：提交给服务端、刷新列表、并把新工作区选中。
  ///
  /// - 手段：连上假接口后调 `addWorkspace`，名字与目录都带过去。
  /// - 判断：返回空串；假接口收到 `addWorkspace:新库`；列表里出现「新库」且
  ///   路径原样；`selectedWorkspaceId` 指向服务端分配的那个标识——用户接着
  ///   建会话时不会建到别的工作区里。
  test('新建工作区后刷新列表并选中它', () async {
    final _FakeApi api = _FakeApi(profiles: <ConnectionView>[_profile('本机')]);
    final ConnectionController connection = ConnectionController(api);
    await connection.initialize();

    final String error = await connection.addWorkspace(
      name: '新库',
      path: '/srv/new',
    );

    expect(error, isEmpty);
    expect(api.calls, contains('addWorkspace:新库'));
    expect(
      connection.workspaces.map((WorkspaceView item) => item.name),
      contains('新库'),
    );
    final WorkspaceView created = connection.workspaces.firstWhere(
      (WorkspaceView item) => item.name == '新库',
    );
    expect(created.path, '/srv/new');
    expect(connection.selectedWorkspaceId, created.id);
  });

  /// 测试删除工作区：服务端删掉之后，列表刷新并把它去掉。
  ///
  /// - 手段：连上之后删掉预置的 `w-1`。
  /// - 判断：假接口收到 `removeWorkspace:w-1`；列表里不再有「笔记」，
  ///   剩下的「资料」还在——删除是精确的，没有误伤。
  test('删除工作区后列表不再有它', () async {
    final _FakeApi api = _FakeApi(profiles: <ConnectionView>[_profile('本机')]);
    final ConnectionController connection = ConnectionController(api);
    await connection.initialize();

    final String error = await connection.removeWorkspace('w-1');

    expect(error, isEmpty);
    expect(api.calls, contains('removeWorkspace:w-1'));
    expect(
      connection.workspaces.map((WorkspaceView item) => item.name),
      <String>['资料'],
    );
  });

  /// 测试草稿会话：点「新会话」只改客户端状态，首次提问才真的建会话。
  ///
  /// - 手段：连上之后 `startDraftSession('w-1')`，确认这时**没有任何请求**；
  ///   然后 `ask('你好世界')`。
  /// - 判断：草稿期间 `createSession` 没被调用、也没有新会话；提问之后假接口先收到
  ///   `createSession:w-1` 再收到 `ask:`，会话名字由假服务端从问题推导，
  ///   正文两回合（问题 + 逆序回答）——`turn_id` 去重生效，问题只记了一次。
  test('草稿会话首次提问才创建并落盘', () async {
    final _FakeApi api = _FakeApi(profiles: <ConnectionView>[_profile('本机')]);
    final ConnectionController connection = ConnectionController(api);
    await connection.initialize();

    final String draftError = await connection.startDraftSession('w-1');
    expect(draftError, isEmpty);
    expect(connection.draftingSession, isTrue);
    expect(connection.selectedSessionId, isNull);
    expect(
      api.calls.where((String call) => call.startsWith('createSession')),
      isEmpty,
      reason: '草稿阶段不应当请求服务端',
    );

    final String error = await connection.ask('你好世界');

    expect(error, isEmpty);
    expect(connection.draftingSession, isFalse);
    expect(
      api.calls.indexOf('createSession:w-1'),
      lessThan(api.calls.indexWhere((String call) => call.startsWith('ask:'))),
    );

    final String? sessionId = connection.selectedSessionId;
    expect(sessionId, isNotNull);
    expect(connection.selectedServerSession?.title, '你好世界');
    final SessionDetailReport? detail = connection.sessionDetailOf(sessionId!);
    expect(detail, isNotNull);
    expect(detail!.turns.length, 2);
    expect(detail.turns[0].text, '你好世界');
    expect(detail.turns[1].text, '界世好你');
  });

  /// 测试工作区改名：提交给服务端并刷新列表。
  ///
  /// - 手段：连上之后对 `w-1` 调 `renameWorkspace('新名字')`。
  /// - 判断：返回空串；假接口收到 `renameWorkspace:w-1`；列表里那条工作区的名字
  ///   变成新的、标识不变。
  test('工作区改名后列表更新', () async {
    final _FakeApi api = _FakeApi(profiles: <ConnectionView>[_profile('本机')]);
    final ConnectionController connection = ConnectionController(api);
    await connection.initialize();

    final String error = await connection.renameWorkspace('w-1', '新名字');

    expect(error, isEmpty);
    expect(api.calls, contains('renameWorkspace:w-1'));
    final WorkspaceView renamed = connection.workspaces.firstWhere(
      (WorkspaceView item) => item.id == 'w-1',
    );
    expect(renamed.name, '新名字');
  });

  /// 测试会话改名：提交给服务端并刷新该工作区的会话列表。
  ///
  /// - 手段：连上之后对 `s-w-1` 调 `renameSession('我的标题')`。
  /// - 判断：返回空串；假接口收到 `renameSession:s-w-1`；列表里那条会话的标题
  ///   变成新的；空白标题则回落到从首条用户消息推导。
  test('会话改名后列表更新且空白标题回落', () async {
    final _FakeApi api = _FakeApi(profiles: <ConnectionView>[_profile('本机')]);
    final ConnectionController connection = ConnectionController(api);
    await connection.initialize();

    final String error = await connection.renameSession(
      'w-1',
      's-w-1',
      '我的标题',
    );
    expect(error, isEmpty);
    expect(api.calls, contains('renameSession:s-w-1'));
    expect(
      connection
          .sessionsOf('w-1')
          .firstWhere((SessionView item) => item.id == 's-w-1')
          .title,
      '我的标题',
    );

    // 先问一句，再清空标题：服务端会从首条用户消息重新取名。
    await connection.selectSession('w-1', 's-w-1');
    await connection.ask('你好世界');
    await connection.renameSession('w-1', 's-w-1', '   ');
    expect(connection.selectedServerSession?.title, '你好世界');
  });

  /// 测试删除会话：删掉之后刷新该工作区的列表，别的会话不受影响。
  ///
  /// - 手段：连上之后先拉 `w-1`、`w-2` 的会话，再删掉 `w-1` 唯一的那条。
  /// - 判断：假接口收到 `removeSession:w-1:s-w-1`；`w-1` 的列表变空，
  ///   而 `w-2` 仍然有它自己那条。
  test('删除会话后它从列表消失', () async {
    final _FakeApi api = _FakeApi(profiles: <ConnectionView>[_profile('本机')]);
    final ConnectionController connection = ConnectionController(api);
    await connection.initialize();
    await connection.loadSessions('w-1');
    await connection.loadSessions('w-2');

    final String error = await connection.removeSession('w-1', 's-w-1');

    expect(error, isEmpty);
    expect(api.calls, contains('removeSession:w-1:s-w-1'));
    expect(connection.sessionsOf('w-1'), isEmpty);
    expect(connection.sessionsOf('w-2').length, 1);
  });

  /// 测试「新建工作区」对话框：填完名字与目录后提交给服务端，列表出现新工作区。
  ///
  /// - 手段：用假接口连上，渲染 `KbAdminApp`；点工具提示为「新建工作区」的按钮，
  ///   在**对话框内**的两个输入框里填名字与目录，点「创建」。
  /// - 判断：假接口收到 `addWorkspace:服务端新库`；界面上出现这个名字。
  ///   （输入框要限定在对话框里找：对话区的输入框也是一个 `TextField`。）
  testWidgets('新建工作区对话框会提交给服务端', (WidgetTester tester) async {
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

    await tester.tap(find.byTooltip('新建工作区'));
    await tester.pumpAndSettle();

    final Finder fields = find.descendant(
      of: find.byType(Dialog),
      matching: find.byType(TextField),
    );
    expect(fields, findsNWidgets(2));
    await tester.enterText(fields.at(0), '服务端新库');
    await tester.enterText(fields.at(1), '/srv/notes');
    await tester.tap(find.text('创建'));
    await tester.pumpAndSettle();

    expect(api.calls, contains('addWorkspace:服务端新库'));
    expect(find.text('服务端新库'), findsWidgets);
  });

  /// 测试选中会话会拉正文，并且同一个会话不会重复拉。
  ///
  /// - 手段：连上假接口后对 `s-w-1` 连续调两次 `selectSession`。
  /// - 判断：选中态与摘要都正确；假接口里 `getSession:s-w-1` **只被调用一次**
  ///   （第二次命中正文缓存）。
  test('选中会话时按需拉正文且不重复拉', () async {
    final _FakeApi api = _FakeApi(profiles: <ConnectionView>[_profile('本机')]);
    final ConnectionController connection = ConnectionController(api);
    await connection.initialize();

    await connection.selectSession('w-1', 's-w-1');
    await connection.selectSession('w-1', 's-w-1');

    expect(connection.selectedSessionId, 's-w-1');
    expect(connection.selectedServerSession?.title, '会话@w-1');
    expect(connection.sessionDetailOf('s-w-1'), isNotNull);
    expect(
      api.calls.where((String call) => call == 'getSession:s-w-1').length,
      1,
    );
  });

  /// 测试提问：正文里出现问题与逆序回答，会话列表的条数也刷新。
  ///
  /// - 手段：选中 `s-w-1` 后调 `ask('你好世界')`。
  /// - 判断：返回空串；假接口收到 `ask:s-w-1`；正文两回合——用户是原问题、
  ///  助手是它的逆序；列表里那条摘要的 `turnCount` 变成 2。
  test('提问后正文与列表都更新', () async {
    final _FakeApi api = _FakeApi(profiles: <ConnectionView>[_profile('本机')]);
    final ConnectionController connection = ConnectionController(api);
    await connection.initialize();
    await connection.selectSession('w-1', 's-w-1');

    final String error = await connection.ask('你好世界');

    expect(error, isEmpty);
    expect(api.calls, contains('ask:s-w-1'));
    final SessionDetailReport? detail = connection.sessionDetailOf('s-w-1');
    expect(detail, isNotNull);
    expect(detail!.turns.length, 2);
    expect(detail.turns[0].role, 'user');
    expect(detail.turns[0].text, '你好世界');
    expect(detail.turns[1].text, '界世好你');
    expect(connection.selectedServerSession?.turnCount, 2);
  });

  /// 测试左上角的主机按钮：显示当前连接的花名，并能切换到另一条连接。
  ///
  /// - 手段：配好两条连接（本机 / 实验室），自动连上「本机」；点主机按钮打开
  ///   菜单，选「实验室」。
  /// - 判断：初始时花名与服务端版本都在界面上；切换后假接口收到
  ///   `connect:实验室`，控制器的 `profileName` 也变成「实验室」。
  testWidgets('左上角主机按钮显示当前花名并可切换连接', (WidgetTester tester) async {
    tester.view.physicalSize = const Size(1400, 900);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);

    final _FakeApi api = _FakeApi(
      profiles: <ConnectionView>[_profile('本机'), _profile('实验室')],
    );
    final ConnectionController connection = ConnectionController(api);
    await connection.initialize();
    expect(connection.profileName, '本机');

    final AppController app = await _appController();
    await tester.pumpWidget(
      KbAdminApp(controller: app, connection: connection),
    );
    await tester.pumpAndSettle();

    expect(find.text('本机'), findsOneWidget);
    expect(find.textContaining('kb_core 9.9.9'), findsOneWidget);

    await tester.tap(find.text('本机'));
    await tester.pumpAndSettle();
    expect(find.text('实验室'), findsOneWidget);
    await tester.tap(find.text('实验室'));
    await tester.pumpAndSettle();

    expect(api.calls, contains('connect:实验室'));
    expect(connection.profileName, '实验室');
  });

  /// 测试"选中会话 → 提问"这条界面链路：服务端的逆序回答出现在对话区。
  ///
  /// - 手段：连上假接口、渲染应用；点侧边栏里那条会话，在输入框里输入「你好」
  ///   并点发送按钮。
  /// - 判断：假接口收到 `getSession:s-w-1` 与 `ask:s-w-1`；对话区同时出现用户
  ///   气泡「你好」与助手的逆序回答「好你」。
  testWidgets('提问之后对话区显示服务端的逆序回答', (WidgetTester tester) async {
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

    // 选中服务端的会话（第一个工作区默认展开，会话已经拉下来了）。
    await tester.tap(find.text('会话@w-1'));
    await tester.pumpAndSettle();
    expect(api.calls, contains('getSession:s-w-1'));

    // 对话区的输入框是全应用唯一的那个 TextField（连接对话框没打开）。
    await tester.enterText(find.byType(TextField), '你好');
    await tester.pumpAndSettle();
    await tester.tap(find.byType(SendArrowIcon));
    await tester.pumpAndSettle();

    expect(api.calls, contains('ask:s-w-1'));
    expect(find.text('你好'), findsWidgets);
    expect(find.text('好你'), findsOneWidget);
  });

  /// 测试界面上的草稿会话：点「新会话」→ 输入第一个问题 → 会话才被创建。
  ///
  /// - 手段：连上假接口、渲染应用；点侧边栏顶部的「新会话」按钮，直接在输入框里
  ///   输入问题并发送。
  /// - 判断：假接口先收到 `createSession:w-1` 再收到 `ask:`；会话列表里出现一条
  ///   以问题为标题的会话；对话区显示问题与逆序回答。
  testWidgets('点「新会话」后第一个问题才创建会话', (WidgetTester tester) async {
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

    // 展开态与折叠轨道各有一个「新会话」按钮（都在树里，靠不透明度切换），
    // 这里点展开态的那个。
    await tester.tap(find.byTooltip('新会话').first);
    await tester.pumpAndSettle();
    expect(connection.draftingSession, isTrue);

    await tester.enterText(find.byType(TextField), '你好世界');
    await tester.pumpAndSettle();
    await tester.tap(find.byType(SendArrowIcon));
    await tester.pumpAndSettle();

    expect(api.calls, contains('createSession:w-1'));
    expect(connection.draftingSession, isFalse);
    expect(connection.selectedServerSession?.title, '你好世界');
    expect(find.text('界世好你'), findsOneWidget);
  });
}
