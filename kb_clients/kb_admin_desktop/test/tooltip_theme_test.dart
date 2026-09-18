// 气泡提示（Tooltip）的配色测试。
//
// 背景：提示条的底色 `tooltip-bg` 在**两种主题下都是深色**
// （浅色主题 `n850` / 深色主题 `n750`），所以文字必须固定用近白色。
// 早先用的是 `labelPrimary`——它跟着主题翻转，于是浅色模式下变成
// "深底深字"，几乎看不见。
//
// 这里不去钉具体色号（那样调色板一动就得改测试），而是钉**对比关系**：
// 字的亮度要足够高、底的亮度要足够低。DSH 上游同样是这么配的
// （`ui-primitives/Tooltip.module.css`：背景 `--dsw-alias-tooltip-bg`、
// 文字 `--dsw-static-neutral-bluish-00`）。

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kb_admin_desktop/src/theme/app_theme.dart';

void main() {
  /// 测试两种主题下气泡提示都是"深底 + 近白字"。
  ///
  /// - 手段：按主题装配 `MaterialApp`，在组件树里用 `TooltipTheme.of` 取出
  ///   **真正生效**的那份配置（而不是只看 `buildDswTheme` 的返回值）。
  /// - 判断：文字的相对亮度 > 0.5、底色的相对亮度 < 0.2，且两者的差距 > 0.4。
  ///   浅色主题下若有人再把文字改回 `labelPrimary`（近黑），这条会立刻失败。
  testWidgets('气泡提示在两种主题下都是深底近白字', (WidgetTester tester) async {
    for (final Brightness brightness in <Brightness>[
      Brightness.dark,
      Brightness.light,
    ]) {
      late TooltipThemeData resolved;
      await tester.pumpWidget(
        MaterialApp(
          theme: buildDswTheme(brightness),
          home: Builder(
            builder: (BuildContext context) {
              resolved = TooltipTheme.of(context);
              return const SizedBox.shrink();
            },
          ),
        ),
      );
      // `MaterialApp` 用 `AnimatedTheme` 在主题之间做过渡；不等它走完，这里读到的
      // 还是上一轮那套颜色（第一次循环就是深色），测试会假通过。
      await tester.pumpAndSettle();

      final Color? foreground = resolved.textStyle?.color;
      final Color? background = (resolved.decoration as BoxDecoration?)?.color;
      expect(foreground, isNotNull, reason: '$brightness：应当设了文字颜色');
      expect(background, isNotNull, reason: '$brightness：应当设了底色');

      final double fg = foreground!.computeLuminance();
      final double bg = background!.computeLuminance();
      expect(
        fg,
        greaterThan(0.5),
        reason: '$brightness：提示文字应当够亮（实际亮度 $fg）',
      );
      expect(
        bg,
        lessThan(0.2),
        reason: '$brightness：提示底色应当够暗（实际亮度 $bg）',
      );
      expect(
        fg - bg,
        greaterThan(0.4),
        reason: '$brightness：前后景亮度差太小，会看不清（$fg vs $bg）',
      );
    }
  });
}
