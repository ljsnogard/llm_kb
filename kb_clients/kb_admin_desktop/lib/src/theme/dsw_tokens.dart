// DSH（deepseek-harness）v0.1.5-rc 设计令牌的 Flutter 移植。
//
// 命名与层级刻意与 DSH 的样式表保持一致，便于两边对照：
//
//   `--dsw-static-*`  原始色阶（`packages/client/ui-theme/src/styles/design-platform.css`）
//        ↓
//   `--dsw-alias-*`   语义别名（同上文件 body / body[data-ds-dark-theme] 段）
//        ↓
//   本文件的 [DswColors] 字段
//
// 另外两项来自 DSH 的布局与动效约定：
//
// - 列宽几何来自 `packages/client/ui-layout/src/client/columns.ts`，见 [DswLayout]；
// - 动效曲线来自 `packages/client/ui-theme/src/styles/base.css`
//   （`--ds-ease-in-out: cubic-bezier(0.4, 0, 0.2, 1)`、
//   `--ds-transition-duration-slow: 0.3s`），见 [DswMotion]。
//
// 这些常量属于客户端的观感约定，改动前请先确认是否仍需与 DSH 对齐。

import 'package:flutter/material.dart';

// ============================================================================
// 原始色阶（对应 --dsw-static-*）
// ============================================================================

/// DSH 原始色阶中的中性偏蓝色阶，深色界面几乎全部由它构成。
abstract final class DswStaticNeutralBluish {
  static const Color n00 = Color(0xFFFFFFFF);
  static const Color n50 = Color(0xFFF9FAFB);
  static const Color n60 = Color(0xFFF5F6F7);
  static const Color n75 = Color(0xFFF1F3F5);
  static const Color n100 = Color(0xFFEBEEF2);
  static const Color n150 = Color(0xFFE9ECF2);
  static const Color n200 = Color(0xFFE1E5EE);
  static const Color n300 = Color(0xFFCFD3D6);
  static const Color n400 = Color(0xFFADB2B8);
  static const Color n500 = Color(0xFF979DA6);
  static const Color n600 = Color(0xFF81858C);
  static const Color n700 = Color(0xFF61666B);
  static const Color n750 = Color(0xFF43454A);
  static const Color n800 = Color(0xFF353638);
  static const Color n850 = Color(0xFF2C2C2E);
  static const Color n875 = Color(0xFF232324);
  static const Color n900 = Color(0xFF1B1B1C);
  static const Color n950 = Color(0xFF151517);
  static const Color n1000 = Color(0xFF0F1115);
}

/// DSH 原始色阶中的中性色阶（少量别名仍然引用它）。
abstract final class DswStaticNeutral {
  static const Color n50 = Color(0xFFFAFAFA);
  static const Color n200 = Color(0xFFE5E5E5);
  static const Color n300 = Color(0xFFD4D4D4);
  static const Color n550 = Color(0xFF65676B);
  static const Color n600 = Color(0xFF545557);
  static const Color n700 = Color(0xFF3C3C3D);
  static const Color n800 = Color(0xFF292929);
}

/// DSH 原始色阶中的蓝色阶（浅色主题的强调色）。
abstract final class DswStaticBlue {
  static const Color b100 = Color(0xFFDBEAFE);
  static const Color b500 = Color(0xFF3B82F6);
  static const Color b600 = Color(0xFF2563EB);
  static const Color b900 = Color(0xFF0E3074);
}

/// DSH 原始色阶中的品牌（deepseek）色阶。
abstract final class DswStaticDeepseek {
  static const Color d50 = Color(0xFFEDF3FE);
  static const Color d100 = Color(0xFFE4EDFD);
  static const Color d200 = Color(0xFFD3E2FF);
  static const Color d400 = Color(0xFF679EFE);
  static const Color d450 = Color(0xFF5686FE);
  static const Color d500 = Color(0xFF4176E6);
  static const Color d800 = Color(0xFF34415B);
}

/// DSH 原始色阶中的状态色。
abstract final class DswStaticState {
  static const Color red400 = Color(0xFFF25A5A);
  static const Color red500 = Color(0xFFEF4444);
  static const Color red600 = Color(0xFFEC1313);
  static const Color red900 = Color(0xFF570C0C);

