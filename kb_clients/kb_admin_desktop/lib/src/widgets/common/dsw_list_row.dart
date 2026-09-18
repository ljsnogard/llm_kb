// 列表行的公共骨架与时间格式。
//
// 本地工作区列表（[WorkspaceList]）与服务端工作区列表
// （[ServerWorkspaceList]）共用这一份：固定行高、圆角、悬停 / 选中用同一种底色。
//
// DSH 的刻意取舍：侧边栏里**选中态与悬停态用同一种底色**，没有第二套强调色。

import 'package:flutter/material.dart';

import '../../theme/app_theme.dart';
import '../../theme/dsw_tokens.dart';

/// 侧边栏列表的一行。
class DswListRow extends StatelessWidget {
  /// 构造一行。
  const DswListRow({
    super.key,
    required this.height,
    required this.selected,
    required this.onTap,
    required this.leading,
    required this.title,
    required this.trailing,
    this.onLeadingTap,
    this.subtitle,
  });

  /// 行高（工作区 34，会话 32）。
  final double height;

  /// 是否高亮（选中或悬停）。
  final bool selected;

  /// 点击整行。
  final VoidCallback onTap;

  /// 点击行首图标（用于「展开 / 收起」这类与选中不同的动作）。
  final VoidCallback? onLeadingTap;

  /// 行首控件。
  final Widget leading;

  /// 主文字。
  final String title;

  /// 次要文字（可空）。
  final String? subtitle;

  /// 行尾控件。
  final List<Widget> trailing;

  @override
  Widget build(BuildContext context) {
    final DswColors c = context.dsw;
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: onTap,
      child: AnimatedContainer(
        duration: DswMotion.fast,
        curve: DswMotion.easeInOut,
        height: height,
        padding: const EdgeInsets.symmetric(horizontal: 8),
        decoration: BoxDecoration(
          color: selected ? c.interactiveHover : Colors.transparent,
          borderRadius: BorderRadius.circular(8),
        ),
        child: Row(
          children: <Widget>[
            SizedBox(
              width: 16,
              child: Center(
                child: onLeadingTap == null
                    ? leading
                    : GestureDetector(
                        behavior: HitTestBehavior.opaque,
                        onTap: onLeadingTap,
                        child: leading,
                      ),
              ),
            ),
            const SizedBox(width: 6),
            Expanded(
              child: Text(
                title,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: DswTypography.body.copyWith(
                  fontSize: 14,
                  height: 20 / 14,
                  color: c.labelPrimary,
                ),
              ),
            ),
            const SizedBox(width: 6),
            ...trailing,
          ],
        ),
      ),
    );
  }
}

/// 会话时间戳按「刚刚 / HH:mm / M月d日」三档显示。
String formatSessionTime(DateTime time) {
  final DateTime now = DateTime.now();
  final Duration age = now.difference(time);
  if (age.inMinutes < 1) {
    return '刚刚';
  }
  if (age.inHours < 12 && now.day == time.day) {
    final String hh = time.hour.toString().padLeft(2, '0');
    final String mm = time.minute.toString().padLeft(2, '0');
    return '$hh:$mm';
  }
  return '${time.month}月${time.day}日';
}
