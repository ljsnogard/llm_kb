// 中间对话区。
//
// 对齐 DSH 的 `ui-conversation` 骨架：
//
// - 顶部是 76px 的列头（`padding: 10px 28px 0 20px`，底部一条 0.5px 分隔线），
//   左侧是「工作区 / 会话」面包屑，右侧角落放**打开右侧文件面板的按钮**
//   （DSH 把这个按钮放在会话头的 corner 座席，而不是右侧栏里——这样面板收起时
//   对话区不付出任何宽度代价）；
// - 中间是消息列表，正文最大宽度 `clamp(680px, 64%, 920px)` 并居中；
// - 底部是输入区。
//
// # 两种数据来源
//
// - **已连上 `kb_core`**：正文来自服务端（`GetSession` 读历史、`Ask` 提问），
//   提问后服务端会把一问一答落盘，重新连线仍然看得到；
// - **未连接**：退回本地那一套（`AppController`），这样没有服务端也能起界面。
//   本地那一套里"发送"只是补一条说明性消息，不会真的提问。

import 'package:flutter/material.dart';

import '../../models/chat_session.dart';
import '../../models/chat_turn.dart';
import '../../models/workspace.dart';
import '../../services/kb_client_api.dart';
import '../../state/app_controller.dart';
import '../../state/connection_controller.dart';
import '../../theme/app_theme.dart';
import '../../theme/dsw_tokens.dart';
import '../common/dsw_controls.dart';
import '../common/dsw_icons.dart';
import 'composer.dart';
import 'message_list.dart';

/// 对话区。
class ConversationPane extends StatefulWidget {
  /// 构造对话区。
  const ConversationPane({
    super.key,
    required this.controller,
    required this.viewportWidth,
    this.connection,
  });

  /// 应用状态。
  final AppController controller;

  /// 整个框架的宽度，用于计算正文最大宽度。
  final double viewportWidth;

  /// 与 `kb_core` 的连接状态；为 `null` 时对话区走本地那一套。
  final ConnectionController? connection;

  @override
  State<ConversationPane> createState() => _ConversationPaneState();
}

class _ConversationPaneState extends State<ConversationPane> {
  final ScrollController _scroll = ScrollController();

  @override
  void initState() {
    super.initState();
    // 正文来自连接状态（选中的会话、正文缓存、加载中标记）；`KbAdminApp` 只在
    // `AppController` 变化时重建，所以这里得自己跟着连接状态重建。
    widget.connection?.addListener(_onConnectionChanged_);
  }