  static const Color green400 = Color(0xFF4ED17E);
  static const Color green500 = Color(0xFF22C55E);
  static const Color green900 = Color(0xFF233C2C);

  static const Color amber400 = Color(0xFFF7AD31);
  static const Color amber500 = Color(0xFFF59E0B);
  static const Color amber600 = Color(0xFFDD8629);
}

// ============================================================================
// 布局几何（对应 --dsh-sidebar-* 与 columns.ts）
// ============================================================================

/// 三栏框架的列宽几何。
///
/// 数值与 DSH 的 `columns.ts` 一一对应，说明见各字段注释。
abstract final class DswLayout {
  /// 侧边栏拖拽下界（`SIDEBAR_MIN`）。
  static const double sidebarMin = 264;

  /// 侧边栏拖拽上界（`SIDEBAR_MAX`）。
  static const double sidebarMax = 420;

  /// 用户未拖拽时的侧边栏宽度（`SIDEBAR_DEFAULT`）。
  static const double sidebarDefault = 280;

  /// 折叠后的图标轨道宽度（`SIDEBAR_COLLAPSED`）：24px 图标 + 两侧 16px 内边距。
  static const double sidebarCollapsed = 56;

  /// 低于该视口宽度时侧边栏自动折叠成轨道（`SIDEBAR_AUTO_COLLAPSE`）。
  static const double sidebarAutoCollapse = 1024;

  /// 右侧栏展开时为中心区保留的最小宽度（`CENTER_MIN`）。
  static const double centerMin = 400;

  /// 右侧栏拖拽下界（`RIGHTBAR_MIN`）。
  static const double rightbarMin = 300;

  /// 右侧栏占视口的最大比例（`RIGHTBAR_MAX_RATIO`）。
  static const double rightbarMaxRatio = 0.7;

  /// 右侧栏首次打开时占视口的比例（`RIGHTBAR_DEFAULT_RATIO`）。
  static const double rightbarDefaultRatio = 0.45;

  /// 侧边栏左右内边距（`--dsh-sidebar-inline-padding`）。
  static const double sidebarInlinePadding = 12;

  /// 会话头 / 工具条上图标按钮的直径。
  static const double iconButtonSize = 28;

  /// 图标轨道里控件盒子的边长（36x36，居中于 56px 轨道）。
  static const double railControlSize = 36;

  /// 对话正文的最大宽度（`--dsw-layout-max`）。
  static const double contentMaxWidth = 760;

  /// 拖拽热区的宽度（DSH 的 `.handle` 是 8px，左右各溢出 4px）。
  static const double resizeHandleWidth = 8;

  /// 窗口允许缩到的最小宽度（逻辑像素）。
  ///
  /// 由布局反推：折叠轨道 [sidebarCollapsed] 56 + 中间区最小 [centerMin] 400 +
  /// 右侧栏最小 [rightbarMin] 300 = 756，取 800 留一点余量。窗口窄于
  /// [sidebarAutoCollapse] 时侧边栏本来就会自动折叠，所以这里是"最紧的那种形态"。
  ///
  /// **原生 runner 读不到这个常量**，三个平台各自硬编码了同一个数值：
  /// `linux/runner/my_application.cc`、`windows/runner/win32_window.cpp`、
  /// `macos/Runner/MainFlutterWindow.swift`。改动时三处要一起改
  /// （`test/column_geometry_test.dart` 有一条断言守着这个关系）。
  static const double minWindowWidth = 800;

  /// 窗口允许缩到的最小高度：列头 76 + 输入区约 64 + 若干行消息。
  ///
  /// 同样需要在三个原生 runner 里同步（见 [minWindowWidth]）。
  static const double minWindowHeight = 520;

  /// 把宽度夹到侧边栏的合法区间并取整（DSH 的 `clampWidth` 也会 round）。
  static double clampSidebar(double px) =>
      px.clamp(sidebarMin, sidebarMax).roundToDouble();

