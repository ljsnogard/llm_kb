// 应用状态。
//
// 分三块：
//
// | 分组 | 内容 | 对应 DSH 的位置 |
// | :--- | :--- | :--- |
// | 布局 | 侧边栏宽度 / 折叠、文件面板显隐 / 宽度、视口宽度 | `ui-layout/src/client/stores.ts` |
// | 数据 | 工作区 → 会话 → 消息 | `ui-workspace` |
// | 配置 | LLM 服务与 API key、主题 | `ui-settings-general` |
//
// 视口宽度**不放在这里**：它由 [AppShell] 的 `LayoutBuilder` 每帧给出，
// 作为参数传给布局计算，避免在 build 期间触发 `notifyListeners()`。

import 'dart:async';

import 'package:flutter/material.dart';

import '../models/chat_session.dart';
import '../models/chat_turn.dart';
import '../models/llm_service.dart';
import '../models/workspace.dart';
import '../services/local_store.dart';
import '../theme/dsw_tokens.dart';

/// 全局应用状态。
class AppController extends ChangeNotifier {
  /// 用一份已经读出的本地快照构造状态。
  AppController(this._store, {required LocalSnapshot snapshot})
    : _themeMode = snapshot.themeMode,
      _sidebarWidth = snapshot.sidebarWidth,
      _sidebarCollapsed = snapshot.sidebarCollapsed,
      _filePanelShown = snapshot.filePanelShown,
      _filePanelWidth = snapshot.filePanelWidth,
      _services = List<LlmService>.of(snapshot.services),
      _activeServiceId = snapshot.activeServiceId,
      _workspaces = List<Workspace>.of(snapshot.workspaces),
      _activeWorkspaceId = snapshot.activeWorkspaceId {
    _normalizeSelection();
  }

  final LocalStore _store;

  // ── 布局 ────────────────────────────────────────────────────────────

  ThemeMode _themeMode;
  double _sidebarWidth;
  bool _sidebarCollapsed;
  bool _narrowExpanded = false;
  bool _filePanelShown;
  double? _filePanelWidth;
  bool _dragging = false;

  // ── 数据 ────────────────────────────────────────────────────────────

  List<LlmService> _services;
  String? _activeServiceId;
  List<Workspace> _workspaces;
  String? _activeWorkspaceId;

  Timer? _workspaceSaveTimer;

  // ── 主题 ────────────────────────────────────────────────────────────

  /// 当前主题模式。
  ThemeMode get themeMode => _themeMode;

  /// 设置主题模式。
  void setThemeMode(ThemeMode mode) {
    if (_themeMode == mode) {
      return;
    }
    _themeMode = mode;
    notifyListeners();
    _persistLayout();
  }

  /// 在深色与浅色之间切换。
  ///
  /// `ThemeMode.system` 下先切到与当前解析结果相反的一侧，符合用户按下
  /// 「切换主题」时的直觉。
  void toggleTheme(Brightness resolved) {
    setThemeMode(resolved == Brightness.dark ? ThemeMode.light : ThemeMode.dark);
  }

  // ── 布局：侧边栏 ────────────────────────────────────────────────────

  /// 用户偏好的侧边栏宽度；0 表示还没拖过，取 [DswLayout.sidebarDefault]。
  double get sidebarWidth =>
      _sidebarWidth == 0 ? DswLayout.sidebarDefault : _sidebarWidth;

  /// 用户是否显式折叠了侧边栏。
  bool get sidebarCollapsed => _sidebarCollapsed;

  /// 是否正在拖拽分栏（拖拽期间关掉过渡动画）。
  bool get dragging => _dragging;

  /// 在给定视口宽度下，侧边栏是否应当折叠。
  ///
  /// 对应 DSH `AppFrame.tsx` L160-161：窄于 [DswLayout.sidebarAutoCollapse]
  /// 时自动折叠成图标轨道，除非用户又手动展开了。
  bool sidebarCollapsedFor(double viewport) =>
      viewport < DswLayout.sidebarAutoCollapse ? !_narrowExpanded : _sidebarCollapsed;

