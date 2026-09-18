// 三栏应用框架。
//
// 结构、尺寸与动效都对齐 DSH v0.1.5-rc 的 `AppFrame`：
//
// ```text
// ┌──────────┬────────────────────────────┬──────────────┐
// │ 侧边栏    │ 对话区                      │ 文件浏览器     │
// │ 工作区列表 │ 消息列表 + 输入框            │ 默认隐藏       │
// │ 底部设置   │                            │              │
// └──────────┴────────────────────────────┴──────────────┘
// ```
//
// 三条关键约定（都能在 DSH 里找到出处）：
//
// 1. **列宽动画 300ms / cubic-bezier(0.4, 0, 0.2, 1)**：`AppFrame.module.css`
//    的 `transition: grid-template-columns var(--ds-transition-duration-slow)
//    var(--ds-ease-in-out)`。
// 2. **拖拽时关闭动画**：缓动的列宽会跟手指脱节（`[data-dragging] { transition: none }`）。
// 3. **右侧面板不裁剪、以固定宽度贴右边缘滑动**：`rightbarCol` 是
//    `overflow: visible`，面板用 `translateX` 滑入，所以滑动过程中面板内容
//    不会重新换行，它的左边缘始终与对话区的右边缘重合。

import 'dart:ui' show lerpDouble;

import 'package:flutter/material.dart';

import '../state/app_controller.dart';
import '../state/connection_controller.dart';
import '../theme/dsw_tokens.dart';
import 'conversation/conversation_pane.dart';
import 'files/workspace_file_panel.dart';
import 'sidebar/sidebar_panel.dart';

/// 三栏框架。
class AppShell extends StatefulWidget {
  /// 构造应用框架。
  const AppShell({super.key, required this.controller, this.connection});

  /// 应用状态。
  final AppController controller;

  /// 与 `kb_core` 的连接状态；为 `null` 时侧边栏退化成纯本地模式。
  final ConnectionController? connection;

  /// 左侧栏轨道的 key（测试用来读取动画中的列宽）。
  static const Key sidebarTrackKey = ValueKey<String>('kb.sidebar.track');

  /// 右侧栏轨道的 key。
  static const Key fileTrackKey = ValueKey<String>('kb.file.track');

  @override
  State<AppShell> createState() => _AppShellState();
}

