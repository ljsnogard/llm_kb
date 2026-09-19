// 与 `kb_core` 的连接状态。
//
// 它只管一件事：**现在连着谁、对面有哪些工作区与会话**。界面布局、主题、LLM 服务
// 那些仍然在 [AppController] 里——两者互不依赖，各自 `notifyListeners()`。
//
// ```text
// initialize()
//   ├─ 读配置（kb_client_config）
//   │    ├─ 不存在 → firstRun = true，界面弹「连接 kb_core」对话框
//   │    └─ 存在   → 取 default 那一条，自动连
//   ├─ connect(profile)：系统层 + 应用层握手
//   └─ 连上之后 refreshWorkspaces()
// selectWorkspace(id)：按需拉那个工作区的会话
// ```

import 'package:flutter/foundation.dart';

import '../services/kb_client_api.dart';

/// 连接当前处在哪一步。
enum ConnectionPhase {
  /// 还没开始（或已经断开）。
  idle,

  /// 正在连（握手 / 拉列表）。
  connecting,

  /// 已连上。
  connected,

  /// 连不上 / 配置坏了。
  failed,
}

/// 与 `kb_core` 的连接状态。
class ConnectionController extends ChangeNotifier {
  /// 用一个原生接口实现构造（测试里塞假的）。
  ConnectionController(this._api);

  final KbClientApi _api;

  bool _initialized = false;
  bool _busy = false;
  bool _firstRun = false;

  String _configPath = '';
  bool _configExists = false;
  String _configError = '';
  String _defaultName = '';
  List<ConnectionView> _profiles = const <ConnectionView>[];

  ConnectionPhase _phase = ConnectionPhase.idle;
  String _profileName = '';
  String _serverVersion = '';
  int _protocolVersion = 0;
  bool _isLocal = false;
  int _launchedPid = 0;
  String _error = '';

  List<WorkspaceView> _workspaces = const <WorkspaceView>[];
  final Map<String, List<SessionView>> _sessions = <String, List<SessionView>>{};
  final Set<String> _loadingSessions = <String>{};
  String? _selectedWorkspaceId;

  /// 选中的会话（连同它属于哪个工作区，因为读取正文两个标识都要）。
  String? _selectedSessionId;
  String? _selectedSessionWorkspaceId;

  /// 是否正处在一个**草稿会话**里。
  ///
  /// 草稿只活在客户端：点「新会话」不会马上在 `kb_core` 上建会话（"没有消息、
  /// 又只有默认名字"的会话不允许落盘），而是等用户发出第一条消息时，
  /// 用这条消息作为首条 `Turn` 去 `CreateSession`——名字由 `kb_core` 从问题推导。
  bool _draftSession = false;

  /// 已经拉到的会话正文，按会话标识缓存。
  final Map<String, SessionDetailReport> _details = <String, SessionDetailReport>{};
  final Set<String> _loadingDetails = <String>{};

  Map<String, String> _kindDescriptions = const <String, String>{};

  // ── 只读状态 ────────────────────────────────────────────────────────

  /// `initialize()` 是否已经跑过（界面据此决定要不要显示加载态）。
  bool get initialized => _initialized;

  /// 有没有正在进行的异步操作。
  bool get busy => _busy;

  /// 配置还不存在——界面应当弹「首次运行」的连接对话框。
  bool get firstRun => _firstRun;

  /// 配置文件的路径（界面上显示出来，方便用户手改）。
  String get configPath => _configPath;

  /// 配置文件是否存在。
  bool get configExists => _configExists;

  /// 读配置时的错误（空串表示没问题）。
  String get configError => _configError;

  /// 已配置的连接方式。
  List<ConnectionView> get profiles => List<ConnectionView>.unmodifiable(_profiles);

  /// 缺省连接方式的名字。
  String get defaultName => _defaultName;

  /// 当前处在哪一步。
  ConnectionPhase get phase => _phase;

  /// 是不是连上了。
  bool get connected => _phase == ConnectionPhase.connected;

  /// 用的是哪条连接方式。
  String get profileName => _profileName;

  /// 服务端版本（握手拿到的）。
  String get serverVersion => _serverVersion;

  /// 协议版本（握手拿到的）。
  int get protocolVersion => _protocolVersion;

