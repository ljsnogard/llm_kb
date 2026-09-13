// DSH 风格的通用控件。
//
// DSH 的界面几乎没有 Material 的痕迹：没有水波纹、没有投影按钮，交互反馈
// 全部是「悬停时换一层底色」（`--dsw-alias-interactive-bg-hover`）。
// 这里的控件把这条约定固定下来，页面里就不用反复写 `MouseRegion`。

import 'package:flutter/material.dart';

import '../../theme/app_theme.dart';
import '../../theme/dsw_tokens.dart';

/// 监听指针是否悬停在本区域内，并把结果交给 [builder]。
class HoverBuilder extends StatefulWidget {
  /// 构造一个悬停探测区。
  const HoverBuilder({
    super.key,
    required this.builder,
    this.cursor = SystemMouseCursors.click,
    this.enabled = true,
  });

  /// 构建函数，`hovered` 表示指针是否在区域内。
  final Widget Function(BuildContext context, bool hovered) builder;

  /// 指针样式；不需要手型时传 [SystemMouseCursors.basic]。
  final MouseCursor cursor;

  /// 为 `false` 时不响应悬停（用于禁用态）。
  final bool enabled;

  @override
  State<HoverBuilder> createState() => _HoverBuilderState();
}

class _HoverBuilderState extends State<HoverBuilder> {
  bool _hovered = false;

  @override
  Widget build(BuildContext context) {
    return MouseRegion(
      cursor: widget.enabled ? widget.cursor : SystemMouseCursors.basic,
      onEnter: widget.enabled ? (_) => setState(() => _hovered = true) : null,
      onExit: widget.enabled ? (_) => setState(() => _hovered = false) : null,
      child: widget.builder(context, widget.enabled && _hovered),
    );
  }
}

/// DSH 的工具条图标按钮：28×28 圆形，悬停时出现一层底色。
///
/// 对应 `.iconButton`（会话头、右侧栏）与 `ExpandButton.module.css` 的 `.button`。
class DswIconButton extends StatelessWidget {
  /// 构造一个图标按钮。
  const DswIconButton({
    super.key,
    this.icon,
    required this.tooltip,
    this.onPressed,
    this.size = DswLayout.iconButtonSize,
    this.iconSize = 15,
    this.glyph,
    this.borderRadius,
  }) : assert(
         icon != null || glyph != null,
         'DswIconButton 需要 icon 或 glyph 之一',
       );

  /// 图标。
  final IconData? icon;

  /// 自绘图形；与 [icon] 二选一。
  final Widget? glyph;

  /// 悬停提示与无障碍标签。
  final String tooltip;

  /// 点击回调；为 `null` 时是禁用态。
  final VoidCallback? onPressed;

  /// 按钮边长。
  final double size;

  /// 图标边长。
  final double iconSize;

  /// 圆角；默认取 [size] 的一半，即正圆。
  final BorderRadius? borderRadius;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    final bool enabled = onPressed != null;
    return Tooltip(
      message: tooltip,
      child: HoverBuilder(
        enabled: enabled,
        builder: (BuildContext context, bool hovered) {
          final Color ink = enabled
              ? (hovered ? c.labelPrimary : c.labelSecondary)
              : c.labelCaption;
          return Semantics(
            button: true,
            label: tooltip,
            child: GestureDetector(
              behavior: HitTestBehavior.opaque,
              onTap: onPressed,
              child: AnimatedContainer(
                duration: DswMotion.fast,
                curve: DswMotion.easeInOut,
                width: size,
                height: size,
                decoration: BoxDecoration(
                  color: hovered && enabled
                      ? c.interactiveHover
                      : Colors.transparent,
                  borderRadius:
                      borderRadius ?? BorderRadius.circular(size / 2),
                ),
                alignment: Alignment.center,
                child: IconTheme(
                  data: IconThemeData(color: ink, size: iconSize),
                  child: glyph ?? Icon(icon, size: iconSize),
                ),
              ),
            ),
          );
        },
      ),
    );
  }
}

