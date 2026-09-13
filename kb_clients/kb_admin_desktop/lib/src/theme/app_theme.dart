// 把 DSH 的设计令牌组装成 Flutter 的 [ThemeData]。
//
// 组织方式与 `kb_svc_salvo/src/web/assets/app.css` 相同：令牌只有一份
// （`dsw_tokens.dart`），主题只负责把语义别名接到 Material 的各个槽位上。
// 组件本身尽量不写死颜色，一律通过 `context.dsw` 取用。

import 'package:flutter/material.dart';

import 'dsw_tokens.dart';

/// 与 DSH `--dsw-font-family` / `--ds-font-family-code` 对应的字体栈。
abstract final class DswTypography {
  /// 正文字体回退链：与 DSH 的 `--dsw-font-family` 一致，末尾补上 Linux 上
  /// 常见的中文字体，避免 Flutter 在无 `PingFang SC` 的平台上出现方框。
  static const List<String> sansFallback = <String>[
    'PingFang SC',
    'Hiragino Sans GB',
    'Microsoft YaHei',
    'Noto Sans CJK SC',
    'Source Han Sans SC',
    'WenQuanYi Micro Hei',
    'Helvetica Neue',
    'Helvetica',
    'Arial',
  ];

  /// 等宽字体回退链（`--ds-font-family-code`，刻意不以裸 `monospace` 结尾）。
  static const List<String> monoFallback = <String>[
    'SF Mono',
    'JetBrains Mono',
    'Fira Code',
    'Consolas',
    'Liberation Mono',
    'Menlo',
    'Courier',
    'PingFang SC',
    'Microsoft YaHei',
  ];

  /// 正文：14px / 1.6，与 `app.css` 的 `body` 一致。
  static const TextStyle body = TextStyle(
    fontSize: 14,
    height: 1.6,
    fontFamilyFallback: sansFallback,
  );

  /// 小号说明文字。
  static const TextStyle caption = TextStyle(
    fontSize: 12,
    height: 1.5,
    fontFamilyFallback: sansFallback,
  );

  /// 等宽正文。
  static const TextStyle mono = TextStyle(
    fontSize: 13,
    height: 1.5,
    fontFamilyFallback: monoFallback,
  );
}

/// 依据 [brightness] 生成一套 DSH 主题。
///
/// 深色是客户端的默认主题，与 DSH 和 `kb_svc_salvo` 的网页端保持一致。
ThemeData buildDswTheme(Brightness brightness) {
  final bool isDark = brightness == Brightness.dark;
  final DswColors c = isDark ? DswColors.dark : DswColors.light;

  final ColorScheme scheme =
      ColorScheme.fromSeed(
        seedColor: c.brandPrimary,
        brightness: brightness,
      ).copyWith(
        primary: c.brandPrimary,
        onPrimary: isDark
            ? DswStaticNeutralBluish.n1000
            : DswStaticNeutralBluish.n00,
        surface: c.bgBase,
        onSurface: c.labelPrimary,
        error: c.stateError,
      );

  final TextTheme textTheme = TextTheme(
    bodyLarge: DswTypography.body.copyWith(color: c.labelPrimary),
    bodyMedium: DswTypography.body.copyWith(color: c.labelPrimary),
    bodySmall: DswTypography.caption.copyWith(color: c.labelSecondary),
    labelLarge: DswTypography.body.copyWith(
      color: c.labelPrimary,
      fontWeight: FontWeight.w500,
    ),
    labelMedium: DswTypography.caption.copyWith(color: c.labelSecondary),
    titleMedium: DswTypography.body.copyWith(
      color: c.labelPrimary,
      fontSize: 16,
      fontWeight: FontWeight.w600,
    ),
    headlineSmall: DswTypography.body.copyWith(
      color: c.labelPrimary,
      fontSize: 28,
      height: 1.3,
      fontWeight: FontWeight.w600,
    ),
  );

  return ThemeData(
    useMaterial3: true,
    brightness: brightness,
    colorScheme: scheme,
    // DSH 没有水波纹与高亮扫过，这里一并关掉，避免按下时出现 Material 的涟漪。
    splashFactory: NoSplash.splashFactory,
    splashColor: Colors.transparent,
    highlightColor: Colors.transparent,
    hoverColor: c.interactiveHover,
    scaffoldBackgroundColor: c.bgBase,
    canvasColor: c.bgBase,
    dividerColor: c.borderL2,
    fontFamilyFallback: DswTypography.sansFallback,
    textTheme: textTheme,
    extensions: <ThemeExtension<dynamic>>[c],
    tooltipTheme: TooltipThemeData(
      waitDuration: const Duration(milliseconds: 500),
      decoration: BoxDecoration(
        color: c.tooltipBg,
        borderRadius: BorderRadius.circular(8),
      ),
      textStyle: DswTypography.caption.copyWith(color: c.labelPrimary),
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
    ),
    scrollbarTheme: ScrollbarThemeData(
      thickness: const WidgetStatePropertyAll<double>(6),
      radius: const Radius.circular(3),
      thumbColor: WidgetStatePropertyAll<Color>(c.scrollbarThumb),
      crossAxisMargin: 2,
    ),
    textSelectionTheme: TextSelectionThemeData(
      cursorColor: c.brandPrimary,
      selectionColor: c.brandPrimary.withValues(alpha: 0.28),
      selectionHandleColor: c.brandPrimary,
    ),
  );
}
