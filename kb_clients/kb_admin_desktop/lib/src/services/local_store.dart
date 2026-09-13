// 客户端本地存储。
//
// 存两类东西：
//
// 1. **界面偏好**：侧边栏宽度 / 是否折叠、文件面板是否展开、主题模式；
// 2. **连接参数与工作区**：LLM 服务列表（含 API key）与「工作区 → 会话」树。
//
// 为什么不放服务端：`kb_svc_salvo` 目前只提供 `GET/POST /api/settings` 与
// `/ws/chat`（见 `kb_svc_salvo/src/web.rs`），没有工作区、会话与历史接口。
// 等 `kb_core` 补上这些接口，本文件里除了界面偏好之外的部分都应改为服务端读写。

import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../models/llm_service.dart';
import '../models/workspace.dart';

/// 已经落盘的一份客户端状态快照。
@immutable
class LocalSnapshot {
  /// 构造一份快照。
  const LocalSnapshot({
    this.themeMode = ThemeMode.dark,
    this.sidebarWidth = 0,
    this.sidebarCollapsed = false,
    this.filePanelShown = false,
    this.filePanelWidth,
    this.services = const <LlmService>[],
    this.activeServiceId,
    this.workspaces = const <Workspace>[],
    this.activeWorkspaceId,
  });

  /// 主题模式。
  final ThemeMode themeMode;

  /// 侧边栏宽度偏好；0 表示还没被用户拖过。
  final double sidebarWidth;

  /// 侧边栏是否被手动折叠。
  final bool sidebarCollapsed;

  /// 右侧文件面板是否展开。
  final bool filePanelShown;

  /// 右侧文件面板宽度偏好；`null` 表示按视口比例取默认值。
  final double? filePanelWidth;

  /// 已配置的 LLM 服务。
  final List<LlmService> services;

  /// 当前生效的服务标识。
  final String? activeServiceId;

  /// 工作区列表。
  final List<Workspace> workspaces;

  /// 当前选中的工作区标识。
  final String? activeWorkspaceId;
}

/// 基于 `shared_preferences` 的本地存储。
///
/// 所有读写都是「尽力而为」：读失败或数据损坏时退回默认值，不阻塞界面启动。
class LocalStore {
  LocalStore._(this._prefs);

  final SharedPreferences _prefs;

  static const String _themeModeKey = 'kb.theme_mode';
  static const String _sidebarWidthKey = 'kb.sidebar_width';
  static const String _sidebarCollapsedKey = 'kb.sidebar_collapsed';
  static const String _filePanelShownKey = 'kb.file_panel_shown';
  static const String _filePanelWidthKey = 'kb.file_panel_width';
  static const String _servicesKey = 'kb.services';
  static const String _activeServiceKey = 'kb.active_service';
  static const String _workspacesKey = 'kb.workspaces';
  static const String _activeWorkspaceKey = 'kb.active_workspace';

  /// 打开本地存储。
  static Future<LocalStore> open() async =>
      LocalStore._(await SharedPreferences.getInstance());

  /// 读取整份快照。
  LocalSnapshot load() {
    return LocalSnapshot(
      themeMode: _readThemeMode(),
      sidebarWidth: _prefs.getDouble(_sidebarWidthKey) ?? 0,
      sidebarCollapsed: _prefs.getBool(_sidebarCollapsedKey) ?? false,
      filePanelShown: _prefs.getBool(_filePanelShownKey) ?? false,
      filePanelWidth: _prefs.getDouble(_filePanelWidthKey),
      services: _readServices(),
      activeServiceId: _prefs.getString(_activeServiceKey),
      workspaces: _readWorkspaces(),
      activeWorkspaceId: _prefs.getString(_activeWorkspaceKey),
    );
  }

  /// 保存界面偏好。
  Future<void> saveLayout({
    required ThemeMode themeMode,
    required double sidebarWidth,
    required bool sidebarCollapsed,
    required bool filePanelShown,
    required double? filePanelWidth,
  }) async {
    await _prefs.setString(_themeModeKey, themeMode.name);
    await _prefs.setDouble(_sidebarWidthKey, sidebarWidth);
    await _prefs.setBool(_sidebarCollapsedKey, sidebarCollapsed);
    await _prefs.setBool(_filePanelShownKey, filePanelShown);
    if (filePanelWidth == null) {
      await _prefs.remove(_filePanelWidthKey);
    } else {
      await _prefs.setDouble(_filePanelWidthKey, filePanelWidth);
    }
  }

  /// 保存 LLM 服务列表与生效服务。
  Future<void> saveServices({
    required List<LlmService> services,
    required String? activeServiceId,
  }) async {
    await _prefs.setString(
      _servicesKey,
      jsonEncode(
        services
            .map((LlmService service) => service.toJson())
            .toList(growable: false),
      ),
    );
    if (activeServiceId == null) {
      await _prefs.remove(_activeServiceKey);
    } else {
      await _prefs.setString(_activeServiceKey, activeServiceId);
    }
  }

  /// 保存工作区树。
  Future<void> saveWorkspaces({
    required List<Workspace> workspaces,
    required String? activeWorkspaceId,
  }) async {
    await _prefs.setString(
      _workspacesKey,
      jsonEncode(
        workspaces
            .map((Workspace workspace) => workspace.toJson())
            .toList(growable: false),
      ),
    );
    if (activeWorkspaceId == null) {
      await _prefs.remove(_activeWorkspaceKey);
    } else {
      await _prefs.setString(_activeWorkspaceKey, activeWorkspaceId);
    }
  }

  ThemeMode _readThemeMode() {
    final String? raw = _prefs.getString(_themeModeKey);
    for (final ThemeMode mode in ThemeMode.values) {
      if (mode.name == raw) {
        return mode;
      }
    }
    return ThemeMode.dark;
  }

  List<LlmService> _readServices() => _readJsonList(
    _servicesKey,
    (Map<String, dynamic> json) => LlmService.fromJson(json),
  );

  List<Workspace> _readWorkspaces() => _readJsonList(
    _workspacesKey,
    (Map<String, dynamic> json) => Workspace.fromJson(json),
  );

  /// 解析一个「JSON 数组」型的键；任何异常都退回空列表。
  List<T> _readJsonList<T>(
    String key,
    T Function(Map<String, dynamic> json) decode,
  ) {
    final String? raw = _prefs.getString(key);
    if (raw == null || raw.isEmpty) {
      return <T>[];
    }
    try {
      final dynamic decoded = jsonDecode(raw);
      if (decoded is! List<dynamic>) {
        return <T>[];
      }
      return decoded
          .whereType<Map<String, dynamic>>()
          .map(decode)
          .toList(growable: false);
    } on FormatException {
      return <T>[];
    }
  }
}
