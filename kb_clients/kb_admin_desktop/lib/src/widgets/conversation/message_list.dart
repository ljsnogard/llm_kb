// 对话正文（消息列表）。
//
// 观感对齐 DSH 的 `ui-chat`：
//
// - **用户消息**是右对齐的胶囊气泡：圆角 22、内边距 10/16、底色
//   `--dsw-specific-bubble`（不是蓝色，这是 DSH 的取舍）；
// - **助手消息没有气泡**，是通栏的正文，行高 24，工具调用与推理分别用独立
//   的块呈现；
// - 生成中的助手消息末尾有一个闪烁的方块光标（`app.css` 的 `.streaming::after`）。

import 'package:flutter/material.dart';

import '../../models/chat_turn.dart';
import '../../theme/app_theme.dart';
import '../../theme/dsw_tokens.dart';

/// 一组消息。
class MessageList extends StatelessWidget {
  /// 构造消息列表。
  const MessageList({super.key, required this.turns});

  /// 要渲染的消息。
  final List<ChatTurn> turns;

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: <Widget>[
        for (int i = 0; i < turns.length; i++) ...<Widget>[
          if (i > 0) const SizedBox(height: 16),
          _TurnView(turn: turns[i]),
        ],
      ],
    );
  }
}

/// 单条消息。
class _TurnView extends StatelessWidget {
  const _TurnView({required this.turn});

  final ChatTurn turn;

  @override
  Widget build(BuildContext context) {
    return turn.role == ChatRole.user
        ? _UserTurn(turn: turn)
        : _AssistantTurn(turn: turn);
  }
}

/// 用户气泡。
class _UserTurn extends StatelessWidget {
  const _UserTurn({required this.turn});

  final ChatTurn turn;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    return Align(
      alignment: Alignment.centerRight,
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 560),
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 10),
          decoration: BoxDecoration(
            color: c.bubble,
            borderRadius: BorderRadius.circular(22),
          ),
          child: Text(
            turn.text,
            style: DswTypography.body.copyWith(color: c.labelPrimary),
          ),
        ),
      ),
    );
  }
}

/// 助手消息：无气泡，通栏渲染。
class _AssistantTurn extends StatelessWidget {
  const _AssistantTurn({required this.turn});

  final ChatTurn turn;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: <Widget>[
        if (turn.reasoning.isNotEmpty) ...<Widget>[
          _ReasoningBlock(turn: turn),
          const SizedBox(height: 12),
        ],
        for (final ToolCallRecord call in turn.toolCalls) ...<Widget>[
          _ToolCallBlock(call: call),
          const SizedBox(height: 12),
        ],
        if (turn.text.isNotEmpty)
          _AnswerBody(text: turn.text, streaming: turn.isStreaming)
        else if (turn.isStreaming)
          const _StreamingCaret(),
        if (turn.notice != null) ...<Widget>[
          const SizedBox(height: 12),
          _NoticeBlock(notice: turn.notice!),
        ],
        if (turn.usage != null && !turn.usage!.isEmpty) ...<Widget>[
          const SizedBox(height: 12),
          Text(
            _formatUsage(turn.usage!),
            style: DswTypography.caption.copyWith(color: c.labelCaption),
          ),
        ],
      ],
    );
  }

  /// 用量行，文案与网页端一致：「token：输入 X / 输出 Y / 合计 Z」。
  String _formatUsage(TokenUsage usage) {
    final List<String> parts = <String>[];
    if (usage.inputTokens != null) {
      parts.add('输入 ${usage.inputTokens}');
    }
    if (usage.outputTokens != null) {
      parts.add('输出 ${usage.outputTokens}');
    }
    if (usage.totalTokens != null) {
      parts.add('合计 ${usage.totalTokens}');
    }
    return 'token：${parts.join(' / ')}';
  }
}

/// 推理块：生成中默认展开，结束后默认折叠。
class _ReasoningBlock extends StatefulWidget {
  const _ReasoningBlock({required this.turn});

  final ChatTurn turn;

  @override
  State<_ReasoningBlock> createState() => _ReasoningBlockState();
}

class _ReasoningBlockState extends State<_ReasoningBlock> {
  bool? _expanded;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    final bool expanded = _expanded ?? widget.turn.isStreaming;

    return Container(
      padding: const EdgeInsets.only(left: 12),
      decoration: BoxDecoration(
        border: Border(left: BorderSide(color: c.borderL3, width: 2)),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: <Widget>[
          GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTap: () => setState(() => _expanded = !expanded),
            child: Row(
              children: <Widget>[
                Icon(
                  expanded
                      ? Icons.keyboard_arrow_down_rounded
                      : Icons.keyboard_arrow_right_rounded,
                  size: 16,
                  color: c.labelCaption,
                ),
                Text(
                  '思考过程',
                  style: DswTypography.caption.copyWith(color: c.labelCaption),
                ),
                const SizedBox(width: 8),
                Text(
                  widget.turn.isStreaming ? '生成中' : '已完成',
                  style: DswTypography.caption.copyWith(color: c.labelCaption),
                ),
              ],
            ),
          ),
          if (expanded)
            Padding(
              padding: const EdgeInsets.only(top: 8),
              child: Text(
                widget.turn.reasoning,
                style: DswTypography.body.copyWith(
                  fontSize: 13,
                  color: c.labelSecondary,
                ),
              ),
            ),
        ],
      ),
    );
  }
}