  /// 是不是本机连接（IPC）而不是远程 TCP。
  bool get isLocal => _isLocal;

  /// 客户端自己起的 `kb_core` 的 pid；附着 / 远程连接是 0。
  int get launchedPid => _launchedPid;

  /// 最近一次失败的说明（空串表示没问题）。
  String get error => _error;

  /// 当前连接下的工作区。
  List<WorkspaceView> get workspaces =>
      List<WorkspaceView>.unmodifiable(_workspaces);

  /// 当前选中的工作区标识。
  String? get selectedWorkspaceId => _selectedWorkspaceId;

  /// 当前选中的工作区（服务端对象）；没选中或已消失时是 `null`。
  WorkspaceView? get selectedServerWorkspace {
    final String? id = _selectedWorkspaceId;
    if (id == null) {
      return null;
    }
    for (final WorkspaceView workspace in _workspaces) {
      if (workspace.id == id) {
        return workspace;
      }
    }
    return null;
  }

  /// 当前选中的会话标识。
  String? get selectedSessionId => _selectedSessionId;

  /// 是不是正处在一个还没落盘的**草稿会话**里（界面上显示为「新会话」）。
  bool get draftingSession => _draftSession;

  /// 当前选中的会话摘要（来自列表缓存）。
  SessionView? get selectedServerSession {
    final String? sessionId = _selectedSessionId;
    final String? workspaceId = _selectedSessionWorkspaceId;
    if (sessionId == null || workspaceId == null) {
      return null;
    }
    for (final SessionView session
        in _sessions[workspaceId] ?? const <SessionView>[]) {
      if (session.id == sessionId) {
        return session;
      }
    }
    return null;
  }

  /// 当前选中的会话正文；还没拉到时是 `null`。
  SessionDetailReport? get selectedSessionDetail {
    final String? sessionId = _selectedSessionId;
    return sessionId == null ? null : _details[sessionId];
  }

  /// 某个会话的正文；还没拉到或失败时是 `null`。
  SessionDetailReport? sessionDetailOf(String sessionId) => _details[sessionId];

  /// 某个会话的正文是不是正在拉。
  bool isLoadingDetail(String sessionId) => _loadingDetails.contains(sessionId);

  /// 某个工作区已经拉到的会话；还没拉过时返回空列表。
  List<SessionView> sessionsOf(String workspaceId) =>
      List<SessionView>.unmodifiable(_sessions[workspaceId] ?? const <SessionView>[]);

  /// 某个工作区的会话是不是正在拉。
  bool isLoadingSessions(String workspaceId) =>
      _loadingSessions.contains(workspaceId);

  /// 连接方式 `kind` → 说明文案。
  Map<String, String> get kindDescriptions =>
      Map<String, String>.unmodifiable(_kindDescriptions);

  /// 说明文案，取不到时给个兜底。
  String describeKind(String kind) => _kindDescriptions[kind] ?? kind;

  // ── 启动 ────────────────────────────────────────────────────────────

  /// 读配置；有配置就自动连上缺省那一条。
  ///
  /// 由 `main` 在 `RustLib.init()` 之后调用一次。任何失败都落在 [error] 上，
  /// 不往外抛——界面要能在"连不上"的状态下继续可用。
  Future<void> initialize() async {
    if (_initialized) {
      return;
    }
    _busy = true;
    notifyListeners();

    try {
      _kindDescriptions = await _loadKindDescriptions();
      _configPath = await _api.configFilePath();
      final ConfigView config = await _api.loadConfig();
      _configExists = config.exists;
      _configError = config.error;
      _profiles = config.connections;
      _defaultName = config.defaultName;

      if (_configError.isNotEmpty) {
        // 配置文件在、但坏了：让用户去修，**不要**当成首次运行覆盖掉。
        _phase = ConnectionPhase.failed;
        _error = _configError;
      } else if (!config.exists || config.connections.isEmpty) {
        _firstRun = true;
      } else {
        await _connectLocked(_defaultProfile());
      }
    } catch (error) {
      _phase = ConnectionPhase.failed;
      _error = '读取连接配置失败: $error';
    } finally {
      _initialized = true;
      _busy = false;
      notifyListeners();
    }
  }

  // ── 连接 / 断开 ─────────────────────────────────────────────────────

