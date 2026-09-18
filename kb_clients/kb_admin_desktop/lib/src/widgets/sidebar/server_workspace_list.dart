// 服务端（`kb_core`）的工作区 → 会话列表。
//
// 与本地版 [WorkspaceList] 长得一样，但**数据来自 `kb_core`**，增删也发生在
// 服务端：
//
// - 展开一个工作区时才去拉它的会话（`list_sessions`），拉过就缓存；
// - 增删：新建 / 删除工作区、在某个工作区里新建 / 删除会话——
//   目录 `path` 是 **`kb_core` 进程所在主机**上的路径，客户端不碰自己的文件系统；
// - 顶部有刷新与「新建工作区」两个按钮。
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
          onAddWorkspace: _promptAddWorkspace_,
        ),
        Expanded(
          child: workspaces.isEmpty
              ? _EmptyHint(
                  text: connection.busy
                      ? '正在读取工作区…'
                      : 'kb_core 里还没有工作区。\n点右上角的 + 新建一个（目录在服务端主机上）。',
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
                      onAddSession: () => _addSession_(workspace),
                      onRemoveWorkspace: () => _removeWorkspace_(workspace),
                      onRemoveSession: (SessionView session) =>
                          _removeSession_(workspace, session),
                    );
                  },
                ),
        ),
      ],
    );
  }

  // ── 增删 ────────────────────────────────────────────────────────────

  /// 弹「新建工作区」对话框，确认后提交给服务端。
  Future<void> _promptAddWorkspace_() async {
    final _WorkspaceDraft? draft = await showDialog<_WorkspaceDraft>(
      context: context,
      builder: (BuildContext context) => const _AddWorkspaceDialog(),
    );
    if (draft == null) {
      return;
    }
    final String error = await widget.connection.addWorkspace(
      name: draft.name,
      path: draft.path,
    );
    _report_(error);
  }

  /// 删除一个工作区（先确认：级联删除它的会话）。
  Future<void> _removeWorkspace_(WorkspaceView workspace) async {
    final bool? confirmed = await showDialog<bool>(
      context: context,
      builder: (BuildContext context) => _ConfirmDialog(
        title: '删除工作区',
        message: '将删除「${workspace.name}」及其名下的全部会话。\n'
            '这个操作在 kb_core 所在的主机上执行，无法撤销。',
        confirmLabel: '删除',
      ),
    );
    if (confirmed != true) {
      return;
    }
    final String error = await widget.connection.removeWorkspace(workspace.id);
    _report_(error);
  }

  /// 在某个工作区里新建一个会话。
  Future<void> _addSession_(WorkspaceView workspace) async {
    final String error = await widget.connection.addSession(workspace.id);
    _report_(error);
  }

  /// 删除一个会话。
  Future<void> _removeSession_(
    WorkspaceView workspace,
    SessionView session,
  ) async {
    final String error = await widget.connection.removeSession(
      workspace.id,
      session.id,
    );
    _report_(error);
  }

  /// 把失败说明弹出来（成功时保持安静：列表本身会变）。
  ///
  /// 调用点都在 `await` 之后，所以先查 `mounted` 再用 `context`。
  void _report_(String error) {
    if (error.isEmpty || !mounted) {
      return;
    }
    ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(error)));
  }
}

/// 区块标题：「工作区」+ 计数 + 新建 + 刷新。
class _Header extends StatelessWidget {
  const _Header({
    required this.count,
    required this.busy,
    required this.onRefresh,
    required this.onAddWorkspace,
  });

