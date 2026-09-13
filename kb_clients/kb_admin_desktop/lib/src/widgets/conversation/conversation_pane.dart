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

import 'package:flutter/material.dart';

import '../../models/workspace.dart';
import '../../models/chat_session.dart';
import '../../models/chat_turn.dart';
import '../../state/app_controller.dart';
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
  });

  /// 应用状态。
  final AppController controller;

  /// 整个框架的宽度，用于计算正文最大宽度。
  final double viewportWidth;

  @override
  State<ConversationPane> createState() => _ConversationPaneState();
}

class _ConversationPaneState extends State<ConversationPane> {
  final ScrollController _scroll = ScrollController();

  @override
  void dispose() {
    _scroll.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final AppController controller = widget.controller;
    final Workspace? workspace = controller.activeWorkspace;
    final ChatSession? session = controller.activeSession;
    final List<ChatTurn> turns = session?.turns ?? const <ChatTurn>[];

    // 新消息到达后保持贴底。这里只在消息条数变化时滚动，避免生成过程中的
    // 每次增量都强制拉到底部、抢走用户向上翻阅的位置。
    _scheduleScrollToBottom(turns.length);

    return DecoratedBox(
      decoration: BoxDecoration(color: context.dsw.bgBase),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: <Widget>[
          _ConversationHeader(
            workspace: workspace,
            session: session,
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
                          ? const _Hero()
                          : MessageList(turns: turns),
                    ),
                  ),
                );
              },
            ),
          ),
          _ComposerSeat(
            controller: controller,
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

/// 列头。
class _ConversationHeader extends StatelessWidget {
  const _ConversationHeader({
    required this.workspace,
    required this.session,
    required this.controller,
    required this.viewportWidth,
  });

  final Workspace? workspace;
  final ChatSession? session;
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
          Expanded(child: _Breadcrumb(workspace: workspace, session: session)),
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
  const _Breadcrumb({required this.workspace, required this.session});

  final Workspace? workspace;
  final ChatSession? session;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    final TextStyle base = DswTypography.body.copyWith(
      fontSize: 14,
      height: 20 / 14,
      color: c.labelTertiary,
    );

    if (workspace == null) {
      return Text('llm_kb', style: base.copyWith(color: c.labelPrimary));
    }

    return Row(
      children: <Widget>[
        Flexible(
          child: Text(
            workspace!.name,
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
            session?.title ?? '未选择会话',
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
    required this.viewportWidth,
  });

  final AppController controller;
  final double viewportWidth;

  @override
  Widget build(BuildContext context) {
    final ChatSession? session = controller.activeSession;
    final bool ready = session != null;

    return Composer(
      hint: ready ? '输入问题，Enter 发送，Shift+Enter 换行' : '先在工作区里新建一个会话',
      onSend: (String text) => _send(context, text),
    );
  }

  void _send(BuildContext context, String text) {
    final AppController controller = this.controller;
    final ChatSession? session = controller.activeSession;
    if (session == null) {
      return;
    }

    controller.appendTurn(ChatTurn.user(text));

    // 对话通道（`ws://<host>/ws/chat`，见 `kb_svc_salvo::wire`）尚未接入。
    // 这里先补一条说明性的助手消息，把消息渲染的各条分支跑通，也避免让界面
    // 看起来「发了没反应」。
    controller.appendTurn(
      ChatTurn(
        id: 'local-reply-${DateTime.now().microsecondsSinceEpoch}',
        role: ChatRole.assistant,
        state: ChatTurnState.done,
        notice: const ChatNotice(
          message:
              '对话通道尚未接入。下一阶段将通过 ws://<host>/ws/chat 连接 '
              'kb_svc_salvo，并按 abs_llm::v1 的帧格式增量渲染回答。',
        ),
      ),
    );
  }
}

/// 空对话时的引导区。
class _Hero extends StatelessWidget {
  const _Hero();

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
            '先在左下角「设置」里填好 LLM 服务的 API key，再从这里提问。',
            textAlign: TextAlign.center,
            style: DswTypography.body.copyWith(color: c.labelSecondary),
          ),
        ],
      ),
    );
  }
}