  /// 连上一条连接方式。返回是否成功。
  Future<bool> connect(ConnectionView profile) async {
    _busy = true;
    notifyListeners();
    try {
      final bool ok = await _connectLocked(profile);
      return ok;
    } finally {
      _busy = false;
      notifyListeners();
    }
  }

  /// 写配置（首次生成或改完保存）。返回空串表示成功。
  Future<String> saveConfig({
    required String defaultName,
    required List<ConnectionView> connections,
  }) async {
    try {
      final String error = await _api.saveConfig(
        defaultName: defaultName,
        connections: connections,
      );
      if (error.isEmpty) {
        _profiles = connections;
        _defaultName = defaultName;
        _configExists = true;
        _configError = '';
      }
      return error;
    } catch (error) {
      return '保存连接配置失败: $error';
    }
  }

  /// 断开当前连接。
  Future<void> disconnect() async {
    _busy = true;
    notifyListeners();
    try {
      await _api.disconnect();
    } catch (_) {
      // 断开失败无所谓：本地状态照样清掉。
    } finally {
      _clearConnection();
      _busy = false;
      notifyListeners();
    }
  }

  /// 首次运行时向原生侧要一条**预填**的本机连接方式。
  ///
  /// `kb-core` 的路径是"猜"出来的（与本可执行文件同目录），打包出来的 App 里通常
  /// 猜不到——那时它的 `kbCore` 是空串，界面要让用户自己填。
  Future<ConnectionView> suggestedLocalConnection(String name) =>
      _api.suggestLocalConnection(name);

  /// 关掉"首次运行"标记（用户取消对话框时调用）。
  void dismissFirstRun() {
    if (!_firstRun) {
      return;
    }
    _firstRun = false;
    notifyListeners();
  }

  /// 清掉最近一次的错误提示。
  void clearError() {
    if (_error.isEmpty) {
      return;
    }
    _error = '';
    notifyListeners();
  }

  // ── 列表 ────────────────────────────────────────────────────────────

  /// 重新拉一遍工作区（会清掉已缓存的会话列表）。
  Future<void> refreshWorkspaces() async {
    if (!connected) {
      return;
    }
    _busy = true;
    notifyListeners();
    try {
      final WorkspacesReport report = await _api.listWorkspaces();
      if (report.ok) {
        _workspaces = report.workspaces;
        _sessions.clear();
        _resetSessionSelection_();
        _error = '';
        if (_selectedWorkspaceId == null ||
            !_workspaces.any((WorkspaceView item) => item.id == _selectedWorkspaceId)) {
          _selectedWorkspaceId =
              _workspaces.isEmpty ? null : _workspaces.first.id;
        }
      } else {
        _error = report.error;
      }
    } catch (error) {
      _error = '列工作区失败: $error';
    } finally {
      _busy = false;
      notifyListeners();
    }
  }

  /// 选中一个工作区，并在需要时拉它的会话。
  Future<void> selectWorkspace(String workspaceId) async {
    final bool changed = _selectedWorkspaceId != workspaceId;
    _selectedWorkspaceId = workspaceId;
    // 切到别的工作区时，原来选中的会话（以及草稿）不再属于当前上下文。
    _draftSession = false;
    if (_selectedSessionWorkspaceId != workspaceId) {
      _selectedSessionId = null;
      _selectedSessionWorkspaceId = null;
    }
    if (changed) {
      notifyListeners();
    }
    await loadSessions(workspaceId);
  }

  /// 选中一个会话：切工作区、切会话，并按需把正文拉下来。
  Future<void> selectSession(String workspaceId, String sessionId) async {
    _selectedWorkspaceId = workspaceId;
    final bool changed = _selectedSessionId != sessionId;
    _selectedSessionId = sessionId;
    _selectedSessionWorkspaceId = workspaceId;
    _draftSession = false;
    if (changed) {
      notifyListeners();
    }
    // 会话摘要来自列表缓存；调用方可能还没拉过（例如直接按标识选中）。
    await loadSessions(workspaceId);
    await loadSessionDetail(workspaceId, sessionId);
  }