  @override
  void didUpdateWidget(ConversationPane oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.connection != widget.connection) {
      oldWidget.connection?.removeListener(_onConnectionChanged_);
      widget.connection?.addListener(_onConnectionChanged_);
    }
  }

  @override
  void dispose() {
    widget.connection?.removeListener(_onConnectionChanged_);
    _scroll.dispose();
    super.dispose();
  }

  void _onConnectionChanged_() {
    if (mounted) {
      setState(() {});
    }
  }

  @override
  Widget build(BuildContext context) {
    final AppController controller = widget.controller;
    final ConnectionController? connection = widget.connection;
    final bool serverMode = connection != null && connection.connected;

    final String? workspaceName;
    final String? sessionTitle;
    final List<ChatTurn> turns;
    final String emptyHint;

    if (serverMode) {
      workspaceName = connection.selectedServerWorkspace?.name;
      sessionTitle = connection.selectedServerSession?.title;
      final SessionDetailReport? detail = connection.selectedSessionDetail;
      turns = detail == null
          ? const <ChatTurn>[]
          : detail.turns.map(chatTurnOf_).toList(growable: false);
      emptyHint = connection.selectedSessionId == null
          ? '在左侧选一个会话，或者在工作区那一行点 + 新建一个。'
          : (connection.isLoadingDetail(connection.selectedSessionId!)
                ? '正在读取会话…'
                : '这个会话还没有消息。在下面输入问题，kb_core 会把它记下来。');
    } else {
      final Workspace? workspace = controller.activeWorkspace;
      final ChatSession? session = controller.activeSession;
      workspaceName = workspace?.name;
      sessionTitle = session?.title;
      turns = session?.turns ?? const <ChatTurn>[];
      emptyHint = '先在左侧栏新建一个会话。';
    }

    // 新消息到达后保持贴底。这里只在消息条数变化时滚动，避免生成过程中的
    // 每次增量都强制拉到底部、抢走用户向上翻阅的位置。
    _scheduleScrollToBottom(turns.length);

    return DecoratedBox(
      decoration: BoxDecoration(color: context.dsw.bgBase),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: <Widget>[
          _ConversationHeader(
            workspaceName: workspaceName,
            sessionTitle: sessionTitle,
            controller: controller,
            viewportWidth: widget.viewportWidth,
          ),
          Expanded(
            child: LayoutBuilder(
              builder: (BuildContext context, BoxConstraints constraints) {
                final double contentWidth = _contentWidth(constraints.maxWidth);
                return SingleChildScrollView(
                  controller: _scroll,
                  padding: const EdgeInsets.symmetric(
                    horizontal: 32,
                    vertical: 16,
                  ),
                  child: Center(
                    child: ConstrainedBox(
                      constraints: BoxConstraints(maxWidth: contentWidth),
                      child: turns.isEmpty
                          ? _Hero(hint: emptyHint)
                          : MessageList(turns: turns),
                    ),
                  ),
                );
              },
            ),
          ),
          _ComposerSeat(
            controller: controller,
            connection: connection,
            serverMode: serverMode,
            viewportWidth: widget.viewportWidth,
          ),
        ],
      ),
    );
  }

  /// 正文最大宽度：`clamp(680, 64% 列宽, 920)`（DSH 的 `--dsh-chat-content-width`）。
  double _contentWidth(double columnWidth) =>
      (columnWidth * 0.64).clamp(680.0, 920.0);

  int _lastTurnCount = 0;

  void _scheduleScrollToBottom(int turnCount) {
    if (turnCount == _lastTurnCount) {
      return;
    }
    _lastTurnCount = turnCount;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!_scroll.hasClients) {
        return;
      }
      _scroll.animateTo(
        _scroll.position.maxScrollExtent,
        duration: DswMotion.normal,
        curve: DswMotion.easeInOut,
      );
    });
  }
}

/// 把服务端的一条消息翻成界面用的 [ChatTurn]。
///
/// 协议里的 `Turn` 与界面模型的字段本来就一一对应（见
/// `abs_kb_svc_v1_desktop::content_` 的模块文档），这里只做一次机械映射；
/// `system` / `tool` 之类的角色暂时按助手样式渲染。
ChatTurn chatTurnOf_(TurnView turn) => ChatTurn(
  id: turn.id,
  role: turn.role == 'assistant' ? ChatRole.assistant : ChatRole.user,
  text: turn.text,
  reasoning: turn.reasoning,
  state: switch (turn.state) {
    'streaming' => ChatTurnState.streaming,
    'failed' => ChatTurnState.failed,
    _ => ChatTurnState.done,
  },
  notice: turn.notice.isEmpty
      ? null
      : ChatNotice(message: turn.notice, isError: turn.noticeIsError),
);

/// 列头。
class _ConversationHeader extends StatelessWidget {
  const _ConversationHeader({
    required this.workspaceName,
    required this.sessionTitle,
    required this.controller,
    required this.viewportWidth,
  });

  final String? workspaceName;
  final String? sessionTitle;
  final AppController controller;
  final double viewportWidth;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    return Container(
      constraints: const BoxConstraints(minHeight: 76),
      padding: const EdgeInsets.fromLTRB(20, 10, 28, 0),
      decoration: BoxDecoration(
        border: Border(bottom: BorderSide(color: c.borderL3, width: 0.5)),
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.center,
        children: <Widget>[
          Expanded(
            child: _Breadcrumb(
              workspaceName: workspaceName,
              sessionTitle: sessionTitle,
            ),
          ),
          const SizedBox(width: 20),
          _FilePanelToggle(
            controller: controller,
            viewportWidth: viewportWidth,
          ),
        ],
      ),
    );
  }
}

/// 「工作区 / 会话」面包屑。
class _Breadcrumb extends StatelessWidget {
  const _Breadcrumb({required this.workspaceName, required this.sessionTitle});

  final String? workspaceName;
  final String? sessionTitle;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    final TextStyle base = DswTypography.body.copyWith(
      fontSize: 14,
      height: 20 / 14,
      color: c.labelTertiary,
    );

    if (workspaceName == null) {
      return Text('llm_kb', style: base.copyWith(color: c.labelPrimary));
    }