  /// 把宽度夹到右侧栏的合法区间并取整。
  ///
  /// 视口极窄时上界（视口 × [rightbarMaxRatio]）会小于下界 [rightbarMin]，
  /// 直接 `clamp` 会抛断言，因此这里先把下界收敛到上界。
  static double clampRightbar(double px, double viewport) {
    final double upper = viewport * rightbarMaxRatio;
    final double lower = upper < rightbarMin ? upper : rightbarMin;
    return px.clamp(lower, upper).roundToDouble();
  }
}

/// 三栏求解结果，对应 DSH `columns.ts` 的 `Columns`。
@immutable
class ColumnGeometry {
  /// 构造一份列宽结果。
  const ColumnGeometry({
    required this.sidebar,
    required this.center,
    required this.rightbar,
  });

  /// 侧边栏实际宽度；折叠时是 [DswLayout.sidebarCollapsed]。
  final double sidebar;

  /// 中间对话区实际宽度。
  final double center;

  /// 右侧栏实际占用的轨道宽度；为 0 表示不占轨道（面板悬挂在中心区之上）。
  final double rightbar;

  /// 求某一帧下的三栏宽度。
  ///
  /// 与 DSH 的 `computeColumns` 同规则：右侧栏先被压缩、再整条轨道消失，
  /// 之后中心区才可以低于 `CENTER_MIN`；侧边栏在此过程中不让步。
  ///
  /// - [viewport]：框架可用宽度。
  /// - [sidebar]：侧边栏宽度偏好，0 表示折叠。
  /// - [rightbar]：右侧栏请求宽度，0 表示不占轨道。
  static ColumnGeometry solve({
    required double viewport,
    required double sidebar,
    required double rightbar,
  }) {
    final double s = sidebar == 0
        ? DswLayout.sidebarCollapsed
        : DswLayout.clampSidebar(sidebar);
    final double available = viewport - s - DswLayout.centerMin;
    final double clamped = DswLayout.clampRightbar(rightbar, viewport);
    final double r = rightbar == 0 || available < DswLayout.rightbarMin
        ? 0
        : (available < clamped ? available : clamped);
    return ColumnGeometry(
      sidebar: s,
      center: (viewport - s - r) < 0 ? 0 : viewport - s - r,
      rightbar: r,
    );
  }

  @override
  bool operator ==(Object other) =>
      other is ColumnGeometry &&
      other.sidebar == sidebar &&
      other.center == center &&
      other.rightbar == rightbar;

  @override
  int get hashCode => Object.hash(sidebar, center, rightbar);
}

// ============================================================================
// 动效（对应 --ds-ease-in-out / --ds-transition-duration-*）
// ============================================================================

/// DSH 的动效曲线与时长。
abstract final class DswMotion {
  /// `--ds-ease-in-out`：列宽滑动、面板推拉统一走这条曲线。
  static const Cubic easeInOut = Cubic(0.4, 0.0, 0.2, 1.0);

  /// `--ds-transition-duration-slow`：整列折叠 / 展开的时长。
  static const Duration slow = Duration(milliseconds: 300);

  /// `--ds-transition-duration`：通用过渡时长。
  static const Duration normal = Duration(milliseconds: 200);

  /// 侧边栏内容淡出、轨道图标淡入的时长（DSH `COLLAPSE_SETTLE_MS`）。
  static const Duration settle = Duration(milliseconds: 150);

  /// 悬停 / 按下等即时反馈的时长。
  static const Duration fast = Duration(milliseconds: 100);
}

// ============================================================================
// 语义别名（对应 --dsw-alias-*）
// ============================================================================