  /// 拉某个会话的正文（已经拉过就不重复拉）。
  Future<void> loadSessionDetail(
    String workspaceId,
    String sessionId, {
    bool force = false,
  }) async {
    if (!connected || _loadingDetails.contains(sessionId)) {
      return;
    }
    if (!force && _details.containsKey(sessionId)) {
      return;
    }

    _loadingDetails.add(sessionId);
    notifyListeners();
    try {
      final SessionDetailReport report = await _api.getSession(
        workspaceId: workspaceId,
        sessionId: sessionId,
      );
      if (report.ok) {
        _details[sessionId] = report;
      } else {
        _error = report.error;
      }
    } catch (error) {
      _error = '读会话失败: $error';
    } finally {
      _loadingDetails.remove(sessionId);
      notifyListeners();
    }
  }

  /// 拉某个工作区的会话（已经拉过就不重复拉）。
  Future<void> loadSessions(String workspaceId, {bool force = false}) async {
    if (!connected || _loadingSessions.contains(workspaceId)) {
      return;
    }
    if (!force && _sessions.containsKey(workspaceId)) {
      return;
    }

    _loadingSessions.add(workspaceId);
    notifyListeners();
    try {
      final SessionsReport report = await _api.listSessions(workspaceId);
      if (report.ok) {
        _sessions[workspaceId] = report.sessions;
        debugPrint('[kb] 工作区 $workspaceId 有 ${report.sessions.length} 个会话');
      } else {
        _error = report.error;
      }
    } catch (error) {
      _error = '列会话失败: $error';
    } finally {
      _loadingSessions.remove(workspaceId);
      notifyListeners();
    }
  }

  // ── 增删（工作区 / 会话） ───────────────────────────────────────────
  //
  // 这些操作都发生在 **`kb_core` 进程所在的主机**上：界面只提交名字与路径，
  // 不碰客户端自己这边的文件系统（见 `ConnectionController` 的类注释）。
  // 每个方法都返回空串表示成功，否则是给用户看的失败说明。

  /// 在服务端新建一个工作区；`path` 是服务端主机上的目录。
  Future<String> addWorkspace({required String name, required String path}) async {
    if (!connected) {
      return _notConnected_;
    }
    return _mutate_(() async {
      final WorkspaceReport report = await _api.addWorkspace(
        name: name,
        path: path,
      );
      if (!report.ok) {
        return report.error;
      }
      await _refreshLocked();
      // 新建出来的工作区直接选中：用户多半接着要往里加会话。
      _selectedWorkspaceId = report.workspace.id;
      return '';
    });
  }

  /// 删除一个工作区；服务端会级联删掉它名下的会话。
  Future<String> removeWorkspace(String workspaceId) async {
    if (!connected) {
      return _notConnected_;
    }
    return _mutate_(() async {
      final OpReport report = await _api.removeWorkspace(workspaceId);
      if (!report.ok) {
        return report.error;
      }
      _sessions.remove(workspaceId);
      if (_selectedWorkspaceId == workspaceId) {
        _selectedWorkspaceId = null;
      }
      if (_selectedSessionWorkspaceId == workspaceId) {
        _resetSessionSelection_();
      }
      await _refreshLocked();
      return '';
    });
  }

  /// 在某个工作区里开一个**草稿会话**（客户端本地状态，不落盘）。
  ///
  /// 界面上显示为「新会话」；用户发出第一条消息时才会 `CreateSession`（带着那条
  /// 消息，好让 `kb_core` 据此起名），所以"空的「新会话」"永远不会出现在磁盘上。
  /// 返回空串表示成功。
  Future<String> startDraftSession(String workspaceId) async {
    if (!connected) {
      return _notConnected_;
    }
    if (!_workspaces.any((WorkspaceView item) => item.id == workspaceId)) {
      return '工作区不存在，先刷新一下';
    }
    _selectedWorkspaceId = workspaceId;
    _selectedSessionId = null;
    _selectedSessionWorkspaceId = null;
    _draftSession = true;
    notifyListeners();
    return '';
  }

  /// 重命名一个工作区；返回空串表示成功。
  Future<String> renameWorkspace(String workspaceId, String name) async {
    if (!connected) {
      return _notConnected_;
    }
    final String trimmed = name.trim();
    if (trimmed.isEmpty) {
      return '名字不能为空';
    }
    return _mutate_(() async {
      final WorkspaceReport report = await _api.renameWorkspace(
        workspaceId: workspaceId,
        name: trimmed,
      );
      if (!report.ok) {
        return report.error;
      }
      await _refreshLocked();
      _selectedWorkspaceId = report.workspace.id;
      return '';
    });
  }

