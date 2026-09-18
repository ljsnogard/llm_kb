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
    if (changed) {
      notifyListeners();
    }
    await loadSessions(workspaceId);
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

  // ── 内部 ────────────────────────────────────────────────────────────

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