// `--dsw-elevation-soft` / `--dsw-elevation-panel` 的投影值。
// DSH 是「一条 0.5px 描边 + 两条很淡的投影」，这里用同样的构成。
const List<BoxShadow> _elevationSoftDark = <BoxShadow>[
  BoxShadow(color: Color(0x0FFFFFFF), spreadRadius: 0.5, blurRadius: 0),
  BoxShadow(color: Color(0x40000000), blurRadius: 8, offset: Offset(0, 2)),
];
const List<BoxShadow> _elevationSoftLight = <BoxShadow>[
  BoxShadow(color: Color(0x14000000), spreadRadius: 0.5, blurRadius: 0),
  BoxShadow(color: Color(0x0F000000), blurRadius: 8, offset: Offset(0, 2)),
];
const List<BoxShadow> _elevationPanelDark = <BoxShadow>[
  BoxShadow(color: Color(0x14FFFFFF), spreadRadius: 0.5, blurRadius: 0),
  BoxShadow(color: Color(0x59000000), blurRadius: 24, offset: Offset(0, 8)),
];
const List<BoxShadow> _elevationPanelLight = <BoxShadow>[
  BoxShadow(color: Color(0x1A000000), spreadRadius: 0.5, blurRadius: 0),
  BoxShadow(color: Color(0x1F000000), blurRadius: 24, offset: Offset(0, 8)),
];

/// 一套主题下实际用到的语义色。
///
/// 作为 [ThemeExtension] 挂在 [ThemeData.extensions] 上，用
/// `Theme.of(context).extension<DswColors>()!` 取用。
@immutable
class DswColors extends ThemeExtension<DswColors> {
  /// 构造一套语义色。
  const DswColors({
    required this.isDark,
    required this.bgBase,
    required this.bgLayer1,
    required this.bgLayer2,
    required this.bgLayer3,
    required this.overlay,
    required this.borderL1,
    required this.borderL2,
    required this.borderL3,
    required this.borderL4,
    required this.labelPrimary,
    required this.labelSecondary,
    required this.labelTertiary,
    required this.labelCaption,
    required this.labelPrimaryInverted,
    required this.interactiveHover,
    required this.interactiveActive,
    required this.sidebarFill,
    required this.sidebarItemHover,
    required this.sidebarItemActive,
    required this.buttonElevatedFill,
    required this.buttonFloatingHover,
    required this.buttonPrimaryFill,
    required this.buttonPrimaryHover,
    required this.buttonPrimaryForeground,
    required this.bubble,
    required this.bubbleHighlight,
    required this.brandPrimary,
    required this.brandText,
    required this.link,
    required this.stateBusinessPrimary,
    required this.buttonInfoFill,
    required this.buttonInfoHover,
    required this.markdownTag,
    required this.elevationSoft,
    required this.elevationPanel,
    required this.inputMajor,
    required this.stateError,
    required this.stateWarn,
    required this.stateSuccess,
    required this.scrollbarThumb,
    required this.tooltipBg,
  });

  /// DSH 的深色主题（客户端默认）。
  static const DswColors dark = DswColors(
    isDark: true,
    bgBase: DswStaticNeutralBluish.n950,
    bgLayer1: DswStaticNeutralBluish.n875,
    bgLayer2: DswStaticNeutralBluish.n850,
    bgLayer3: DswStaticNeutralBluish.n800,
    overlay: Color(0x80000000),
    borderL1: Color(0x0FFFFFFF),
    borderL2: Color(0x1FFFFFFF),
    borderL3: Color(0x29FFFFFF),
    borderL4: Color(0x33FFFFFF),
    labelPrimary: DswStaticNeutralBluish.n50,
    labelSecondary: DswStaticNeutralBluish.n300,
    labelTertiary: DswStaticNeutralBluish.n400,
    labelCaption: DswStaticNeutralBluish.n600,
    labelPrimaryInverted: DswStaticNeutralBluish.n800,
    interactiveHover: Color(0x14FFFFFF),
    interactiveActive: Color(0x24FFFFFF),
    sidebarFill: DswStaticNeutralBluish.n900,
    sidebarItemHover: DswStaticNeutralBluish.n850,
    sidebarItemActive: DswStaticNeutralBluish.n750,
    buttonElevatedFill: DswStaticNeutralBluish.n750,
    buttonFloatingHover: DswStaticNeutralBluish.n800,
    buttonPrimaryFill: DswStaticNeutralBluish.n50,
    buttonPrimaryHover: DswStaticNeutralBluish.n100,
    buttonPrimaryForeground: DswStaticNeutralBluish.n1000,
    bubble: DswStaticNeutralBluish.n850,
    bubbleHighlight: DswStaticNeutralBluish.n750,
    brandPrimary: DswStaticDeepseek.d450,
    brandText: DswStaticNeutralBluish.n50,
    link: DswStaticDeepseek.d400,
    stateBusinessPrimary: DswStaticDeepseek.d400,
    buttonInfoFill: DswStaticDeepseek.d400,
    buttonInfoHover: DswStaticDeepseek.d500,
    markdownTag: DswStaticNeutralBluish.n850,
    elevationSoft: _elevationSoftDark,
    elevationPanel: _elevationPanelDark,
    inputMajor: DswStaticNeutralBluish.n850,
    stateError: DswStaticState.red400,
    stateWarn: DswStaticState.amber400,
    stateSuccess: DswStaticState.green500,
    scrollbarThumb: DswStaticNeutral.n700,
    tooltipBg: DswStaticNeutralBluish.n750,
  );

