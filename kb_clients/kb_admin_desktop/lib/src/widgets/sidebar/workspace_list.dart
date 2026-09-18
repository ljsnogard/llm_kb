// 工作区列表（左侧栏中间的浏览区）。
//
// 对齐 DSH `ui-workspace` 的 `WorkspaceBrowser`：
//
// - 工作区行 34px 高、8px 圆角；悬停时文件夹图标换成三角箭头，同时右侧浮出
//   行内操作；
// - 会话行 32px 高，缩进一级；
// - **选中态与悬停态用同一种底色**（`interactive-bg-hover`），这是 DSH 的
//   刻意取舍：侧边栏里没有第二套强调色。
//
// 目前没有工作区接口（见 `kb_core`/`dev-notes.md` §1），列表来自客户端本地存储。

import 'package:flutter/material.dart';

import '../../models/chat_session.dart';
import '../../models/workspace.dart';
import '../../state/app_controller.dart';
import '../../theme/app_theme.dart';
import '../../theme/dsw_tokens.dart';
import '../common/dsw_controls.dart';
import '../common/dsw_icons.dart';
import '../common/dsw_list_row.dart';

/// 工作区与会话的两级列表。
class WorkspaceList extends StatefulWidget {
  /// 构造工作区列表。
  const WorkspaceList({super.key, required this.controller});

  /// 应用状态。
  final AppController controller;

  @override
  State<WorkspaceList> createState() => _WorkspaceListState();
}

class _WorkspaceListState extends State<WorkspaceList> {
  /// 被用户手动收起的工作区。
  ///
  /// 用「收起集合」而不是「展开集合」，这样默认状态是所有工作区都展开，
  /// 新加进来的工作区也会自动展开。
  final Set<String> _collapsed = <String>{};