    return Row(
      children: <Widget>[
        Flexible(
          child: Text(
            workspaceName!,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: base,
          ),
        ),
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: 6),
          child: Text('/', style: base.copyWith(color: c.labelCaption)),
        ),
        Flexible(
          child: Text(
            sessionTitle ?? '未选择会话',
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: base.copyWith(
              color: c.labelPrimary,
              fontWeight: FontWeight.w500,
            ),
          ),
        ),
      ],
    );
  }
}

/// 右侧文件面板的开合按钮。
///
/// 对应 DSH 的 `ExpandButton`：面板收起时显示，展开时不渲染；图标是左侧栏
/// 那个面板图标的镜像。
class _FilePanelToggle extends StatelessWidget {
  const _FilePanelToggle({
    required this.controller,
    required this.viewportWidth,
  });

  final AppController controller;
  final double viewportWidth;

  @override
  Widget build(BuildContext context) {
    final bool shown = controller.filePanelShown;
    return DswIconButton(
      tooltip: shown ? '隐藏工作区文件' : '显示工作区文件',
      onPressed: () => controller.toggleFilePanel(viewportWidth),
      size: 28,
      iconSize: 15,
      glyph: PanelLeftIcon(
        size: 15,
        mirrored: !shown,
        color: context.dsw.labelSecondary,
      ),
    );
  }
}

/// 输入区座席：包一层与正文同宽的容器。
class _ComposerSeat extends StatelessWidget {
  const _ComposerSeat({
    required this.controller,
    required this.connection,
    required this.serverMode,
    required this.viewportWidth,
  });

  final AppController controller;
  final ConnectionController? connection;
  final bool serverMode;
  final double viewportWidth;

  @override
  Widget build(BuildContext context) {
    if (serverMode) {
      final bool hasSession = connection!.selectedSessionId != null;
      return Composer(
        hint: hasSession ? '输入问题，Enter 发送，Shift+Enter 换行' : '先在左侧选一个会话',
        onSend: (String text) => _sendServer(context, text),
      );
    }

    final ChatSession? session = controller.activeSession;
    final bool ready = session != null;
    return Composer(
      hint: ready ? '输入问题，Enter 发送，Shift+Enter 换行' : '先在工作区里新建一个会话',
      onSend: (String text) => _sendLocal(context, text),
    );
  }

  /// 已连接：真的把问题发给 `kb_core`。
  void _sendServer(BuildContext context, String text) {
    final ConnectionController? conn = connection;
    if (conn == null) {
      return;
    }
    conn.ask(text).then((String error) {
      if (error.isNotEmpty && context.mounted) {
        ScaffoldMessenger.of(
          context,
        ).showSnackBar(SnackBar(content: Text(error)));
      }
    });
  }

  /// 未连接：退回本地那一套（补一条说明性助手消息）。
  void _sendLocal(BuildContext context, String text) {
    final AppController controller = this.controller;
    final ChatSession? session = controller.activeSession;
    if (session == null) {
      return;
    }

    controller.appendTurn(ChatTurn.user(text));
    controller.appendTurn(
      ChatTurn(
        id: 'local-reply-${DateTime.now().microsecondsSinceEpoch}',
        role: ChatRole.assistant,
        state: ChatTurnState.done,
        notice: const ChatNotice(
          message: '还没有连接 kb_core：这条只在本地。连上之后提问会由服务端记录，'
              '重新连线也还在。',
        ),
      ),
    );
  }
}

/// 空对话时的引导区。
class _Hero extends StatelessWidget {
  const _Hero({required this.hint});

  final String hint;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    return Padding(
      padding: const EdgeInsets.only(top: 72),
      child: Column(
        children: <Widget>[
          Row(
            mainAxisAlignment: MainAxisAlignment.center,
            children: <Widget>[
              Container(
                width: 10,
                height: 10,
                decoration: BoxDecoration(
                  color: c.stateBusinessPrimary,
                  shape: BoxShape.circle,
                ),
              ),
              const SizedBox(width: 10),
              Text(
                'llm_kb',
                style: DswTypography.body.copyWith(
                  fontSize: 26,
                  height: 32 / 26,
                  fontWeight: FontWeight.w500,
                  color: c.labelPrimary,
                ),
              ),
            ],
          ),
          const SizedBox(height: 12),
          Text(
            hint,
            textAlign: TextAlign.center,
            style: DswTypography.body.copyWith(color: c.labelSecondary),
          ),
        ],
      ),
    );
  }
}