  /// DSH 的浅色主题。
  static const DswColors light = DswColors(
    isDark: false,
    bgBase: DswStaticNeutralBluish.n00,
    bgLayer1: DswStaticNeutralBluish.n00,
    bgLayer2: DswStaticNeutralBluish.n00,
    bgLayer3: DswStaticNeutralBluish.n00,
    overlay: Color(0x4D0F1115),
    borderL1: Color(0x0A000000),
    borderL2: Color(0x1A000000),
    borderL3: Color(0x1F000000),
    borderL4: Color(0x29000000),
    labelPrimary: DswStaticNeutralBluish.n1000,
    labelSecondary: DswStaticNeutralBluish.n700,
    labelTertiary: DswStaticNeutralBluish.n600,
    labelCaption: DswStaticNeutralBluish.n400,
    labelPrimaryInverted: DswStaticNeutralBluish.n00,
    interactiveHover: Color(0x0F263148),
    interactiveActive: Color(0x1A263148),
    sidebarFill: DswStaticNeutralBluish.n50,
    sidebarItemHover: DswStaticNeutralBluish.n75,
    sidebarItemActive: DswStaticNeutralBluish.n100,
    buttonElevatedFill: DswStaticNeutralBluish.n00,
    buttonFloatingHover: DswStaticNeutralBluish.n75,
    buttonPrimaryFill: DswStaticNeutralBluish.n1000,
    buttonPrimaryHover: DswStaticNeutralBluish.n750,
    buttonPrimaryForeground: DswStaticNeutralBluish.n00,
    bubble: DswStaticDeepseek.d50,
    bubbleHighlight: DswStaticDeepseek.d200,
    brandPrimary: DswStaticBlue.b600,
    brandText: DswStaticNeutralBluish.n1000,
    link: DswStaticDeepseek.d500,
    stateBusinessPrimary: DswStaticDeepseek.d500,
    buttonInfoFill: DswStaticDeepseek.d500,
    buttonInfoHover: DswStaticDeepseek.d400,
    markdownTag: DswStaticNeutralBluish.n75,
    elevationSoft: _elevationSoftLight,
    elevationPanel: _elevationPanelLight,
    inputMajor: DswStaticNeutralBluish.n00,
    stateError: DswStaticState.red600,
    stateWarn: DswStaticState.amber600,
    stateSuccess: DswStaticState.green500,
    scrollbarThumb: DswStaticNeutral.n200,
    tooltipBg: DswStaticNeutralBluish.n850,
  );

  /// 当前是否为深色。
  final bool isDark;

  /// 页面底色 `--dsw-alias-bg-base`。
  final Color bgBase;

  /// 抬升层 1 `--dsw-alias-bg-layer-1`。
  final Color bgLayer1;

  /// 抬升层 2 `--dsw-alias-bg-layer-2`。
  final Color bgLayer2;

  /// 抬升层 3 `--dsw-alias-bg-layer-3`（菜单）。
  final Color bgLayer3;

  /// 遮罩 `--dsw-alias-bg-mask-1`。
  final Color overlay;

  /// 最弱分隔线 `--dsw-alias-border-l1`。
  final Color borderL1;

  /// 弱分隔线 `--dsw-alias-border-l2`。
  final Color borderL2;

  /// 常规分隔线 `--dsw-alias-border-l3`。
  final Color borderL3;