  /// 侧边栏的宽度偏好（0 表示折叠），直接喂给 `ColumnGeometry.solve`。
  double sidebarPreferenceFor(double viewport) =>
      sidebarCollapsedFor(viewport) ? 0 : sidebarWidth;

  /// 折叠 / 展开侧边栏。
  void toggleSidebar(double viewport) {
    if (sidebarCollapsedFor(viewport)) {
      if (viewport < DswLayout.sidebarAutoCollapse) {
        _narrowExpanded = true;
      } else {
        _sidebarCollapsed = false;
      }
    } else {
      _narrowExpanded = false;
      _sidebarCollapsed = true;
    }
    notifyListeners();
    _persistLayout();
  }

  /// 拖拽侧边栏分栏。
  void setSidebarWidth(double width) {
    final double next = DswLayout.clampSidebar(width);
    if (next == _sidebarWidth) {
      return;
    }
    _sidebarWidth = next;
    _sidebarCollapsed = false;
    _narrowExpanded = true;
    notifyListeners();
  }

  // ── 布局：右侧文件面板 ─────────────────────────────────────────────

  /// 右侧文件面板是否展开。
  bool get filePanelShown => _filePanelShown;

  /// 文件面板的宽度偏好；`null` 表示按视口比例取默认值。
  double? get filePanelWidth => _filePanelWidth;

  /// 在给定视口宽度下，文件面板请求的宽度。
  double filePanelPreferenceFor(double viewport) =>
      _filePanelWidth ?? viewport * DswLayout.rightbarDefaultRatio;

  /// 展开或收起文件面板。
  void toggleFilePanel(double viewport) {
    _filePanelShown = !_filePanelShown;
    // 窄视口下打开面板时，先把侧边栏收回轨道，给对话区腾出空间
    // （对应 DSH 的 `narrowExpanded` 复位行为）。
    if (_filePanelShown && viewport < DswLayout.sidebarAutoCollapse) {
      _narrowExpanded = false;
    }
    notifyListeners();
    _persistLayout();
  }

  /// 收起文件面板。
  void hideFilePanel() {
    if (!_filePanelShown) {
      return;
    }
    _filePanelShown = false;
    notifyListeners();
    _persistLayout();
  }

  /// 拖拽文件面板分栏。
  void setFilePanelWidth(double width, double viewport) {
    final double next = DswLayout.clampRightbar(width, viewport);
    if (next == _filePanelWidth) {
      return;
    }
    _filePanelWidth = next;
    notifyListeners();
  }

  /// 标记一次拖拽的开始 / 结束。
  void setDragging(bool value) {
    if (_dragging == value) {
      return;
    }
    _dragging = value;
    notifyListeners();
    if (!value) {
      _persistLayout();
    }
  }

  // ── 工作区 ──────────────────────────────────────────────────────────

  /// 所有工作区。
  List<Workspace> get workspaces => List<Workspace>.unmodifiable(_workspaces);

  /// 当前选中的工作区标识。
  String? get activeWorkspaceId => _activeWorkspaceId;

  /// 当前选中的工作区。
  Workspace? get activeWorkspace => _workspaceById(_activeWorkspaceId);

  /// 当前选中的会话。
  ChatSession? get activeSession => activeWorkspace?.activeSession;

  Workspace? _workspaceById(String? id) {
    if (id == null) {
      return null;
    }
    for (final Workspace workspace in _workspaces) {
      if (workspace.id == id) {
        return workspace;
      }
    }
    return null;
  }

  /// 选中一个工作区。
  void selectWorkspace(String id) {
    if (_activeWorkspaceId == id) {
      return;
    }
    _activeWorkspaceId = id;
    notifyListeners();
    _scheduleWorkspaceSave();
  }

