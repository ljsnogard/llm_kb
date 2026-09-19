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
// │ ● 主机名 ▾     [◧]   │   │  [◧] │
// │   已连接 kb_core …   │   │  [+] │
// │ [  新会话        ]   │   │  [◈] │
// │ 工作区               │   │      │
// │  ▾ 工作区 A          │   │      │
// │     会话 1           │   │      │
// │     会话 2           │   │      │
// │ [⚙ 设置]            │   │  [⚙] │
// └──────────────────────┘   └──────┘
// ```
//
// 左上角那个按钮是**主机名（花名）+ 切换 `kb_core` 的入口**：名字来自客户端
// 自己的连接配置（`kb_client_config` 的 `Connection::name`），与握手协议无关。
// 决策见 `dev-notes/kb_admin_desktop-20260918-1712.md` §2。

import 'package:flutter/material.dart';

import '../../services/kb_client_api.dart';
import '../../state/app_controller.dart';
import '../../state/connection_controller.dart';
import '../../theme/app_theme.dart';
import '../../theme/dsw_tokens.dart';
import '../common/dsw_controls.dart';
import '../common/dsw_icons.dart';
import '../connection/connection_dialog.dart';
import '../settings/settings_dialog.dart';
import 'server_workspace_list.dart';
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
    this.connection,
  });

  /// 折叠进度：0 表示完全折叠成轨道，1 表示完全展开。
  final double animation;

  /// 展开态的宽度偏好（内容按这个宽度排版，再由外层裁切）。
  final double expandedWidth;

  /// 应用状态。
  final AppController controller;

  /// 与 `kb_core` 的连接状态；为 `null` 时侧边栏退化成纯本地模式
  /// （widget 测试就是这么用的，不需要初始化原生库）。
  final ConnectionController? connection;

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
              child: _SidebarWide(controller: controller, connection: connection),
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
                child: _SidebarRail(controller: controller, connection: connection),
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
  const _SidebarWide({required this.controller, this.connection});

  final AppController controller;
  final ConnectionController? connection;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(
        horizontal: DswLayout.sidebarInlinePadding,
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: <Widget>[
          if (connection == null)
            _BrandRow(controller: controller)
          else
            _HostRow(controller: controller, connection: connection!),
          _NewSessionButton(
            onPressed: _newSessionAction_(context, controller, connection),
          ),
          // 连接是异步建立的（`initialize()` 在 `runApp` 之后才跑完），所以这一块
          // 必须跟着连接状态重建——否则"连上之后列表换成服务端数据"永远不发生。
          Expanded(
            child: connection == null
                ? WorkspaceList(controller: controller)
                : AnimatedBuilder(
                    animation: connection!,
                    builder: (BuildContext context, Widget? _) {
                      return connection!.connected
                          ? ServerWorkspaceList(connection: connection!)
                          : WorkspaceList(controller: controller);
                    },
                  ),
          ),
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
  const _SidebarRail({required this.controller, this.connection});

  final AppController controller;
  final ConnectionController? connection;

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
            onPressed: _newSessionAction_(context, controller, connection),
          ),
          if (connection != null)
            _HostRailButton(connection: connection!),
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

/// 左上角的「当前主机」按钮。
///
/// - **标题**是当前连接配置里的名字（客户端自己起的花名），与握手协议无关：
///   `kb_core` 那边不知道自己被叫什么；
/// - 点开是一个菜单：列出所有已配置的连接方式，选中哪一条就连它（旧的连接被
///   替换掉，`local-launch` 起的子进程也随之结束）；最后一项是「管理连接方式…」，
///   打开原来的连接对话框。
class _HostRow extends StatelessWidget {
  const _HostRow({required this.controller, required this.connection});

  final AppController controller;
  final ConnectionController connection;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    return AnimatedBuilder(
      animation: connection,
      builder: (BuildContext context, Widget? _) {
        final (Color dot, String title, String subtitle, String tooltip) =
            _hostStatus_(connection, context);
        final Widget body = Padding(
          padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 6),
          child: Row(
            children: <Widget>[
              Container(
                width: 8,
                height: 8,
                decoration: BoxDecoration(color: dot, shape: BoxShape.circle),
              ),
              const SizedBox(width: 8),
              Expanded(
                child: Column(
                  mainAxisAlignment: MainAxisAlignment.center,
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: <Widget>[
                    Text(
                      title,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: DswTypography.body.copyWith(
                        color: c.brandText,
                        fontSize: 15,
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                    Text(
                      subtitle,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: DswTypography.caption.copyWith(
                        fontSize: 11,
                        color: c.labelTertiary,
                      ),
                    ),
                  ],
                ),
              ),
              Icon(Icons.unfold_more, size: 14, color: c.labelTertiary),
            ],
          ),
        );

        return SizedBox(
          height: 60,
          child: Row(
            children: <Widget>[
              Expanded(
                child: Tooltip(
                  message: tooltip,
                  child: _HostButton(connection: connection, child: body),
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
      },
    );
  }
}

/// 折叠轨道上的主机入口：一个图标，颜色反映状态。
class _HostRailButton extends StatelessWidget {
  const _HostRailButton({required this.connection});

  final ConnectionController connection;

  @override
  Widget build(BuildContext context) {
    return AnimatedBuilder(
      animation: connection,
      builder: (BuildContext context, Widget? _) {
        final (Color dot, String title, _, String tooltip) = _hostStatus_(
          connection,
          context,
        );
        return Padding(
          padding: const EdgeInsets.only(bottom: 12),
          child: Tooltip(
            message: '$title（$tooltip）',
            child: _HostButton(
              connection: connection,
              child: SizedBox(
                width: DswLayout.railControlSize,
                height: DswLayout.railControlSize,
                child: Center(
                  child: Icon(Icons.dns_outlined, size: 18, color: dot),
                ),
              ),
            ),
          ),
        );
      },
    );
  }
}

/// 主机按钮的交互壳：有已配置项时弹切换菜单，否则直接开连接对话框。
class _HostButton extends StatelessWidget {
  const _HostButton({required this.connection, required this.child});

  final ConnectionController connection;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;

    if (connection.profiles.isEmpty) {
      return GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: () => showConnectionDialog(context, connection),
        child: child,
      );
    }

    return PopupMenuButton<String>(
      tooltip: '',
      color: c.bgLayer2,
      position: PopupMenuPosition.under,
      onSelected: (String value) =>
          _onHostSelected_(context, connection, value),
      itemBuilder: (BuildContext context) => <PopupMenuEntry<String>>[
        for (final ConnectionView profile in connection.profiles)
          PopupMenuItem<String>(
            value: 'profile:${profile.name}',
            child: Row(
              children: <Widget>[
                SizedBox(
                  width: 20,
                  child:
                      connection.connected &&
                          connection.profileName == profile.name
                      ? Icon(Icons.check, size: 14, color: c.stateSuccess)
                      : null,
                ),
                Expanded(
                  child: Text(
                    profile.name,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: DswTypography.body.copyWith(
                      fontSize: 13,
                      color: c.labelPrimary,
                    ),
                  ),
                ),
                const SizedBox(width: 12),
                Text(
                  profile.kind,
                  style: DswTypography.caption.copyWith(
                    fontSize: 11,
                    color: c.labelCaption,
                  ),
                ),
              ],
            ),
          ),
        const PopupMenuDivider(),
        const PopupMenuItem<String>(
          value: 'manage',
          child: Text('管理连接方式…'),
        ),
      ],
      child: child,
    );
  }
}

/// 菜单选择：切换连接，或打开连接对话框。
Future<void> _onHostSelected_(
  BuildContext context,
  ConnectionController connection,
  String value,
) async {
  if (value == 'manage') {
    await showConnectionDialog(context, connection);
    return;
  }

  final String name = value.startsWith('profile:')
      ? value.substring('profile:'.length)
      : '';
  for (final ConnectionView profile in connection.profiles) {
    if (profile.name != name) {
      continue;
    }
    final bool ok = await connection.connect(profile);
    if (!ok && context.mounted) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
          content: Text(connection.error.isEmpty ? '连接失败' : connection.error),
        ),
      );
    }
    return;
  }
}

/// 「新会话」按钮该做什么。
///
/// - **已连接 `kb_core`**：开一个**草稿会话**（只改客户端状态，不发请求）。
///   用户发出第一条消息时才会 `CreateSession`，名字由 `kb_core` 从那个问题推导——
///   于是"空的「新会话」"永远不会落盘。失败时弹一句说明。
/// - **未连接**：退回本地那一套（`AppController`），这样没有服务端也能起界面。
VoidCallback _newSessionAction_(
  BuildContext context,
  AppController controller,
  ConnectionController? connection,
) {
  final ConnectionController? conn = connection;
  if (conn == null || !conn.connected) {
    return controller.startSession;
  }
  return () {
    conn.newSessionInSelectedWorkspace().then((String error) {
      if (error.isNotEmpty && context.mounted) {
        ScaffoldMessenger.of(
          context,
        ).showSnackBar(SnackBar(content: Text(error)));
      }
    });
  };
}

/// 连接状态 → （圆点颜色, 标题, 副标题, 悬停提示）。
///
/// **标题就是"主机名"**——连接配置里用户自己起的花名；未连接时是一句占位文案。
/// 副标题放服务端版本与"本机 / 远程"，让"我现在连的是哪一台"一眼可见。
(Color, String, String, String) _hostStatus_(
  ConnectionController connection,
  BuildContext context,
) {
  final DswColors c = context.dsw;
  switch (connection.phase) {
    case ConnectionPhase.idle:
      return (
        c.labelCaption,
        '选择主机',
        '未连接 kb_core',
        '点击选择要连接的 kb_core',
      );
    case ConnectionPhase.connecting:
      final String name = connection.profileName.isEmpty
          ? '正在连接…'
          : connection.profileName;
      return (c.stateWarn, name, '正在连接 kb_core…', '正在握手');
    case ConnectionPhase.connected:
      final String where = connection.isLocal
          ? (connection.launchedPid > 0
                ? '本机（自起 pid ${connection.launchedPid}）'
                : '本机')
          : '远程';
      return (
        c.stateSuccess,
        connection.profileName,
        'kb_core ${connection.serverVersion} · $where',
        '已连接：$where，协议 v${connection.protocolVersion}',
      );
    case ConnectionPhase.failed:
      return (
        c.stateError,
        connection.profileName.isEmpty ? '连接失败' : connection.profileName,
        '连接失败，点击切换或重试',
        connection.error.isEmpty ? '点击重试' : connection.error,
      );
  }
}