class _AppShellState extends State<AppShell> {
  /// 拖拽开始时被拖那一列的渲染宽度。
  ///
  /// 拖拽增量始终相对这个冻结点计算，而不是相对「宽度偏好」——否则抓住一条
  /// 已经被压缩过的列时，第一帧就会跳回偏好值（DSH `AppFrame.tsx` L175-191
  /// 的 `sidebarBase` / `rightbarBase` 同理）。同一时刻只会有一处在拖拽，
  /// 因此两条列共用一个字段。
  double _dragBase = 0;

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (BuildContext context, BoxConstraints constraints) {
        final double viewport = constraints.maxWidth;
        final AppController controller = widget.controller;

        final bool sidebarCollapsed = controller.sidebarCollapsedFor(viewport);
        final bool fileShown = controller.filePanelShown;

        final double sidebarPreference = controller.sidebarWidth;
        final double filePreference = controller.filePanelPreferenceFor(viewport);

        // 面板自身的固定宽度：与 DSH 的 `normal.rightbar` 对应，
        // 即使轨道宽度为 0，面板也按这个宽度渲染再整体滑出去。
        final double panelWidth = DswLayout.clampRightbar(
          filePreference,
          viewport,
        );

        return TweenAnimationBuilder<double>(
          tween: Tween<double>(
            begin: sidebarCollapsed ? 0 : 1,
            end: sidebarCollapsed ? 0 : 1,
          ),
          duration: controller.dragging ? Duration.zero : DswMotion.slow,
          curve: DswMotion.easeInOut,
          builder: (BuildContext context, double sidebarT, Widget? _) {
            return TweenAnimationBuilder<double>(
              tween: Tween<double>(
                begin: fileShown ? 1 : 0,
                end: fileShown ? 1 : 0,
              ),
              duration: controller.dragging ? Duration.zero : DswMotion.slow,
              curve: DswMotion.easeInOut,
              builder: (BuildContext context, double fileT, Widget? _) {
                final double sidebarWidth = lerpDouble(
                  DswLayout.sidebarCollapsed,
                  sidebarPreference,
                  sidebarT,
                )!;
                final double fileTrackWidth = fileT * panelWidth;

                return Stack(
                  // 右侧面板以固定宽度悬挂在中心区之上，必须允许溢出绘制
                  // （DSH 的 `.rightbarCol { overflow: visible }`）。
                  clipBehavior: Clip.none,
                  children: <Widget>[
                    Positioned.fill(
                      child: Row(
                        crossAxisAlignment: CrossAxisAlignment.stretch,
                        children: <Widget>[
                          _buildSidebarTrack(sidebarWidth, sidebarT),
                          Expanded(
                            child: ConversationPane(
                              controller: controller,
                              viewportWidth: viewport,
                            ),
                          ),
                          _buildFileTrack(
                            fileTrackWidth: fileTrackWidth,
                            panelWidth: panelWidth,
                            fileT: fileT,
                            viewport: viewport,
                            controller: controller,
                          ),
                        ],
                      ),
                    ),
                    if (!sidebarCollapsed)
                      _buildResizeHandle(
                        left: sidebarWidth - DswLayout.resizeHandleWidth / 2,
                        onStart: () {
                          _dragBase = sidebarWidth;
                          controller.setDragging(true);
                        },
                        onDrag: (double dx) =>
                            controller.setSidebarWidth(_dragBase + dx),
                        onEnd: () => controller.setDragging(false),
                      ),
                    if (fileShown)
                      _buildResizeHandle(
                        left: viewport -
                            fileTrackWidth -
                            DswLayout.resizeHandleWidth / 2,
                        onStart: () {
                          _dragBase = panelWidth;
                          controller.setDragging(true);
                        },
                        onDrag: (double dx) => controller.setFilePanelWidth(
                          _dragBase - dx,
                          viewport,
                        ),
                        onEnd: () => controller.setDragging(false),
                      ),
                  ],
                );
              },
            );
          },
        );
      },
    );
  }

  Widget _buildSidebarTrack(double width, double t) {
    final DswColors c = context.dsw;
    return SizedBox(
      key: AppShell.sidebarTrackKey,
      width: width,
      child: DecoratedBox(
        decoration: BoxDecoration(
          color: c.sidebarFill,
          border: Border(right: BorderSide(color: c.borderL3, width: 0.5)),
        ),
        child: ClipRect(
          child: SidebarPanel(
            animation: t,
            expandedWidth: widget.controller.sidebarWidth,
            controller: widget.controller,
            connection: widget.connection,
          ),
        ),
      ),
    );
  }

  Widget _buildFileTrack({
    required double fileTrackWidth,
    required double panelWidth,
    required double fileT,
    required double viewport,
    required AppController controller,
  }) {
    // 轨道宽度从 0 长到 panelWidth，而面板始终按 panelWidth 渲染、整体贴右
    // 边缘平移，因此滑动过程中面板内容不会重新换行；它的左边缘与对话区的
    // 右边缘严格重合。
    return SizedBox(
      key: AppShell.fileTrackKey,
      width: fileTrackWidth,
      child: OverflowBox(
        alignment: Alignment.centerRight,
        minWidth: panelWidth,
        maxWidth: panelWidth,
        child: Transform.translate(
          offset: Offset((1 - fileT) * panelWidth, 0),
          child: SizedBox(
            width: panelWidth,
            child: IgnorePointer(
              ignoring: fileT < 0.02,
              child: WorkspaceFilePanel(
                controller: controller,
                onClose: () => controller.toggleFilePanel(viewport),
              ),
            ),
          ),
        ),
      ),
    );
  }

  /// 8px 宽的隐形拖拽热区，居中压在列边界上（DSH 的 `.handle`）。
  Widget _buildResizeHandle({
    required double left,
    required VoidCallback onStart,
    required void Function(double dx) onDrag,
    required VoidCallback onEnd,
  }) {
    return Positioned(
      left: left,
      top: 0,
      bottom: 0,
      width: DswLayout.resizeHandleWidth,
      child: _ResizeHandle(
        onStart: onStart,
        onDrag: onDrag,
        onEnd: onEnd,
      ),
    );
  }
}

/// 一条可拖拽的列边界。
class _ResizeHandle extends StatefulWidget {
  const _ResizeHandle({
    required this.onStart,
    required this.onDrag,
    required this.onEnd,
  });

  final VoidCallback onStart;
  final void Function(double dx) onDrag;
  final VoidCallback onEnd;

  @override
  State<_ResizeHandle> createState() => _ResizeHandleState();
}

class _ResizeHandleState extends State<_ResizeHandle> {
  double _origin = 0;

  @override
  Widget build(BuildContext context) {
    return MouseRegion(
      cursor: SystemMouseCursors.resizeColumn,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onHorizontalDragStart: (DragStartDetails details) {
          _origin = details.globalPosition.dx;
          widget.onStart();
        },
        onHorizontalDragUpdate: (DragUpdateDetails details) {
          widget.onDrag(details.globalPosition.dx - _origin);
        },
        onHorizontalDragEnd: (_) => widget.onEnd(),
        onHorizontalDragCancel: widget.onEnd,
      ),
    );
  }
}
