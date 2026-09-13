// 会话模型。
//
// 服务端目前是单会话内存态（见 `dev-notes.md` §4.6「多会话」尚未决策），
// 所以客户端这边先自己维护「工作区 → 会话」的两级结构：界面按这套结构渲染，
// 等服务端补上多会话接口，再把 [ChatSession] 的 id 换成服务端下发的即可。

import 'package:flutter/foundation.dart';

import 'chat_turn.dart';

/// 一次对话。
@immutable
class ChatSession {
  /// 构造一个会话。
  const ChatSession({
    required this.id,
    required this.title,
    this.turns = const <ChatTurn>[],
    required this.updatedAt,
  });

  /// 新建一个空会话。
  factory ChatSession.create({String title = '新会话'}) => ChatSession(
    id: 's-${DateTime.now().microsecondsSinceEpoch}',
    title: title,
    updatedAt: DateTime.now(),
  );

  /// 会话标识。
  final String id;

  /// 会话标题（取首条用户消息的前若干字）。
  final String title;

  /// 会话内的消息。
  final List<ChatTurn> turns;

  /// 最近一次活动时间。
  final DateTime updatedAt;

  /// 返回一份替换了若干字段的副本。
  ChatSession copyWith({
    String? title,
    List<ChatTurn>? turns,
    DateTime? updatedAt,
  }) {
    return ChatSession(
      id: id,
      title: title ?? this.title,
      turns: turns ?? this.turns,
      updatedAt: updatedAt ?? this.updatedAt,
    );
  }

  /// 从 JSON 还原。
  factory ChatSession.fromJson(Map<String, dynamic> json) => ChatSession(
    id: json['id'] as String? ?? '',
    title: json['title'] as String? ?? '新会话',
    turns: (json['turns'] as List<dynamic>? ?? <dynamic>[])
        .map((dynamic item) => ChatTurn.fromJson(item as Map<String, dynamic>))
        .toList(growable: false),
    updatedAt:
        DateTime.tryParse(json['updated_at'] as String? ?? '') ??
        DateTime.now(),
  );

  /// 序列化为 JSON。
  Map<String, dynamic> toJson() => <String, dynamic>{
    'id': id,
    'title': title,
    'updated_at': updatedAt.toIso8601String(),
    'turns': turns
        .where((ChatTurn turn) => !turn.isStreaming)
        .map((ChatTurn turn) => turn.toJson())
        .toList(growable: false),
  };
}