  /// 强调分隔线 `--dsw-alias-border-l4`。
  final Color borderL4;

  /// 主文字 `--dsw-alias-label-primary`。
  final Color labelPrimary;

  /// 次级文字 `--dsw-alias-label-secondary`。
  final Color labelSecondary;

  /// 三级文字 `--dsw-alias-label-tertiary`。
  final Color labelTertiary;

  /// 说明文字 `--dsw-alias-label-caption`。
  final Color labelCaption;

  /// 反色文字 `--dsw-alias-label-primary-inverted`。
  final Color labelPrimaryInverted;

  /// 悬停底色 `--dsw-alias-interactive-bg-hover`。
  final Color interactiveHover;

  /// 选中底色 `--dsw-alias-interactive-bg-active`。
  final Color interactiveActive;

  /// 侧边栏填充 `--dsw-specific-sidebar-fill`。
  final Color sidebarFill;

  /// 侧边栏条目悬停 `--dsw-specific-sidebar-nav-item-hover`。
  final Color sidebarItemHover;

  /// 侧边栏条目选中 `--dsw-specific-sidebar-nav-item-active`。
  final Color sidebarItemActive;

  /// 浮起按钮填充 `--dsw-alias-button-elevated-fill`。
  final Color buttonElevatedFill;

  /// 浮起按钮悬停 `--dsw-alias-button-floating-hover`。
  final Color buttonFloatingHover;

  /// 主按钮填充 `--dsw-alias-button-primary-fill`。
  final Color buttonPrimaryFill;

  /// 主按钮悬停 `--dsw-alias-button-primary-hover`。
  final Color buttonPrimaryHover;

  /// 主按钮前景。
  final Color buttonPrimaryForeground;

  /// 输入框填充 `--dsw-specific-input-major`。
  final Color inputMajor;

  /// 消息气泡 `--dsw-specific-bubble`。
  final Color bubble;

  /// 气泡强调 `--dsw-specific-bubble-highlight`。
  final Color bubbleHighlight;

  /// 品牌色 `--dsw-alias-brand-primary`。
  final Color brandPrimary;

  /// 品牌文字 `--dsw-alias-brand-text`。
  final Color brandText;

  /// 链接色 `--dsw-alias-link`。
  final Color link;

  /// 业务强调色 `--dsw-alias-state-business-primary`。
  ///
  /// 这是 DSH 界面里真正的「蓝色强调」，注意它**不是** `brandPrimary`
  /// （后者在这套令牌里是墨色 ink，只用于文字与主按钮）。
  final Color stateBusinessPrimary;

  /// 发送按钮填充 `--dsw-alias-button-info-fill`。
  final Color buttonInfoFill;

  /// 发送按钮悬停 `--dsw-alias-button-info-hover`。
  final Color buttonInfoHover;

  /// 选中态标签底色 `--dsw-alias-markdown-tag`（右侧栏激活页签用它）。
  final Color markdownTag;

  /// 轻量投影 `--dsw-elevation-soft`。
  final List<BoxShadow> elevationSoft;

  /// 面板投影 `--dsw-elevation-panel`。
  final List<BoxShadow> elevationPanel;

  /// 错误色 `--dsw-alias-state-error-primary`。
  final Color stateError;

  /// 警告色 `--dsw-alias-state-warn-primary`。
  final Color stateWarn;

  /// 成功色 `--dsw-alias-state-success-primary`。
  final Color stateSuccess;

  /// 滚动条滑块 `--dsw-alias-scrollbar-bg-l2`。
  final Color scrollbarThumb;

  /// 气泡提示底色 `--dsw-alias-tooltip-bg`。
  final Color tooltipBg;