  /// 新增一个工作区并选中它。
  ///
  /// 返回新建的工作区，便于调用方立刻为它建一个会话。
  Workspace addWorkspace({required String name, required String path}) {
    final Workspace workspace = Workspace.create(name: name, path: path);
    _workspaces = <Workspace>[..._workspaces, workspace];
    _activeWorkspaceId = workspace.id;
    notifyListeners();
    _scheduleWorkspaceSave();
    return workspace;
  }

  /// 删除一个工作区。
  void removeWorkspace(String id) {
    _workspaces = _workspaces
        .where((Workspace workspace) => workspace.id != id)
        .toList(growable: false);
    _normalizeSelection();
    notifyListeners();
    _scheduleWorkspaceSave();
  }

  // ── 会话 ────────────────────────────────────────────────────────────

  /// 在当前工作区里新建一个会话并选中。
  ///
  /// 没有工作区时先不建：调用方应当先让用户创建工作区。
  void startSession() {
    final Workspace? workspace = activeWorkspace;
    if (workspace == null) {
      return;
    }
    final ChatSession session = ChatSession.create();
    _replaceWorkspace(
      workspace.copyWith(
        sessions: <ChatSession>[session, ...workspace.sessions],
        lastSessionId: session.id,
      ),
    );
    notifyListeners();
    _scheduleWorkspaceSave();
  }

  /// 选中某个会话。
  void selectSession(String sessionId) {
    final Workspace? workspace = activeWorkspace;
    if (workspace == null) {
      return;
    }
    _replaceWorkspace(workspace.copyWith(lastSessionId: sessionId));
    notifyListeners();
    _scheduleWorkspaceSave();
  }

  /// 删除一个会话。
  void removeSession(String sessionId) {
    final Workspace? workspace = activeWorkspace;
    if (workspace == null) {
      return;
    }
    final List<ChatSession> remaining = workspace.sessions
        .where((ChatSession session) => session.id != sessionId)
        .toList(growable: false);
    _replaceWorkspace(
      workspace.copyWith(
        sessions: remaining,
        clearLastSession: workspace.lastSessionId == sessionId,
      ),
    );
    notifyListeners();
    _scheduleWorkspaceSave();
  }

  // ── 消息 ────────────────────────────────────────────────────────────

  /// 往当前会话追加一条消息。
  void appendTurn(ChatTurn turn) {
    final Workspace? workspace = activeWorkspace;
    final ChatSession? session = workspace?.activeSession;
    if (workspace == null || session == null) {
      return;
    }
    _updateSession(workspace, session, (ChatSession current) {
      final List<ChatTurn> turns = <ChatTurn>[...current.turns, turn];
      return current.copyWith(
        turns: turns,
        title: _deriveTitle(current.title, turns),
        updatedAt: DateTime.now(),
      );
    });
  }

  /// 用 [turnId] 定位并替换一条消息。
  void replaceTurn(ChatTurn turn) {
    final Workspace? workspace = activeWorkspace;
    final ChatSession? session = workspace?.activeSession;
    if (workspace == null || session == null) {
      return;
    }
    _updateSession(workspace, session, (ChatSession current) {
      final bool found = current.turns.any(
        (ChatTurn item) => item.id == turn.id,
      );
      if (!found) {
        return current;
      }
      return current.copyWith(
        turns: current.turns
            .map((ChatTurn item) => item.id == turn.id ? turn : item)
            .toList(growable: false),
        updatedAt: DateTime.now(),
      );
    });
  }

  /// 把会话标题从默认值改成首条用户消息的摘要。
  String _deriveTitle(String current, List<ChatTurn> turns) {
    if (current != '新会话') {
      return current;
    }
    for (final ChatTurn turn in turns) {
      if (turn.role == ChatRole.user && turn.text.trim().isNotEmpty) {
        final String text = turn.text.trim().replaceAll('\n', ' ');
        return text.length <= 18 ? text : '${text.substring(0, 18)}…';
      }
    }
    return current;
  }

