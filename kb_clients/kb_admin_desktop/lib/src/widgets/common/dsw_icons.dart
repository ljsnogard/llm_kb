// DSH 图标集里本客户端需要的那几个。
//
// DSH 用的是仓库内自绘的 inline SVG（`packages/client/ui-primitives/src/icons/
// index.tsx`），没有第三方图标依赖。这里对其中最有辨识度的两个
// （面板切换、发送箭头）同样用 [CustomPainter] 画出来，其余用 Material 图标近似。

import 'package:flutter/material.dart';

/// DSH 的 `IconPanelLeftOutline16`：一个圆角窗口 + 左侧竖分隔线。
///
/// 左侧栏的折叠按钮与右侧栏的展开按钮共用它，后者用 [mirrored] 镜像，
/// 对应 DSH 的 `transform: scaleX(-1)`。
class PanelLeftIcon extends StatelessWidget {
  /// 构造一个面板图标。
  const PanelLeftIcon({
    super.key,
    this.size = 16,
    this.color,
    this.mirrored = false,
    this.strokeWidth = 1.2,
  });

  /// 图标边长。
  final double size;

  /// 线条颜色；为空时取当前 [IconTheme] / 文本色。
  final Color? color;

  /// 是否水平镜像（用于右侧栏）。
  final bool mirrored;

  /// 线宽。
  final double strokeWidth;

  @override
  Widget build(BuildContext context) {
    final Color resolved =
        color ?? IconTheme.of(context).color ?? const Color(0xFFFAFAFB);
    final Widget painter = CustomPaint(
      size: Size.square(size),
      painter: _PanelLeftPainter(color: resolved, strokeWidth: strokeWidth),
    );
    if (!mirrored) {
      return painter;
    }
    return Transform(
      alignment: Alignment.center,
      transform: Matrix4.identity()..scaleByDouble(-1.0, 1.0, 1.0, 1.0),
      child: painter,
    );
  }
}

class _PanelLeftPainter extends CustomPainter {
  const _PanelLeftPainter({required this.color, required this.strokeWidth});

  final Color color;
  final double strokeWidth;

  @override
  void paint(Canvas canvas, Size size) {
    final double scale = size.width / 16.0;
    final Paint paint = Paint()
      ..color = color
      ..style = PaintingStyle.stroke
      ..strokeWidth = strokeWidth
      ..strokeCap = StrokeCap.round
      ..strokeJoin = StrokeJoin.round;

    final RRect frame = RRect.fromRectAndRadius(
      Rect.fromLTWH(2.0 * scale, 3.0 * scale, 12.0 * scale, 10.0 * scale),
      Radius.circular(2.5 * scale),
    );
    canvas.drawRRect(frame, paint);

    // 左侧分隔线：DSH 的图标里它从窗口顶边贯到底边。
    canvas.drawLine(
      Offset(6.0 * scale, 3.0 * scale),
      Offset(6.0 * scale, 13.0 * scale),
      paint,
    );
  }

  @override
  bool shouldRepaint(_PanelLeftPainter oldDelegate) =>
      oldDelegate.color != color || oldDelegate.strokeWidth != strokeWidth;
}

/// DSH 的发送按钮图形：一个向上的箭头，尾部带一小段竖线。
class SendArrowIcon extends StatelessWidget {
  /// 构造一个发送箭头。
  const SendArrowIcon({super.key, this.size = 16, this.color});

  /// 图标边长。
  final double size;

  /// 线条颜色。
  final Color? color;

  @override
  Widget build(BuildContext context) {
    final Color resolved =
        color ?? IconTheme.of(context).color ?? const Color(0xFFFFFFFF);
    return CustomPaint(
      size: Size.square(size),
      painter: _SendArrowPainter(color: resolved),
    );
  }
}

class _SendArrowPainter extends CustomPainter {
  const _SendArrowPainter({required this.color});

  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    final double scale = size.width / 16.0;
    final Paint paint = Paint()
      ..color = color
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.6 * scale
      ..strokeCap = StrokeCap.round
      ..strokeJoin = StrokeJoin.round;

    final double cx = 8.0 * scale;
    canvas.drawLine(
      Offset(cx, 13.0 * scale),
      Offset(cx, 3.5 * scale),
      paint,
    );
    final Path head = Path()
      ..moveTo(4.0 * scale, 7.5 * scale)
      ..lineTo(cx, 3.5 * scale)
      ..lineTo(12.0 * scale, 7.5 * scale);
    canvas.drawPath(head, paint);
  }

  @override
  bool shouldRepaint(_SendArrowPainter oldDelegate) =>
      oldDelegate.color != color;
}

/// DSH 的「停止」图形：一个圆角方块。
class StopSquareIcon extends StatelessWidget {
  /// 构造一个停止方块。
  const StopSquareIcon({super.key, this.size = 16, this.color});

  /// 图标边长。
  final double size;

  /// 颜色。
  final Color? color;

  @override
  Widget build(BuildContext context) {
    final Color resolved =
        color ?? IconTheme.of(context).color ?? const Color(0xFFFFFFFF);
    return CustomPaint(
      size: Size.square(size),
      painter: _StopSquarePainter(color: resolved),
    );
  }
}

class _StopSquarePainter extends CustomPainter {
  const _StopSquarePainter({required this.color});

  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    final double side = size.width * (10.0 / 16.0);
    final RRect rect = RRect.fromRectAndRadius(
      Rect.fromCenter(
        center: Offset(size.width / 2, size.height / 2),
        width: side,
        height: side,
      ),
      Radius.circular(size.width * (2.0 / 16.0)),
    );
    canvas.drawRRect(rect, Paint()..color = color);
  }

  @override
  bool shouldRepaint(_StopSquarePainter oldDelegate) =>
      oldDelegate.color != color;
}

/// DSH 的目录展开箭头 `IconTriangleRightFill14`：实心三角，展开时旋转 90°。
class TriangleRightIcon extends StatelessWidget {
  /// 构造一个三角箭头。
  const TriangleRightIcon({
    super.key,
    required this.expanded,
    this.size = 12,
    this.color,
  });

  /// 是否展开（展开时指向下方）。
  final bool expanded;

  /// 图标边长。
  final double size;

  /// 颜色。
  final Color? color;

  @override
  Widget build(BuildContext context) {
    final Color resolved =
        color ?? IconTheme.of(context).color ?? const Color(0xFF81858C);
    return AnimatedRotation(
      turns: expanded ? 0.25 : 0,
      duration: const Duration(milliseconds: 150),
      curve: const Cubic(0.4, 0.0, 0.2, 1.0),
      child: CustomPaint(
        size: Size.square(size),
        painter: _TriangleRightPainter(color: resolved),
      ),
    );
  }
}

class _TriangleRightPainter extends CustomPainter {
  const _TriangleRightPainter({required this.color});

  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    final double w = size.width;
    final Path path = Path()
      ..moveTo(w * 0.36, w * 0.24)
      ..lineTo(w * 0.70, w * 0.50)
      ..lineTo(w * 0.36, w * 0.76)
      ..close();
    canvas.drawPath(
      path,
      Paint()
        ..color = color
        ..style = PaintingStyle.fill,
    );
  }

  @override
  bool shouldRepaint(_TriangleRightPainter oldDelegate) =>
      oldDelegate.color != color;
}
