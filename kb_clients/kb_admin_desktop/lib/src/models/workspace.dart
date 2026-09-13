// 工作区模型。
//
// 一个工作区对应磁盘上的一个目录，也是未来「工作区文件浏览器」的根。
// 服务端目前还没有工作区接口（`kb_core` 只起了 HTTP/WS 服务，见 dev-notes.md §1），
// 因此这里先由客户端自己保存工作区列表；等 `kb_core` 提供工作区接口后，
// [Workspace] 会退化成服务端条目的本地缓存。

import 'package:flutter/foundation.dart';

import 'chat_session.dart';

/// 一个知识库工作区。
@immutable
class Workspace {
  /// 构造一个工作区。
  const Workspace({
    required this.id,
    required this.name,
    required this.path,
    this.sessions = const <ChatSession>[],
    this.lastSessionId,
  });

  /// 用目录名新建一个工作区。
  factory Workspace.create({required String name, required String path}) =>
      Workspace(
        id: 'w-${DateTime.now().microsecondsSinceEpoch}',
        name: name,
        path: path,
      );

  /// 工作区标识。
  final String id;

  /// 展示名。
  final String name;

  /// 对应的磁盘目录。
  final String path;

  /// 工作区内的会话。
  final List<ChatSession> sessions;

  /// 上次停留的会话标识。
  final String? lastSessionId;

  /// 当前应当展示的会话；没有任何会话时为 `null`。
  ChatSession? get activeSession {
    if (sessions.isEmpty) {
      return null;
    }
    for (final ChatSession session in sessions) {
      if (session.id == lastSessionId) {
        return session;
      }
    }
    return sessions.first;
  }

  /// 返回一份替换了若干字段的副本。
  Workspace copyWith({
    String? name,
    String? path,
    List<ChatSession>? sessions,
    String? lastSessionId,
    bool clearLastSession = false,
  }) {
    return Workspace(
      id: id,
      name: name ?? this.name,
      path: path ?? this.path,
      sessions: sessions ?? this.sessions,
      lastSessionId: clearLastSession
          ? null
          : (lastSessionId ?? this.lastSessionId),
    );
  }

  /// 从 JSON 还原。
  factory Workspace.fromJson(Map<String, dynamic> json) => Workspace(
    id: json['id'] as String? ?? '',
    name: json['name'] as String? ?? '',
    path: json['path'] as String? ?? '',
    sessions: (json['sessions'] as List<dynamic>? ?? <dynamic>[])
        .map(
          (dynamic item) => ChatSession.fromJson(item as Map<String, dynamic>),
        )
        .toList(growable: false),
    lastSessionId: json['last_session_id'] as String?,
  );

  /// 序列化为 JSON。
  Map<String, dynamic> toJson() => <String, dynamic>{
    'id': id,
    'name': name,
    'path': path,
    if (lastSessionId != null) 'last_session_id': lastSessionId,
    'sessions': sessions
        .map((ChatSession session) => session.toJson())
        .toList(growable: false),
  };
}
