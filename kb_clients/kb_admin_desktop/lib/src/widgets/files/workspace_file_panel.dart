// 右侧「工作区文件」面板。
//
// 对齐 DSH 的 `ui-sidebar-files` `FilesBody`：
//
// ```text
// ┌────────────────────────────────┐
// │ /home/me/notes            [⟳] │  ← 38px 的头部，底部 0.5px 分隔线
// ├────────────────────────────────┤
// │  目录为空                       │  ← 13px / 1.5 的正文
// └────────────────────────────────┘
// ```
//
// **默认隐藏**，由对话区列头右上角的按钮调出（见 `app_shell.dart` 的滑动动画）。
// 文件树本身暂时是空的：`kb_core` 还没有目录浏览接口（见 dev-notes.md §1），
// 这里只把面板的骨架、路径回显与空状态先立起来。

import 'package:flutter/material.dart';

import '../../models/workspace.dart';
import '../../state/app_controller.dart';
import '../../theme/app_theme.dart';
import '../../theme/dsw_tokens.dart';
import '../common/dsw_controls.dart';

/// 工作区文件浏览器。
class WorkspaceFilePanel extends StatelessWidget {
  /// 构造文件面板。
  const WorkspaceFilePanel({
    super.key,
    required this.controller,
    required this.onClose,
  });

  /// 应用状态。
  final AppController controller;

  /// 收起面板。
  final VoidCallback onClose;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    final Workspace? workspace = controller.activeWorkspace;

    return DecoratedBox(
      decoration: BoxDecoration(
        color: c.bgBase,
        border: Border(left: BorderSide(color: c.borderL4, width: 0.5)),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: <Widget>[
          _PanelHeader(path: workspace?.path ?? '', onClose: onClose),
          Expanded(
            child: workspace == null
                ? const _PanelHint(text: '尚未选择工作区。')
                : const _PanelHint(
                    text: '目录为空。\n\n文件浏览器将在 kb_core 提供目录接口后接入，'
                        '这里先保留空状态。',
                  ),
          ),
        ],
      ),
    );
  }
}

/// 面板头部：工作区根目录 + 收起按钮。
class _PanelHeader extends StatelessWidget {
  const _PanelHeader({required this.path, required this.onClose});

  final String path;
  final VoidCallback onClose;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    return Container(
      height: 38,
      padding: const EdgeInsets.only(left: 16, right: 6),
      decoration: BoxDecoration(
        border: Border(bottom: BorderSide(color: c.borderL3, width: 0.5)),
      ),
      child: Row(
        children: <Widget>[
          Expanded(child: _PathLabel(path: path)),
          DswIconButton(
            tooltip: '收起文件面板',
            onPressed: onClose,
            size: 28,
            iconSize: 16,
            icon: Icons.chevron_right,
          ),
        ],
      ),
    );
  }
}

/// 路径回显：中间层级用三级文字色，最后一段用主文字色（DSH 的同一个处理）。
///
/// 太长时只压缩前面的目录部分，最后一段永远完整可见。
class _PathLabel extends StatelessWidget {
  const _PathLabel({required this.path});

  final String path;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    if (path.isEmpty) {
      return Text(
        '未绑定目录',
        style: DswTypography.caption.copyWith(color: c.labelTertiary),
      );
    }

    final int separator = path.lastIndexOf('/');
    final String parent = separator < 0 ? '' : path.substring(0, separator + 1);
    final String name = separator < 0 ? path : path.substring(separator + 1);

    return Row(
      children: <Widget>[
        if (parent.isNotEmpty)
          Flexible(
            child: Text(
              parent,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              textDirection: TextDirection.rtl,
              style: DswTypography.caption.copyWith(color: c.labelTertiary),
            ),
          ),
        Text(
          name,
          maxLines: 1,
          style: DswTypography.caption.copyWith(color: c.labelPrimary),
        ),
      ],
    );
  }
}

/// 面板正文里的说明文字。
class _PanelHint extends StatelessWidget {
  const _PanelHint({required this.text});

  final String text;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 12, 16, 12),
      child: Text(
        text,
        style: DswTypography.body.copyWith(
          fontSize: 13,
          height: 1.5,
          color: c.labelTertiary,
        ),
      ),
    );
  }
}