/// 工具调用：虚线框 + 工具名。
class _ToolCallBlock extends StatelessWidget {
  const _ToolCallBlock({required this.call});

  final ToolCallRecord call;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(10),
        border: Border.all(color: c.borderL3),
      ),
      child: Row(
        children: <Widget>[
          Text(
            '工具调用',
            style: DswTypography.caption.copyWith(color: c.labelSecondary),
          ),
          const SizedBox(width: 8),
          Text(
            call.name,
            style: DswTypography.mono.copyWith(color: c.labelPrimary),
          ),
        ],
      ),
    );
  }
}

/// 错误 / 说明提示条。
class _NoticeBlock extends StatelessWidget {
  const _NoticeBlock({required this.notice});

  final ChatNotice notice;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    final bool error = notice.isError;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(10),
        color: error
            ? c.stateError.withValues(alpha: 0.12)
            : c.interactiveHover,
        border: Border.all(
          color: error
              ? c.stateError.withValues(alpha: 0.45)
              : c.borderL3,
        ),
      ),
      child: Text(
        notice.message,
        style: DswTypography.body.copyWith(
          fontSize: 13,
          color: error ? c.labelPrimary : c.labelSecondary,
        ),
      ),
    );
  }
}

/// 答案正文。
///
/// 与网页端 `app.js` 的处理一致：把 ``` 围栏切成等宽代码块，其余按纯文本
/// 渲染（保留换行）。这里刻意不做完整的 Markdown 解析——模型输出永远当作文本
/// 对待，不会当作标记解析。
class _AnswerBody extends StatelessWidget {
  const _AnswerBody({required this.text, required this.streaming});

  final String text;
  final bool streaming;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    final List<_Segment> segments = _splitFences(text);

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: <Widget>[
        for (int i = 0; i < segments.length; i++) ...<Widget>[
          if (i > 0) const SizedBox(height: 12),
          if (segments[i].isCode)
            Container(
              width: double.infinity,
              padding: const EdgeInsets.all(12),
              decoration: BoxDecoration(
                color: c.bgLayer1,
                borderRadius: BorderRadius.circular(10),
                border: Border.all(color: c.borderL3),
              ),
              child: SingleChildScrollView(
                scrollDirection: Axis.horizontal,
                child: Text(
                  segments[i].text,
                  style: DswTypography.mono.copyWith(color: c.labelPrimary),
                ),
              ),
            )
          else
            Text(
              segments[i].text,
              style: DswTypography.body.copyWith(
                color: c.labelPrimary,
                height: 24 / 14,
              ),
            ),
          if (i == segments.length - 1 && streaming) ...<Widget>[
            const SizedBox(height: 2),
            const _StreamingCaret(),
          ],
        ],
      ],
    );
  }

  /// 按 ``` 围栏把正文切成普通文本与代码段。
  List<_Segment> _splitFences(String source) {
    final List<_Segment> segments = <_Segment>[];
    final List<String> buffer = <String>[];
    bool inCode = false;

    void flush() {
      if (buffer.isEmpty) {
        return;
      }
      final String joined = buffer.join('\n');
      if (joined.trim().isNotEmpty) {
        segments.add(_Segment(text: joined, isCode: inCode));
      }
      buffer.clear();
    }

    for (final String line in source.split('\n')) {
      if (line.trimLeft().startsWith('```')) {
        flush();
        inCode = !inCode;
        continue;
      }
      buffer.add(line);
    }
    flush();
    return segments;
  }
}

/// 正文的一个片段。
class _Segment {
  const _Segment({required this.text, required this.isCode});

  final String text;
  final bool isCode;
}

/// 生成中的闪烁光标，对应 `app.css` 的 `.streaming::after`。
class _StreamingCaret extends StatefulWidget {
  const _StreamingCaret();

  @override
  State<_StreamingCaret> createState() => _StreamingCaretState();
}

class _StreamingCaretState extends State<_StreamingCaret>
    with SingleTickerProviderStateMixin {
  late final AnimationController _controller = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 1000),
  )..repeat();

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    return SizedBox(
      height: 20,
      child: AnimatedBuilder(
        animation: _controller,
        builder: (BuildContext context, Widget? child) {
          // `steps(2, start)` 的等价物：整段周期里一半时间可见。
          final bool visible = _controller.value < 0.5;
          return Opacity(opacity: visible ? 1 : 0, child: child);
        },
        child: Text(
          '▍',
          style: DswTypography.body.copyWith(color: c.labelCaption),
        ),
      ),
    );
  }
}
