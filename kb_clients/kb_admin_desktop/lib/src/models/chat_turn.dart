// 对话相关的数据模型。
//
// 字段命名与 `kb_svc_salvo::wire` 的浏览器侧帧保持一致（`abs_llm::v1` 的词汇），
// 这样等 HTTP / WebSocket 通道接上以后，只需要在服务层做一次 JSON 映射，
// 界面层不必再改。

import 'package:flutter/foundation.dart';

/// 一条消息的说话人。
enum ChatRole {
  /// 用户。
  user,

  /// 助手。
  assistant,
}

/// 一条助手消息的生成状态。
enum ChatTurnState {
  /// 正在增量生成。
  streaming,

  /// 已结束（正常结束或被取消）。
  done,

  /// 以错误结束。
  failed,
}

/// 助手输出文本所属的逻辑部分，对应 `wire::LogicOutput`。
enum TurnLogic {
  /// 面向最终答案的文本。
  answer,

  /// provider 公开的推理内容。
  reasoning,

  /// 函数调用。
  functionCall,

  /// 动态内容搜索。
  dynamicSearchCall,

  /// 静态内容搜索。
  staticSearchCall,
}

/// 一次工具调用的记录，对应 `wire::ServerMessage::ToolCall`。
@immutable
class ToolCallRecord {
  /// 构造一条工具调用记录。
  const ToolCallRecord({
    required this.id,
    required this.name,
    required this.arguments,
  });

  /// 工具调用标识。
  final String id;

  /// 工具名。
  final String name;

  /// 参数（原始 JSON 文本）。
  final String arguments;

  /// 从 JSON 还原。
  factory ToolCallRecord.fromJson(Map<String, dynamic> json) => ToolCallRecord(
    id: json['id'] as String? ?? '',
    name: json['name'] as String? ?? '',
    arguments: json['arguments'] as String? ?? '',
  );

  /// 序列化为 JSON。
  Map<String, dynamic> toJson() => <String, dynamic>{
    'id': id,
    'name': name,
    'arguments': arguments,
  };
}

/// token 用量，对应 `wire::Usage`。
@immutable
class TokenUsage {
  /// 构造一份用量。
  const TokenUsage({this.inputTokens, this.outputTokens, this.totalTokens});

  /// 输入 token 数。
  final int? inputTokens;

  /// 输出 token 数。
  final int? outputTokens;

  /// 总 token 数。
  final int? totalTokens;

  /// 是否三个计数都缺失。
  bool get isEmpty =>
      inputTokens == null && outputTokens == null && totalTokens == null;

  /// 从 JSON 还原。
  factory TokenUsage.fromJson(Map<String, dynamic> json) => TokenUsage(
    inputTokens: (json['input_tokens'] as num?)?.toInt(),
    outputTokens: (json['output_tokens'] as num?)?.toInt(),
    totalTokens: (json['total_tokens'] as num?)?.toInt(),
  );

  /// 序列化为 JSON。
  Map<String, dynamic> toJson() => <String, dynamic>{
    if (inputTokens != null) 'input_tokens': inputTokens,
    if (outputTokens != null) 'output_tokens': outputTokens,
    if (totalTokens != null) 'total_tokens': totalTokens,
  };
}

/// 一条附在助手消息上的提示（错误或说明），对应网页端的 `notice` 节点。
@immutable
class ChatNotice {
  /// 构造一条提示。
  const ChatNotice({required this.message, this.isError = false});

  /// 提示正文。
  final String message;

  /// 是否为错误提示。
  final bool isError;

  /// 从 JSON 还原。
  factory ChatNotice.fromJson(Map<String, dynamic> json) => ChatNotice(
    message: json['message'] as String? ?? '',
    isError: json['is_error'] as bool? ?? false,
  );

  /// 序列化为 JSON。
  Map<String, dynamic> toJson() => <String, dynamic>{
    'message': message,
    'is_error': isError,
  };
}

/// 对话中的一条消息。
@immutable
class ChatTurn {
  /// 构造一条消息。
  const ChatTurn({
    required this.id,
    required this.role,
    this.text = '',
    this.reasoning = '',
    this.state = ChatTurnState.done,
    this.toolCalls = const <ToolCallRecord>[],
    this.usage,
    this.notice,
  });

  /// 构造一条用户消息。
  factory ChatTurn.user(String text) => ChatTurn(
    id: 'local-${DateTime.now().microsecondsSinceEpoch}',
    role: ChatRole.user,
    text: text,
  );

  /// 消息标识（后续接后端时就是 `turn_id`）。
  final String id;

  /// 说话人。
  final ChatRole role;

  /// 回答正文。
  final String text;

  /// 推理正文。
  final String reasoning;

  /// 生成状态。
  final ChatTurnState state;

  /// 本轮的工具调用。
  final List<ToolCallRecord> toolCalls;

  /// token 用量。
  final TokenUsage? usage;

  /// 错误或说明。
  final ChatNotice? notice;

  /// 是否正在生成。
  bool get isStreaming => state == ChatTurnState.streaming;

  /// 返回一份替换了若干字段的副本。
  ChatTurn copyWith({
    String? text,
    String? reasoning,
    ChatTurnState? state,
    List<ToolCallRecord>? toolCalls,
    TokenUsage? usage,
    ChatNotice? notice,
  }) {
    return ChatTurn(
      id: id,
      role: role,
      text: text ?? this.text,
      reasoning: reasoning ?? this.reasoning,
      state: state ?? this.state,
      toolCalls: toolCalls ?? this.toolCalls,
      usage: usage ?? this.usage,
      notice: notice ?? this.notice,
    );
  }

  /// 从 JSON 还原。
  factory ChatTurn.fromJson(Map<String, dynamic> json) => ChatTurn(
    id: json['id'] as String? ?? '',
    role: json['role'] == 'assistant' ? ChatRole.assistant : ChatRole.user,
    text: json['text'] as String? ?? '',
    reasoning: json['reasoning'] as String? ?? '',
    state: ChatTurnState.values.firstWhere(
      (ChatTurnState value) => value.name == json['state'],
      orElse: () => ChatTurnState.done,
    ),
    toolCalls: (json['tool_calls'] as List<dynamic>? ?? <dynamic>[])
        .map(
          (dynamic item) =>
              ToolCallRecord.fromJson(item as Map<String, dynamic>),
        )
        .toList(growable: false),
    usage: json['usage'] == null
        ? null
        : TokenUsage.fromJson(json['usage'] as Map<String, dynamic>),
    notice: json['notice'] == null
        ? null
        : ChatNotice.fromJson(json['notice'] as Map<String, dynamic>),
  );

  /// 序列化为 JSON。
  ///
  /// 正在生成中的消息不会被持久化（[ChatTurnState.streaming] 落盘没有意义，
  /// 恢复后也不可能继续），由调用方过滤。
  Map<String, dynamic> toJson() => <String, dynamic>{
    'id': id,
    'role': role.name,
    'text': text,
    'reasoning': reasoning,
    'state': state.name,
    if (toolCalls.isNotEmpty)
      'tool_calls': toolCalls
          .map((ToolCallRecord call) => call.toJson())
          .toList(growable: false),
    if (usage != null) 'usage': usage!.toJson(),
    if (notice != null) 'notice': notice!.toJson(),
  };
}