  /// 删除一个会话。
  Future<String> removeSession(String workspaceId, String sessionId) async {
    if (!connected) {
      return _notConnected_;
    }
    return _mutate_(() async {
      final OpReport report = await _api.removeSession(
        workspaceId: workspaceId,
        sessionId: sessionId,
      );
      if (!report.ok) {
        return report.error;
      }
      if (_selectedSessionId == sessionId) {
        _resetSessionSelection_();
      } else {
        _details.remove(sessionId);
      }
      await _reloadSessions_(workspaceId);
      return '';
    });
  }

  /// 在当前选中的工作区里开一个草稿会话。
  ///
  /// 侧边栏顶部的「新会话」按钮在已连接时走这里；返回空串表示成功。
  Future<String> newSessionInSelectedWorkspace() async {
    if (_workspaces.isEmpty) {
      return '还没有工作区，先在列表右上角新建一个';
    }
    final String? workspaceId = _selectedWorkspaceId;
    if (workspaceId == null) {
      return '还没有选中工作区';
    }
    return startDraftSession(workspaceId);
  }

  /// 重命名一个会话（改标题）；返回空串表示成功。
  ///
  /// `title` 只有空白时由服务端重新推导（取首条用户消息），所以它也用来
  /// "清掉手工起的名字"。
  Future<String> renameSession(
    String workspaceId,
    String sessionId,
    String title,
  ) async {
    if (!connected) {
      return _notConnected_;
    }
    return _mutate_(() async {
      final SessionReport report = await _api.renameSession(
        workspaceId: workspaceId,
        sessionId: sessionId,
        title: title,
      );
      if (!report.ok) {
        return report.error;
      }
      await _reloadSessions_(workspaceId);
      return '';
    });
  }

  // ── 提问（生成） ────────────────────────────────────────────────────

  /// 向当前选中的会话提问（草稿会话则先建会话）；返回空串表示成功。
  ///
  /// `kb_core` 现在跑的是临时模拟的 LLM（把问题逆序输出），一回就带着两条新
  /// 消息回来，因此这里直接更新正文缓存，并顺手刷新会话列表（`turn_count` 变了）。
  /// 换成真 LLM 的流式生成后，这里会改成"发出去 + 订阅事件"。
  Future<String> ask(String question) async {
    if (!connected) {
      return _notConnected_;
    }
    if (_busy) {
      return '上一条提问还在处理中';
    }
    final WorkspaceView? workspace = selectedServerWorkspace;
    if (workspace == null) {
      return '先在左侧选一个工作区';
    }
    final String text = question.trim();
    if (text.isEmpty) {
      return '';
    }

    final bool drafting = _draftSession;
    final String? sessionId = _selectedSessionId;
    if (!drafting && sessionId == null) {
      return '先在左侧选一个会话，或者点「新会话」';
    }

    return _mutate_(() async {
      final String turnId = _newTurnId_();
      final String targetSessionId;

      if (drafting) {
        // 草稿首次提问：先把这条消息作为会话的第一条 `Turn` 建出去（名字由
        // `kb_core` 从问题推导），再让 `Ask` 补上回答。两步用同一个 `turn_id`，
        // 服务端会去重，所以问题不会被记两遍。
        final SessionReport created = await _api.createSession(
          workspaceId: workspace.id,
          title: '',
          turnId: turnId,
          question: text,
        );
        if (!created.ok) {
          return created.error;
        }
        targetSessionId = created.session.id;
      } else {
        targetSessionId = sessionId!;
      }

      final SessionDetailReport report = await _api.ask(
        workspaceId: workspace.id,
        sessionId: targetSessionId,
        turnId: turnId,
        question: text,
      );
      if (!report.ok) {
        return report.error;
      }

      _draftSession = false;
      _selectedWorkspaceId = workspace.id;
      _selectedSessionId = targetSessionId;
      _selectedSessionWorkspaceId = workspace.id;
      _details[targetSessionId] = report;
      await _reloadSessions_(workspace.id);
      return '';
    });
  }

  // ── 内部 ────────────────────────────────────────────────────────────

  /// 还没连上 `kb_core` 时统一的说明。
  static const String _notConnected_ = '还没有连接 kb_core';