  final int count;
  final bool busy;
  final Future<void> Function() onRefresh;
  final VoidCallback onAddWorkspace;

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
            tooltip: '新建工作区',
            onPressed: busy ? null : onAddWorkspace,
            size: 28,
            iconSize: 16,
            icon: Icons.add,
          ),
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
    required this.onAddSession,
    required this.onRemoveWorkspace,
    required this.onRemoveSession,
  });

  final WorkspaceView workspace;
  final bool expanded;
  final ConnectionController connection;
  final VoidCallback onToggleExpanded;
  final VoidCallback onAddSession;
  final VoidCallback onRemoveWorkspace;
  final void Function(SessionView session) onRemoveSession;

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
                      DswIconButton(
                        tooltip: '在此工作区新建会话',
                        size: 24,
                        iconSize: 16,
                        icon: Icons.add,
                        onPressed: widget.onAddSession,
                      ),
                      DswIconButton(
                        tooltip: '删除工作区',
                        size: 24,
                        iconSize: 16,
                        icon: Icons.delete_outline,
                        onPressed: widget.onRemoveWorkspace,
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
              child: _Caption('还没有会话，点工作区行上的 + 新建'),
            )
          else
            for (final SessionView session in sessions)
              _SessionRow(
                session: session,
                selected: connection.selectedSessionId == session.id,
                onTap: () => connection.selectSession(workspace.id, session.id),
                onDelete: () => widget.onRemoveSession(session),
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
    required this.onDelete,
  });

  final SessionView session;
  final bool selected;
  final VoidCallback onTap;
  final VoidCallback onDelete;

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
            trailing: hovered
                ? <Widget>[
                    DswIconButton(
                      tooltip: '删除会话',
                      size: 24,
                      iconSize: 16,
                      icon: Icons.delete_outline,
                      onPressed: onDelete,
                    ),
                  ]
                : <Widget>[
                    Text(
                      session.turnCount == 0
                          ? formatSessionTime(updated)
                          : '${session.turnCount} 条 · ${formatSessionTime(updated)}',
                      style: DswTypography.caption.copyWith(
                        color: c.labelTertiary,
                      ),
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

/// 通用确认对话框；点「确认」返回 `true`。
class _ConfirmDialog extends StatelessWidget {
  const _ConfirmDialog({
    required this.title,
    required this.message,
    required this.confirmLabel,
  });

  final String title;
  final String message;
  final String confirmLabel;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    return AlertDialog(
      backgroundColor: c.bgLayer2,
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(20)),
      title: Text(
        title,
        style: DswTypography.body.copyWith(
          fontSize: 16,
          fontWeight: FontWeight.w600,
          color: c.labelPrimary,
        ),
      ),
      content: Text(
        message,
        style: DswTypography.caption.copyWith(
          fontSize: 13,
          color: c.labelSecondary,
          height: 1.6,
        ),
      ),
      actions: <Widget>[
        DswGhostButton(
          label: '取消',
          onPressed: () => Navigator.of(context).pop(false),
        ),
        DswPrimaryButton(
          label: confirmLabel,
          onPressed: () => Navigator.of(context).pop(true),
        ),
      ],
    );
  }
}

/// 新建工作区对话框的返回值。
class _WorkspaceDraft {
  const _WorkspaceDraft({required this.name, required this.path});

  final String name;
  final String path;
}

/// 新建工作区对话框。
///
/// **目录是 `kb_core` 所在主机上的路径**——客户端只把它原样提交给服务端，
/// 不会在本地做任何文件系统操作，所以文案里必须说清楚这一点。
class _AddWorkspaceDialog extends StatefulWidget {
  const _AddWorkspaceDialog();

  @override
  State<_AddWorkspaceDialog> createState() => _AddWorkspaceDialogState();
}

class _AddWorkspaceDialogState extends State<_AddWorkspaceDialog> {
  late final TextEditingController _name = TextEditingController();
  late final TextEditingController _path = TextEditingController();

  @override
  void dispose() {
    _name.dispose();
    _path.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    return Dialog(
      backgroundColor: c.bgLayer2,
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(20)),
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 420),
        child: Padding(
          padding: const EdgeInsets.all(20),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: <Widget>[
              Text(
                '新建工作区',
                style: DswTypography.body.copyWith(
                  fontSize: 16,
                  fontWeight: FontWeight.w600,
                  color: c.labelPrimary,
                ),
              ),
              const SizedBox(height: 6),
              Text(
                '工作区由 kb_core 保管：下面的目录是 kb_core 所在主机上的路径。',
                style: DswTypography.caption.copyWith(color: c.labelTertiary),
              ),
              const SizedBox(height: 16),
              DswLabeledField(
                label: '名称',
                controller: _name,
                hint: '例如：llm_kb 笔记',
              ),
              const SizedBox(height: 10),
              DswLabeledField(
                label: '目录',
                controller: _path,
                hint: '/home/me/notes',
              ),
              const SizedBox(height: 20),
              Row(
                mainAxisAlignment: MainAxisAlignment.end,
                children: <Widget>[
                  DswGhostButton(
                    label: '取消',
                    onPressed: () => Navigator.of(context).pop(),
                  ),
                  const SizedBox(width: 8),
                  DswPrimaryButton(
                    label: '创建',
                    onPressed: () {
                      final String name = _name.text.trim();
                      final String path = _path.text.trim();
                      Navigator.of(context).pop(
                        _WorkspaceDraft(
                          name: name.isEmpty ? '未命名工作区' : name,
                          path: path,
                        ),
                      );
                    },
                  ),
                ],
              ),
            ],
          ),
        ),
      ),
    );
  }
}
