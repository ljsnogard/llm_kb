// 左侧栏。
//
// 对齐 DSH `ui-sidebar/src/client/SidebarRoot`：
//
// - 展开态：`padding: 6px 12px`，内容宽度固定为用户的宽度偏好；
// - 折叠态（56px 图标轨道）：`padding: 18px 10px 6px`，控件 36×36；
// - 折叠/展开是「滑动 + 交叉淡入淡出」：内容保持展开时的宽度原地淡出，
//   由滑动的列把它裁掉（所以中途不会重排）；中途之后轨道内容再淡入。
//
// ```text
// ┌ 展开 ────────────────┐   ┌ 折叠 ─┐
// │ llm_kb        [◧]   │   │  [◧] │
// │ [  新会话        ]   │   │  [+] │
// │ 工作区               │   │      │
// │  ▾ 工作区 A          │   │      │
// │     会话 1           │   │      │
// │     会话 2           │   │      │
// │ [⚙ 设置]            │   │  [⚙] │
// └──────────────────────┘   └──────┘
// ```

import 'package:flutter/material.dart';

import '../../state/app_controller.dart';
import '../../theme/app_theme.dart';
import '../../theme/dsw_tokens.dart';
import '../common/dsw_controls.dart';
import '../common/dsw_icons.dart';
import '../settings/settings_dialog.dart';
import 'workspace_list.dart';

/// 左侧栏内容。
///
/// 外层（[AppShell]）负责给出动画中的列宽；这里只负责在「轨道」与「展开」
/// 两种形态之间交叉淡化。
class SidebarPanel extends StatelessWidget {
  /// 构造左侧栏。
  const SidebarPanel({
    super.key,
    required this.animation,
    required this.expandedWidth,
    required this.controller,
  });

  /// 折叠进度：0 表示完全折叠成轨道，1 表示完全展开。
  final double animation;

  /// 展开态的宽度偏好（内容按这个宽度排版，再由外层裁切）。
  final double expandedWidth;

  /// 应用状态。
  final AppController controller;

  /// 展开内容的 key（测试用来读取交叉淡化中的不透明度）。
  static const Key wideKey = ValueKey<String>('kb.sidebar.wide');

  /// 折叠轨道内容的 key。
  static const Key railKey = ValueKey<String>('kb.sidebar.rail');

  /// 展开内容的淡入淡出区间。
  ///
  /// DSH 是「先淡出 150ms、再淡入 150ms」；这里让两段略有重叠，避免中间出现
  /// 一帧全空的闪烁，观感更接近实机。
  static const double _wideFadeStart = 0.35;
  static const double _railFadeEnd = 0.65;

  double get _wideOpacity =>
      ((animation - _wideFadeStart) / (1 - _wideFadeStart)).clamp(0.0, 1.0);

  double get _railOpacity =>
      ((_railFadeEnd - animation) / _railFadeEnd).clamp(0.0, 1.0);

  @override
  Widget build(BuildContext context) {
    return Stack(
      fit: StackFit.expand,
      children: <Widget>[
        OverflowBox(
          alignment: Alignment.centerLeft,
          minWidth: expandedWidth,
          maxWidth: expandedWidth,
          child: Opacity(
            key: wideKey,
            opacity: _wideOpacity,
            child: IgnorePointer(
              ignoring: _wideOpacity < 0.5,
              child: _SidebarWide(controller: controller),
            ),
          ),
        ),
        Align(
          alignment: Alignment.centerLeft,
          child: SizedBox(
            width: DswLayout.sidebarCollapsed,
            child: Opacity(
              key: railKey,
              opacity: _railOpacity,
              child: IgnorePointer(
                ignoring: _railOpacity < 0.5,
                child: _SidebarRail(controller: controller),
              ),
            ),
          ),
        ),
      ],
    );
  }
}

/// 展开态的侧边栏内容。
class _SidebarWide extends StatelessWidget {
  const _SidebarWide({required this.controller});

  final AppController controller;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(
        horizontal: DswLayout.sidebarInlinePadding,
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: <Widget>[
          _BrandRow(controller: controller),
          _NewSessionButton(onPressed: controller.startSession),
          Expanded(child: WorkspaceList(controller: controller)),
          const SizedBox(height: 4),
          _SettingsTrigger(
            onPressed: () => showSettingsDialog(context, controller),
          ),
          const SizedBox(height: 6),
        ],
      ),
    );
  }
}

/// 折叠态的图标轨道（56px）。
class _SidebarRail extends StatelessWidget {
  const _SidebarRail({required this.controller});

  final AppController controller;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(10, 18, 10, 6),
      child: Column(
        children: <Widget>[
          _PanelToggleButton(
            collapsed: true,
            onPressed: () => _toggle(context),
          ),
          const SizedBox(height: 12),
          _NewSessionButton(
            collapsed: true,
            onPressed: controller.startSession,
          ),
          const Spacer(),
          _SettingsTrigger(
            collapsed: true,
            onPressed: () => showSettingsDialog(context, controller),
          ),
        ],
      ),
    );
  }

  void _toggle(BuildContext context) {
    controller.toggleSidebar(MediaQuery.sizeOf(context).width);
  }
}