  @override
  DswColors copyWith({
    bool? isDark,
    Color? bgBase,
    Color? bgLayer1,
    Color? bgLayer2,
    Color? bgLayer3,
    Color? overlay,
    Color? borderL1,
    Color? borderL2,
    Color? borderL3,
    Color? borderL4,
    Color? labelPrimary,
    Color? labelSecondary,
    Color? labelTertiary,
    Color? labelCaption,
    Color? labelPrimaryInverted,
    Color? interactiveHover,
    Color? interactiveActive,
    Color? sidebarFill,
    Color? sidebarItemHover,
    Color? sidebarItemActive,
    Color? buttonElevatedFill,
    Color? buttonFloatingHover,
    Color? buttonPrimaryFill,
    Color? buttonPrimaryHover,
    Color? buttonPrimaryForeground,
    Color? bubble,
    Color? bubbleHighlight,
    Color? brandPrimary,
    Color? brandText,
    Color? link,
    Color? stateBusinessPrimary,
    Color? buttonInfoFill,
    Color? buttonInfoHover,
    Color? markdownTag,
    List<BoxShadow>? elevationSoft,
    List<BoxShadow>? elevationPanel,
    Color? inputMajor,
    Color? stateError,
    Color? stateWarn,
    Color? stateSuccess,
    Color? scrollbarThumb,
    Color? tooltipBg,
  }) {
    return DswColors(
      isDark: isDark ?? this.isDark,
      bgBase: bgBase ?? this.bgBase,
      bgLayer1: bgLayer1 ?? this.bgLayer1,
      bgLayer2: bgLayer2 ?? this.bgLayer2,
      bgLayer3: bgLayer3 ?? this.bgLayer3,
      overlay: overlay ?? this.overlay,
      borderL1: borderL1 ?? this.borderL1,
      borderL2: borderL2 ?? this.borderL2,
      borderL3: borderL3 ?? this.borderL3,
      borderL4: borderL4 ?? this.borderL4,
      labelPrimary: labelPrimary ?? this.labelPrimary,
      labelSecondary: labelSecondary ?? this.labelSecondary,
      labelTertiary: labelTertiary ?? this.labelTertiary,
      labelCaption: labelCaption ?? this.labelCaption,
      labelPrimaryInverted: labelPrimaryInverted ?? this.labelPrimaryInverted,
      interactiveHover: interactiveHover ?? this.interactiveHover,
      interactiveActive: interactiveActive ?? this.interactiveActive,
      sidebarFill: sidebarFill ?? this.sidebarFill,
      sidebarItemHover: sidebarItemHover ?? this.sidebarItemHover,
      sidebarItemActive: sidebarItemActive ?? this.sidebarItemActive,
      buttonElevatedFill: buttonElevatedFill ?? this.buttonElevatedFill,
      buttonFloatingHover: buttonFloatingHover ?? this.buttonFloatingHover,
      buttonPrimaryFill: buttonPrimaryFill ?? this.buttonPrimaryFill,
      buttonPrimaryHover: buttonPrimaryHover ?? this.buttonPrimaryHover,
      buttonPrimaryForeground:
          buttonPrimaryForeground ?? this.buttonPrimaryForeground,
      bubble: bubble ?? this.bubble,
      bubbleHighlight: bubbleHighlight ?? this.bubbleHighlight,
      brandPrimary: brandPrimary ?? this.brandPrimary,
      brandText: brandText ?? this.brandText,
      link: link ?? this.link,
      stateBusinessPrimary: stateBusinessPrimary ?? this.stateBusinessPrimary,
      buttonInfoFill: buttonInfoFill ?? this.buttonInfoFill,
      buttonInfoHover: buttonInfoHover ?? this.buttonInfoHover,
      markdownTag: markdownTag ?? this.markdownTag,
      elevationSoft: elevationSoft ?? this.elevationSoft,
      elevationPanel: elevationPanel ?? this.elevationPanel,
      inputMajor: inputMajor ?? this.inputMajor,
      stateError: stateError ?? this.stateError,
      stateWarn: stateWarn ?? this.stateWarn,
      stateSuccess: stateSuccess ?? this.stateSuccess,
      scrollbarThumb: scrollbarThumb ?? this.scrollbarThumb,
      tooltipBg: tooltipBg ?? this.tooltipBg,
    );
  }

