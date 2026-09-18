// 服务端（`kb_core`）的工作区 → 会话列表。
//
// 与本地版 [WorkspaceList] 长得一样，但**数据来自 `kb_core`**：
//
// - 展开一个工作区时才去拉它的会话（`list_sessions`），拉过就缓存；
// - 只读：增删工作区 / 会话要发 `AddWorkspace` / `CreateSession`，下一轮再接；
// - 顶部有刷新按钮，重拉工作区列表。
//
// 它只读 [ConnectionController]，不碰 `AppController`。

import 'package:flutter/material.dart';

import '../../services/kb_client_api.dart';
import '../../state/connection_controller.dart';
import '../../theme/app_theme.dart';
import '../../theme/dsw_tokens.dart';
import '../common/dsw_controls.dart';
import '../common/dsw_icons.dart';
import '../common/dsw_list_row.dart';

/// `kb_core` 上的工作区与会话两级列表。
class ServerWorkspaceList extends StatefulWidget {
  /// 构造服务端工作区列表。
  const ServerWorkspaceList({super.key, required this.connection});

  /// 连接状态。
  final ConnectionController connection;

  @override
  State<ServerWorkspaceList> createState() => _ServerWorkspaceListState();
}

class _ServerWorkspaceListState extends State<ServerWorkspaceList> {
  /// 被用户手动收起的工作区（默认全展开）。
  final Set<String> _collapsed = <String>{};

  @override
  void initState() {
    super.initState();
    // 列表内容全部来自连接状态（工作区、每个工作区的会话、加载中标记），
    // 所以这里必须跟着它重建——`AnimatedBuilder` 只会重建它包住的那一块。
    widget.connection.addListener(_onConnectionChanged_);
  }

  @override
  void didUpdateWidget(ServerWorkspaceList oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.connection != widget.connection) {
      oldWidget.connection.removeListener(_onConnectionChanged_);
      widget.connection.addListener(_onConnectionChanged_);
    }
  }

  @override
  void dispose() {
    widget.connection.removeListener(_onConnectionChanged_);
    super.dispose();
  }

  void _onConnectionChanged_() {
    if (mounted) {
      setState(() {});
    }
  }

  @override
  Widget build(BuildContext context) {
    final ConnectionController connection = widget.connection;
    final List<WorkspaceView> workspaces = connection.workspaces;

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: <Widget>[
        _Header(
          count: workspaces.length,
          busy: connection.busy,
          onRefresh: connection.refreshWorkspaces,
        ),
        Expanded(
          child: workspaces.isEmpty
              ? _EmptyHint(
                  text: connection.busy
                      ? '正在读取工作区…'
                      : 'kb_core 里还没有工作区。\n在服务端建好之后点右上角刷新。',
                )
              : ListView.builder(
                  padding: const EdgeInsets.only(bottom: 16),
                  itemCount: workspaces.length,
                  itemBuilder: (BuildContext context, int index) {
                    final WorkspaceView workspace = workspaces[index];
                    return _WorkspaceTile(
                      workspace: workspace,
                      expanded: !_collapsed.contains(workspace.id),
                      connection: connection,
                      onToggleExpanded: () => setState(() {
                        if (!_collapsed.remove(workspace.id)) {
                          _collapsed.add(workspace.id);
                        }
                      }),
                    );
                  },
                ),
        ),
      ],
    );
  }
}

/// 区块标题：「工作区」+ 计数 + 刷新。
class _Header extends StatelessWidget {
  const _Header({
    required this.count,
    required this.busy,
    required this.onRefresh,
  });

  final int count;
  final bool busy;
  final Future<void> Function() onRefresh;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    return SizedBox(
      height: 36,
      child: Row(
        children: <Widget>[
          const SizedBox(width: 8),
          Text(
            '工作区',
            style: DswTypography.caption.copyWith(
              color: c.labelTertiary,
              fontWeight: FontWeight.w500,
            ),
          ),
          const SizedBox(width: 6),
          Text(
            '$count',
            style: DswTypography.caption.copyWith(color: c.labelCaption),
          ),
          const Spacer(),
          DswIconButton(
            tooltip: '刷新工作区',
            onPressed: busy ? null : () => onRefresh(),
            size: 28,
            iconSize: 16,
            icon: Icons.refresh,
          ),
        ],
      ),
    );
  }
}

/// 一个工作区及其会话。
///
/// 做成有状态是因为"展开着的工作区应当在第一次渲染后就把会话拉下来"——
/// 在 build 里发请求不行，所以放到 `initState` / `didUpdateWidget` 里。
class _WorkspaceTile extends StatefulWidget {
  const _WorkspaceTile({
    required this.workspace,
    required this.expanded,
    required this.connection,
    required this.onToggleExpanded,
  });

  final WorkspaceView workspace;
  final bool expanded;
  final ConnectionController connection;
  final VoidCallback onToggleExpanded;