/// 品牌行：展开时是「圆点 + llm_kb」加右侧的折叠按钮。
class _BrandRow extends StatelessWidget {
  const _BrandRow({required this.controller});

  final AppController controller;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    return SizedBox(
      height: 60,
      child: Row(
        children: <Widget>[
          Container(
            width: 10,
            height: 10,
            decoration: BoxDecoration(
              color: c.stateBusinessPrimary,
              shape: BoxShape.circle,
            ),
          ),
          const SizedBox(width: 8),
          Expanded(
            child: Text(
              'llm_kb',
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: DswTypography.body.copyWith(
                color: c.brandText,
                fontSize: 18,
                fontWeight: FontWeight.w600,
                letterSpacing: 0.4,
              ),
            ),
          ),
          _PanelToggleButton(
            collapsed: false,
            onPressed: () =>
                controller.toggleSidebar(MediaQuery.sizeOf(context).width),
          ),
        ],
      ),
    );
  }
}

/// 「新会话」按钮：38px 高、12px 圆角、浮起底色。
class _NewSessionButton extends StatelessWidget {
  const _NewSessionButton({
    required this.onPressed,
    this.collapsed = false,
  });

  final VoidCallback onPressed;
  final bool collapsed;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    return Tooltip(
      message: '新会话',
      child: HoverBuilder(
        builder: (BuildContext context, bool hovered) {
          return GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTap: onPressed,
            child: AnimatedContainer(
              duration: DswMotion.fast,
              curve: DswMotion.easeInOut,
              height: collapsed ? DswLayout.railControlSize : 38,
              width: collapsed ? DswLayout.railControlSize : null,
              margin: collapsed
                  ? const EdgeInsets.only(bottom: 12)
                  : const EdgeInsets.only(left: 2, right: 2, bottom: 8),
              decoration: BoxDecoration(
                color: hovered
                    ? (collapsed ? c.interactiveHover : c.buttonFloatingHover)
                    : (collapsed ? Colors.transparent : c.buttonElevatedFill),
                border: Border.all(
                  color: collapsed ? Colors.transparent : c.borderL3,
                  width: 0.5,
                ),
                borderRadius: BorderRadius.circular(12),
              ),
              child: Row(
                mainAxisAlignment: MainAxisAlignment.center,
                mainAxisSize: MainAxisSize.min,
                children: <Widget>[
                  Icon(
                    Icons.add_comment_outlined,
                    size: collapsed ? 18 : 14,
                    color: c.labelPrimary,
                  ),
                  if (!collapsed) ...<Widget>[
                    const SizedBox(width: 6),
                    Text(
                      '新会话',
                      style: DswTypography.body.copyWith(
                        color: c.labelPrimary,
                        fontWeight: FontWeight.w500,
                      ),
                    ),
                  ],
                ],
              ),
            ),
          );
        },
      ),
    );
  }
}

/// 折叠 / 展开侧边栏的按钮。
class _PanelToggleButton extends StatelessWidget {
  const _PanelToggleButton({required this.collapsed, required this.onPressed});

  final bool collapsed;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    final String tooltip = collapsed ? '展开侧边栏' : '折叠侧边栏';
    return DswIconButton(
      tooltip: tooltip,
      onPressed: onPressed,
      size: collapsed ? DswLayout.railControlSize : 28,
      borderRadius: BorderRadius.circular(collapsed ? 18 : 14),
      glyph: PanelLeftIcon(
        size: collapsed ? 18 : 16,
        color: collapsed ? c.labelPrimary : c.labelSecondary,
      ),
    );
  }
}

/// 侧边栏底部的设置入口。
///
/// 用户的要求是「工作区列表左下角提供一个设置按钮」，对应 DSH 的
/// `sidebar.settings` 座席：42px 高、12px 圆角、悬停换底色。
class _SettingsTrigger extends StatelessWidget {
  const _SettingsTrigger({
    required this.onPressed,
    this.collapsed = false,
  });

  final VoidCallback onPressed;
  final bool collapsed;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    if (collapsed) {
      return DswIconButton(
        tooltip: '设置',
        onPressed: onPressed,
        size: DswLayout.railControlSize,
        borderRadius: BorderRadius.circular(18),
        glyph: Icon(Icons.settings_outlined, size: 18, color: c.labelPrimary),
      );
    }

    return HoverBuilder(
      builder: (BuildContext context, bool hovered) {
        return Semantics(
          button: true,
          label: '设置',
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTap: onPressed,
            child: AnimatedContainer(
              duration: DswMotion.fast,
              curve: DswMotion.easeInOut,
              height: 42,
              padding: const EdgeInsets.only(left: 8, right: 10),
              decoration: BoxDecoration(
                color: hovered ? c.interactiveHover : Colors.transparent,
                borderRadius: BorderRadius.circular(12),
              ),
              child: Row(
                children: <Widget>[
                  Icon(Icons.settings_outlined, size: 16, color: c.labelPrimary),
                  const SizedBox(width: 8),
                  Text(
                    '设置',
                    style: DswTypography.body.copyWith(color: c.labelPrimary),
                  ),
                ],
              ),
            ),
          ),
        );
      },
    );
  }
}