  void _updateSession(
    Workspace workspace,
    ChatSession session,
    ChatSession Function(ChatSession current) update,
  ) {
    final ChatSession next = update(session);
    if (identical(next, session)) {
      return;
    }
    _replaceWorkspace(
      workspace.copyWith(
        sessions: workspace.sessions
            .map((ChatSession item) => item.id == session.id ? next : item)
            .toList(growable: false),
      ),
    );
    notifyListeners();
    _scheduleWorkspaceSave();
  }

  void _replaceWorkspace(Workspace workspace) {
    _workspaces = _workspaces
        .map((Workspace item) => item.id == workspace.id ? workspace : item)
        .toList(growable: false);
  }

  // ── LLM 服务 ────────────────────────────────────────────────────────

  /// 已配置的服务。
  List<LlmService> get services => List<LlmService>.unmodifiable(_services);

  /// 当前生效的服务标识。
  String? get activeServiceId => _activeServiceId;

  /// 当前生效的服务。
  LlmService? get activeService {
    for (final LlmService service in _services) {
      if (service.id == _activeServiceId) {
        return service;
      }
    }
    return null;
  }

  /// 是否已经配好一个可用的服务（有 key）。
  bool get hasUsableService =>
      _services.any((LlmService service) => service.hasApiKey);

  /// 新增或覆盖一个服务。
  void upsertService(LlmService service) {
    final int index = _services.indexWhere(
      (LlmService item) => item.id == service.id,
    );
    if (index >= 0) {
      _services = <LlmService>[
        ..._services.sublist(0, index),
        service,
        ..._services.sublist(index + 1),
      ];
    } else {
      _services = <LlmService>[..._services, service];
    }
    // 还没有生效服务时，顺手把第一个有 key 的服务设为生效
    // （与 `kb_svc_salvo` 的 `upsert_service` 行为一致）。
    if (_activeServiceId == null && service.hasApiKey) {
      _activeServiceId = service.id;
    }
    notifyListeners();
    _persistServices();
  }

  /// 删除一个服务。
  void removeService(String id) {
    _services = _services
        .where((LlmService service) => service.id != id)
        .toList(growable: false);
    if (_activeServiceId == id) {
      _activeServiceId = _services
          .where((LlmService service) => service.hasApiKey)
          .map((LlmService service) => service.id)
          .firstOrNull;
    }
    notifyListeners();
    _persistServices();
  }

  /// 切换生效服务。
  void setActiveService(String id) {
    if (_activeServiceId == id) {
      return;
    }
    _activeServiceId = id;
    notifyListeners();
    _persistServices();
  }

  // ── 内部 ────────────────────────────────────────────────────────────

  void _normalizeSelection() {
    if (_workspaces.isEmpty) {
      _activeWorkspaceId = null;
      return;
    }
    if (_workspaceById(_activeWorkspaceId) == null) {
      _activeWorkspaceId = _workspaces.first.id;
    }
  }

  void _persistLayout() {
    unawaited(
      _store.saveLayout(
        themeMode: _themeMode,
        sidebarWidth: _sidebarWidth,
        sidebarCollapsed: _sidebarCollapsed,
        filePanelShown: _filePanelShown,
        filePanelWidth: _filePanelWidth,
      ),
    );
  }

  void _persistServices() {
    unawaited(
      _store.saveServices(
        services: _services,
        activeServiceId: _activeServiceId,
      ),
    );
  }

  /// 工作区的写入做 500ms 防抖：流式生成时消息会频繁变化，
  /// 每次增量都落盘既无必要也会拖慢界面。
  void _scheduleWorkspaceSave() {
    _workspaceSaveTimer?.cancel();
    _workspaceSaveTimer = Timer(const Duration(milliseconds: 500), () {
      unawaited(
        _store.saveWorkspaces(
          workspaces: _workspaces,
          activeWorkspaceId: _activeWorkspaceId,
        ),
      );
    });
  }

  @override
  void dispose() {
    _workspaceSaveTimer?.cancel();
    super.dispose();
  }
}