/// 主按钮：实心填充 + 圆角小方块，对应 DSH 的 `--dsw-alias-button-primary-*`。
///
/// 注意这套令牌里的「主按钮」是**墨色**（深色主题下是近白、浅色主题下是近黑），
/// 而不是蓝色；那一抹 deepseek 蓝只留给发送按钮与选中态强调。
class DswPrimaryButton extends StatelessWidget {
  /// 构造一个主按钮。
  const DswPrimaryButton({
    super.key,
    required this.label,
    this.onPressed,
    this.icon,
  });

  /// 按钮文字。
  final String label;

  /// 点击回调；为 `null` 时是禁用态。
  final VoidCallback? onPressed;

  /// 可选的前置图标。
  final IconData? icon;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    final bool enabled = onPressed != null;
    return HoverBuilder(
      enabled: enabled,
      builder: (BuildContext context, bool hovered) {
        return GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTap: onPressed,
          child: AnimatedOpacity(
            duration: DswMotion.fast,
            opacity: enabled ? 1 : 0.4,
            child: AnimatedContainer(
              duration: DswMotion.fast,
              curve: DswMotion.easeInOut,
              height: 32,
              padding: const EdgeInsets.symmetric(horizontal: 16),
              decoration: BoxDecoration(
                color: hovered ? c.buttonPrimaryHover : c.buttonPrimaryFill,
                borderRadius: BorderRadius.circular(
                  DswLayout.iconButtonSize / 2,
                ),
              ),
              child: Row(
                mainAxisSize: MainAxisSize.min,
                children: <Widget>[
                  if (icon != null) ...<Widget>[
                    Icon(icon, size: 15, color: c.buttonPrimaryForeground),
                    const SizedBox(width: 6),
                  ],
                  Text(
                    label,
                    style: DswTypography.body.copyWith(
                      color: c.buttonPrimaryForeground,
                      fontWeight: FontWeight.w500,
                    ),
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

/// 幽灵按钮：透明底 + 细描边，对应 `.btn--ghost`。
class DswGhostButton extends StatelessWidget {
  /// 构造一个幽灵按钮。
  const DswGhostButton({
    super.key,
    required this.label,
    this.onPressed,
    this.danger = false,
    this.icon,
  });

  /// 按钮文字。
  final String label;

  /// 点击回调。
  final VoidCallback? onPressed;

  /// 是否用危险色（删除类操作）。
  final bool danger;

  /// 可选的前置图标。
  final IconData? icon;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    final bool enabled = onPressed != null;
    return HoverBuilder(
      enabled: enabled,
      builder: (BuildContext context, bool hovered) {
        final Color ink = !enabled
            ? c.labelCaption
            : danger && hovered
            ? c.stateError
            : (hovered ? c.labelPrimary : c.labelSecondary);
        return GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTap: onPressed,
          child: AnimatedContainer(
            duration: DswMotion.fast,
            curve: DswMotion.easeInOut,
            height: 32,
            padding: const EdgeInsets.symmetric(horizontal: 14),
            decoration: BoxDecoration(
              border: Border.all(color: c.borderL3),
              borderRadius: BorderRadius.circular(6),
            ),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: <Widget>[
                if (icon != null) ...<Widget>[
                  Icon(icon, size: 15, color: ink),
                  const SizedBox(width: 6),
                ],
                Text(
                  label,
                  style: DswTypography.body.copyWith(color: ink),
                ),
              ],
            ),
          ),
        );
      },
    );
  }
}

/// 极小的行内按钮，对应网页端的 `.mini-btn`（服务条目上的「使用 / 编辑 / 删除」）。
class DswMiniButton extends StatelessWidget {
  /// 构造一个小按钮。
  const DswMiniButton({
    super.key,
    required this.label,
    this.onPressed,
    this.danger = false,
  });

  /// 按钮文字。
  final String label;

  /// 点击回调。
  final VoidCallback? onPressed;

  /// 是否用危险色。
  final bool danger;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    return HoverBuilder(
      builder: (BuildContext context, bool hovered) {
        final Color ink = !hovered
            ? c.labelSecondary
            : (danger ? c.stateError : c.labelPrimary);
        return GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTap: onPressed,
          child: Container(
            padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
            decoration: BoxDecoration(
              border: Border.all(color: c.borderL3),
              borderRadius: BorderRadius.circular(6),
            ),
            child: Text(
              label,
              style: DswTypography.caption.copyWith(color: ink),
            ),
          ),
        );
      },
    );
  }
}

/// DSH 的 0.5px 发丝分隔线。
///
/// CSS 里到处是 `0.5px` 的描边；Flutter 在 1x 屏上画不出半个逻辑像素，
/// 因此统一取 1 个物理像素的观感：[BorderSide.width] = 1 会偏粗，
/// 这里用 0.5 交给引擎做抗锯齿，在高 DPI 屏上最接近 DSH。
class DswHairline extends StatelessWidget {
  /// 构造一条分隔线。
  const DswHairline({super.key, this.axis = Axis.horizontal, this.color});

  /// 线条方向。
  final Axis axis;

  /// 颜色；默认取当前主题的 `borderL3`。
  final Color? color;

  @override
  Widget build(BuildContext context) {
    final Color resolved = color ?? context.dsw.borderL3;
    return axis == Axis.horizontal
        ? Container(height: 0.5, color: resolved)
        : Container(width: 0.5, color: resolved);
  }
}

/// 区块小标题，对应 `.panel__subtitle` / 工作区列表的「工作区」。
class DswSectionLabel extends StatelessWidget {
  /// 构造一个区块标题。
  const DswSectionLabel(this.text, {super.key, this.padding});

  /// 标题文字。
  final String text;

  /// 自定义内边距。
  final EdgeInsetsGeometry? padding;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    return Padding(
      padding: padding ?? const EdgeInsets.fromLTRB(8, 0, 8, 6),
      child: Text(
        text,
        style: DswTypography.caption.copyWith(
          color: c.labelCaption,
          fontWeight: FontWeight.w500,
          letterSpacing: 0.6,
        ),
      ),
    );
  }
}

/// 带标题的输入框，对齐网页端 `.field`（小号标题 + 深色底 + 细描边）。
class DswLabeledField extends StatelessWidget {
  /// 构造一个带标题的输入框。
  const DswLabeledField({
    super.key,
    required this.label,
    required this.controller,
    this.hint,
    this.obscure = false,
    this.suffix,
    this.onSubmitted,
  });

  /// 字段标题。
  final String label;

  /// 文本控制器。
  final TextEditingController controller;

  /// 占位文字。
  final String? hint;

  /// 是否遮蔽输入（API key）。
  final bool obscure;

  /// 右侧附加控件（例如「显示 / 隐藏」）。
  final Widget? suffix;

  /// 提交回调。
  final VoidCallback? onSubmitted;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: <Widget>[
        Text(
          label,
          style: DswTypography.caption.copyWith(color: c.labelCaption),
        ),
        const SizedBox(height: 4),
        TextField(
          controller: controller,
          obscureText: obscure,
          onSubmitted: onSubmitted == null ? null : (_) => onSubmitted!(),
          style: DswTypography.body.copyWith(color: c.labelPrimary),
          decoration: InputDecoration(
            isDense: true,
            hintText: hint,
            hintStyle: DswTypography.body.copyWith(color: c.labelCaption),
            suffixIcon: suffix,
            suffixIconConstraints: const BoxConstraints(
              minWidth: 36,
              minHeight: 36,
            ),
            filled: true,
            fillColor: c.bgBase,
            contentPadding: const EdgeInsets.symmetric(
              horizontal: 10,
              vertical: 10,
            ),
            enabledBorder: OutlineInputBorder(
              borderRadius: BorderRadius.circular(6),
              borderSide: BorderSide(color: c.borderL3),
            ),
            focusedBorder: OutlineInputBorder(
              borderRadius: BorderRadius.circular(6),
              borderSide: BorderSide(color: c.stateBusinessPrimary),
            ),
          ),
        ),
      ],
    );
  }
}