  /// 清掉"选中的会话"、草稿状态与正文缓存。
  void _resetSessionSelection_() {
    _selectedSessionId = null;
    _selectedSessionWorkspaceId = null;
    _draftSession = false;
    _details.clear();
    _loadingDetails.clear();
  }

  /// 生成一个回合标识（协议要求它由客户端生成）。
  String _newTurnId_() => 't-${DateTime.now().microsecondsSinceEpoch}';

  /// 一次会改动服务端状态的操作：统一处理忙标记、错误与通知。
  Future<String> _mutate_(Future<String> Function() work) async {
    _busy = true;
    notifyListeners();
    try {
      final String error = await work();
      _error = error;
      return error;
    } catch (error) {
      final String message = '操作失败: $error';
      _error = message;
      return message;
    } finally {
      _busy = false;
      notifyListeners();
    }
  }

  /// 重新拉某个工作区的会话（不做"已缓存就跳过"的判断）。
  Future<void> _reloadSessions_(String workspaceId) async {
    try {
      final SessionsReport report = await _api.listSessions(workspaceId);
      if (report.ok) {
        _sessions[workspaceId] = report.sessions;
      } else {
        _error = report.error;
      }
    } catch (error) {
      _error = '列会话失败: $error';
    }
  }

  /// 真正做连接（调用方负责 `_busy` 与 `notifyListeners`）。
  Future<bool> _connectLocked(ConnectionView profile) async {
    _phase = ConnectionPhase.connecting;
    _error = '';
    notifyListeners();

    ConnectReport report;
    try {
      report = await _api.connect(profile);
    } catch (error) {
      _phase = ConnectionPhase.failed;
      _error = '连接 kb_core 失败: $error';
      return false;
    }

    if (!report.ok) {
      _phase = ConnectionPhase.failed;
      _error = report.error;
      debugPrint('[kb] 连接「${profile.name}」失败: $_error');
      return false;
    }

    _profileName = report.profileName;
    _serverVersion = report.serverVersion;
    _protocolVersion = report.protocolVersion;
    _isLocal = report.isLocal;
    _launchedPid = report.launchedPid;
    _phase = ConnectionPhase.connected;
    _firstRun = false;
    _selectedWorkspaceId = null;
    _workspaces = const <WorkspaceView>[];
    _sessions.clear();
    _resetSessionSelection_();

    await _refreshLocked();
    debugPrint(
      '[kb] 已连接「$_profileName」：kb_core $_serverVersion（协议 v$_protocolVersion，'
      '${_isLocal ? '本机' : '远程'}${_launchedPid > 0 ? '，自起 pid $_launchedPid' : ''}）'
      '，工作区 ${_workspaces.length} 个',
    );
    return true;
  }

  /// [refreshWorkspaces] 的"无状态"版本，给连接流程内部复用。
  Future<void> _refreshLocked() async {
    try {
      final WorkspacesReport report = await _api.listWorkspaces();
      if (report.ok) {
        _workspaces = report.workspaces;
        _selectedWorkspaceId = _workspaces.isEmpty ? null : _workspaces.first.id;
      } else {
        _error = report.error;
      }
    } catch (error) {
      _error = '列工作区失败: $error';
    }
  }

  void _clearConnection() {
    _phase = ConnectionPhase.idle;
    _profileName = '';
    _serverVersion = '';
    _protocolVersion = 0;
    _isLocal = false;
    _launchedPid = 0;
    _workspaces = const <WorkspaceView>[];
    _sessions.clear();
    _selectedWorkspaceId = null;
    _resetSessionSelection_();
  }

  /// 取缺省那一条：`defaultName` 指定的；没指定就用第一条。
  ConnectionView _defaultProfile() {
    for (final ConnectionView profile in _profiles) {
      if (profile.name == _defaultName) {
        return profile;
      }
    }
    return _profiles.first;
  }

  Future<Map<String, String>> _loadKindDescriptions() async {
    final Map<String, String> descriptions = <String, String>{};
    try {
      for (final String kind in await _api.connectionKinds()) {
        descriptions[kind] = await _api.connectionKindDescription(kind);
      }
    } catch (_) {
      // 取不到说明也不该挡住连接；界面会退化成显示 kind 本身。
    }
    return descriptions;
  }
}