  @override
  State<_WorkspaceTile> createState() => _WorkspaceTileState();
}

class _WorkspaceTileState extends State<_WorkspaceTile> {
  @override
  void initState() {
    super.initState();
    if (widget.expanded) {
      _loadSessions_();
    }
  }

  @override
  void didUpdateWidget(_WorkspaceTile oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (widget.expanded &&
        (!oldWidget.expanded || oldWidget.workspace.id != widget.workspace.id)) {
      _loadSessions_();
    }
  }

  /// 拉会话。
  ///
  /// 必须等到这一帧画完：`loadSessions` 会立刻 `notifyListeners()`，而在
  /// `initState` / `didUpdateWidget` 期间通知会让正在构建的组件被标脏
  /// （`setState() or markNeedsBuild() called during build`）。
  void _loadSessions_() {
    final String workspaceId = widget.workspace.id;
    final ConnectionController connection = widget.connection;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) {
        connection.loadSessions(workspaceId);
      }
    });
  }

  @override
  Widget build(BuildContext context) {
    final WorkspaceView workspace = widget.workspace;
    final ConnectionController connection = widget.connection;
    final bool selected = connection.selectedWorkspaceId == workspace.id;
    final List<SessionView> sessions = connection.sessionsOf(workspace.id);
    final bool loading = connection.isLoadingSessions(workspace.id);

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: <Widget>[
        HoverBuilder(
          builder: (BuildContext context, bool hovered) {
            return DswListRow(
              height: 34,
              selected: selected || hovered,
              onTap: () => connection.selectWorkspace(workspace.id),
              onLeadingTap: widget.onToggleExpanded,
              leading: hovered
                  ? TriangleRightIcon(
                      expanded: widget.expanded,
                      size: 12,
                      color: context.dsw.labelTertiary,
                    )
                  : Icon(
                      widget.expanded
                          ? Icons.folder_open_outlined
                          : Icons.folder_outlined,
                      size: 16,
                      color: context.dsw.labelTertiary,
                    ),
              title: workspace.name,
              trailing: hovered
                  ? <Widget>[
                      Tooltip(
                        message: workspace.path,
                        child: Icon(
                          Icons.info_outline,
                          size: 14,
                          color: context.dsw.labelTertiary,
                        ),
                      ),
                    ]
                  : const <Widget>[],
            );
          },
        ),
        if (widget.expanded)
          if (loading && sessions.isEmpty)
            const Padding(
              padding: EdgeInsets.only(left: 24, top: 6, bottom: 6),
              child: _Caption('正在读取会话…'),
            )
          else if (sessions.isEmpty)
            const Padding(
              padding: EdgeInsets.only(left: 24, top: 6, bottom: 6),
              child: _Caption('还没有会话'),
            )
          else
            for (final SessionView session in sessions)
              _SessionRow(
                session: session,
                selected: selected,
                onTap: () => connection.selectWorkspace(workspace.id),
              ),
      ],
    );
  }
}

/// 一条会话。
class _SessionRow extends StatelessWidget {
  const _SessionRow({
    required this.session,
    required this.selected,
    required this.onTap,
  });

  final SessionView session;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    final DateTime updated = DateTime.fromMillisecondsSinceEpoch(
      session.updatedAtMillis.toInt(),
    );

    return HoverBuilder(
      builder: (BuildContext context, bool hovered) {
        return Padding(
          padding: const EdgeInsets.only(left: 18),
          child: DswListRow(
            height: 32,
            selected: selected || hovered,
            onTap: onTap,
            leading: Container(
              width: 6,
              height: 6,
              decoration: BoxDecoration(
                color: session.turnCount == 0
                    ? c.labelCaption
                    : c.stateBusinessPrimary,
                shape: BoxShape.circle,
              ),
            ),
            title: session.title,
            trailing: <Widget>[
              Text(
                session.turnCount == 0
                    ? formatSessionTime(updated)
                    : '${session.turnCount} 条 · ${formatSessionTime(updated)}',
                style: DswTypography.caption.copyWith(color: c.labelTertiary),
              ),
            ],
          ),
        );
      },
    );
  }
}

/// 一行次要说明文字。
class _Caption extends StatelessWidget {
  const _Caption(this.text);

  final String text;

  @override
  Widget build(BuildContext context) {
    return Text(
      text,
      style: DswTypography.caption.copyWith(
        fontSize: 12,
        color: context.dsw.labelCaption,
      ),
    );
  }
}

/// 空列表提示。
class _EmptyHint extends StatelessWidget {
  const _EmptyHint({required this.text});

  final String text;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(8, 4, 8, 0),
      child: Text(
        text,
        style: DswTypography.caption.copyWith(
          fontSize: 13,
          color: context.dsw.labelTertiary,
          height: 1.6,
        ),
      ),
    );
  }
}