  @override
  DswColors lerp(covariant DswColors? other, double t) {
    if (other == null) {
      return this;
    }
    return DswColors(
      isDark: t < 0.5 ? isDark : other.isDark,
      bgBase: Color.lerp(bgBase, other.bgBase, t)!,
      bgLayer1: Color.lerp(bgLayer1, other.bgLayer1, t)!,
      bgLayer2: Color.lerp(bgLayer2, other.bgLayer2, t)!,
      bgLayer3: Color.lerp(bgLayer3, other.bgLayer3, t)!,
      overlay: Color.lerp(overlay, other.overlay, t)!,
      borderL1: Color.lerp(borderL1, other.borderL1, t)!,
      borderL2: Color.lerp(borderL2, other.borderL2, t)!,
      borderL3: Color.lerp(borderL3, other.borderL3, t)!,
      borderL4: Color.lerp(borderL4, other.borderL4, t)!,
      labelPrimary: Color.lerp(labelPrimary, other.labelPrimary, t)!,
      labelSecondary: Color.lerp(labelSecondary, other.labelSecondary, t)!,
      labelTertiary: Color.lerp(labelTertiary, other.labelTertiary, t)!,
      labelCaption: Color.lerp(labelCaption, other.labelCaption, t)!,
      labelPrimaryInverted:
          Color.lerp(labelPrimaryInverted, other.labelPrimaryInverted, t)!,
      interactiveHover: Color.lerp(interactiveHover, other.interactiveHover, t)!,
      interactiveActive:
          Color.lerp(interactiveActive, other.interactiveActive, t)!,
      sidebarFill: Color.lerp(sidebarFill, other.sidebarFill, t)!,
      sidebarItemHover:
          Color.lerp(sidebarItemHover, other.sidebarItemHover, t)!,
      sidebarItemActive:
          Color.lerp(sidebarItemActive, other.sidebarItemActive, t)!,
      buttonElevatedFill:
          Color.lerp(buttonElevatedFill, other.buttonElevatedFill, t)!,
      buttonFloatingHover:
          Color.lerp(buttonFloatingHover, other.buttonFloatingHover, t)!,
      buttonPrimaryFill:
          Color.lerp(buttonPrimaryFill, other.buttonPrimaryFill, t)!,
      buttonPrimaryHover:
          Color.lerp(buttonPrimaryHover, other.buttonPrimaryHover, t)!,
      buttonPrimaryForeground:
          Color.lerp(buttonPrimaryForeground, other.buttonPrimaryForeground, t)!,
      bubble: Color.lerp(bubble, other.bubble, t)!,
      bubbleHighlight: Color.lerp(bubbleHighlight, other.bubbleHighlight, t)!,
      brandPrimary: Color.lerp(brandPrimary, other.brandPrimary, t)!,
      brandText: Color.lerp(brandText, other.brandText, t)!,
      link: Color.lerp(link, other.link, t)!,
      stateBusinessPrimary: Color.lerp(
        stateBusinessPrimary,
        other.stateBusinessPrimary,
        t,
      )!,
      buttonInfoFill: Color.lerp(buttonInfoFill, other.buttonInfoFill, t)!,
      buttonInfoHover: Color.lerp(buttonInfoHover, other.buttonInfoHover, t)!,
      markdownTag: Color.lerp(markdownTag, other.markdownTag, t)!,
      // 阴影不参与插值：两套主题的投影差别很小，直接取目标值即可。
      elevationSoft: t < 0.5 ? elevationSoft : other.elevationSoft,
      elevationPanel: t < 0.5 ? elevationPanel : other.elevationPanel,
      inputMajor: Color.lerp(inputMajor, other.inputMajor, t)!,
      stateError: Color.lerp(stateError, other.stateError, t)!,
      stateWarn: Color.lerp(stateWarn, other.stateWarn, t)!,
      stateSuccess: Color.lerp(stateSuccess, other.stateSuccess, t)!,
      scrollbarThumb: Color.lerp(scrollbarThumb, other.scrollbarThumb, t)!,
      tooltipBg: Color.lerp(tooltipBg, other.tooltipBg, t)!,
    );
  }
}

/// 便捷取用 [DswColors] 的扩展。
extension DswColorsContext on BuildContext {
  /// 当前主题的语义色。
  ///
  /// 只应在已经挂上 [DswColors] 扩展的子树中调用；见 `app_theme.dart`。
  DswColors get dsw => Theme.of(this).extension<DswColors>() ?? DswColors.dark;
}