  @override
  Widget build(BuildContext context) {
    final AppController controller = widget.controller;
    final List<Workspace> workspaces = controller.workspaces;

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: <Widget>[
        _RegionHeader(
          onAddWorkspace: () => _promptAddWorkspace(context, controller),
        ),
        Expanded(
          child: workspaces.isEmpty
              ? const _EmptyWorkspaceHint()
              : ListView.builder(
                  padding: const EdgeInsets.only(bottom: 16),
                  itemCount: workspaces.length,
                  itemBuilder: (BuildContext context, int index) {
                    final Workspace workspace = workspaces[index];
                    return _WorkspaceTile(
                      workspace: workspace,
                      expanded: !_collapsed.contains(workspace.id),
                      controller: controller,
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

  Future<void> _promptAddWorkspace(
    BuildContext context,
    AppController controller,
  ) async {
    final _WorkspaceDraft? draft = await showDialog<_WorkspaceDraft>(
      context: context,
      builder: (BuildContext context) => const _AddWorkspaceDialog(),
    );
    if (draft == null) {
      return;
    }
    controller.addWorkspace(name: draft.name, path: draft.path);
    controller.startSession();
  }
}

/// 区块标题行：「工作区」加一个新增按钮。
class _RegionHeader extends StatelessWidget {
  const _RegionHeader({required this.onAddWorkspace});

  final VoidCallback onAddWorkspace;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    return SizedBox(
      height: 36,
      child: Row(
        children: <Widget>[
          Expanded(
            child: Padding(
              padding: const EdgeInsets.only(left: 8),
              child: Text(
                '工作区',
                style: DswTypography.caption.copyWith(
                  color: c.labelTertiary,
                  fontWeight: FontWeight.w500,
                ),
              ),
            ),
          ),
          DswIconButton(
            tooltip: '新增工作区',
            onPressed: onAddWorkspace,
            size: 28,
            iconSize: 16,
            icon: Icons.add,
          ),
        ],
      ),
    );
  }
}

/// 一个工作区及其会话。
class _WorkspaceTile extends StatelessWidget {
  const _WorkspaceTile({
    required this.workspace,
    required this.expanded,
    required this.controller,
    required this.onToggleExpanded,
  });

  final Workspace workspace;
  final bool expanded;
  final AppController controller;
  final VoidCallback onToggleExpanded;

  @override
  Widget build(BuildContext context) {
    final AppController controller = this.controller;
    final bool selected = controller.activeWorkspaceId == workspace.id;

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: <Widget>[
        HoverBuilder(
          builder: (BuildContext context, bool hovered) {
            return DswListRow(
              height: 34,
              selected: selected || hovered,
              onTap: () => controller.selectWorkspace(workspace.id),
              onLeadingTap: onToggleExpanded,
              leading: hovered
                  ? TriangleRightIcon(
                      expanded: expanded,
                      size: 12,
                      color: context.dsw.labelTertiary,
                    )
                  : Icon(
                      expanded
                          ? Icons.folder_open_outlined
                          : Icons.folder_outlined,
                      size: 16,
                      color: context.dsw.labelTertiary,
                    ),
              title: workspace.name,
              trailing: hovered
                  ? <Widget>[
                      DswIconButton(
                        tooltip: '在此工作区新建会话',
                        size: 24,
                        iconSize: 16,
                        icon: Icons.add,
                        onPressed: () {
                          controller.selectWorkspace(workspace.id);
                          controller.startSession();
                        },
                      ),
                      DswIconButton(
                        tooltip: '删除工作区',
                        size: 24,
                        iconSize: 16,
                        icon: Icons.delete_outline,
                        onPressed: () =>
                            controller.removeWorkspace(workspace.id),
                      ),
                    ]
                  : const <Widget>[],
            );
          },
        ),
        if (expanded)
          for (final ChatSession session in workspace.sessions)
            _SessionRow(
              session: session,
              selected:
                  selected && workspace.activeSession?.id == session.id,
              onTap: () {
                controller.selectWorkspace(workspace.id);
                controller.selectSession(session.id);
              },
              onDelete: () {
                controller.selectWorkspace(workspace.id);
                controller.removeSession(session.id);
              },
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

  final ChatSession session;
  final bool selected;
  final VoidCallback onTap;
  final VoidCallback onDelete;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
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
                color: session.turns.isEmpty
                    ? c.labelCaption
                    : c.stateBusinessPrimary,
                shape: BoxShape.circle,
              ),
            ),
            title: session.title,
            trailing: <Widget>[
              if (hovered)
                DswIconButton(
                  tooltip: '删除会话',
                  size: 24,
                  iconSize: 16,
                  icon: Icons.delete_outline,
                  onPressed: onDelete,
                )
              else
                Text(
                  formatSessionTime(session.updatedAt),
                  style: DswTypography.caption.copyWith(color: c.labelTertiary),
                ),
            ],
          ),
        );
      },
    );
  }

}

/// 一个工作区都没有时的提示。
class _EmptyWorkspaceHint extends StatelessWidget {
  const _EmptyWorkspaceHint();

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    return Padding(
      padding: const EdgeInsets.fromLTRB(8, 4, 8, 0),
      child: Text(
        '还没有工作区。\n点右上角的 + 添加一个目录。',
        style: DswTypography.caption.copyWith(
          fontSize: 13,
          color: c.labelTertiary,
          height: 1.6,
        ),
      ),
    );
  }
}

/// 新增工作区对话框的返回值。
class _WorkspaceDraft {
  const _WorkspaceDraft({required this.name, required this.path});

  final String name;
  final String path;
}

/// 新增工作区对话框。
///
/// TODO(下一阶段)：接上服务端工作区接口后，这里应当换成一个目录选择器
/// （`kb_core` 目前还没有工作区概念）。
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
                '新增工作区',
                style: DswTypography.body.copyWith(
                  fontSize: 16,
                  fontWeight: FontWeight.w600,
                  color: c.labelPrimary,
                ),
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
