// 三栏几何求解的移植测试。
//
// 期望值直接取自 DSH v0.1.5-rc 的
// `packages/client/ui-layout/tests/columns.client.spec.ts`：只要这组数字对得上，
// 就说明 [ColumnGeometry.solve] 与 DSH 的 `computeColumns` 是同一套规则。

import 'package:flutter_test/flutter_test.dart';
import 'package:kb_admin_desktop/src/theme/dsw_tokens.dart';

void main() {
  group('clampSidebar', () {
    /// 测试侧边栏宽度夹取与取整。
    /// - 手段：分别传入低于下界、高于上界、带小数的宽度。
    /// - 判断：返回值分别等于 264、420、取整后的中间值。
    test('夹到 [264, 420] 并取整', () {
      expect(DswLayout.clampSidebar(100), 264);
      expect(DswLayout.clampSidebar(9999), 420);
      expect(DswLayout.clampSidebar(300.4), 300);
    });

    /// 测试极窄视口下右侧栏宽度夹取不会抛断言。
    /// - 手段：视口小到 `viewport * 0.7 < 300`，再求右侧栏宽度。
    /// - 判断：返回上界（视口 × 0.7），且调用过程不抛异常。
    test('极窄视口下夹到视口比例上界', () {
      expect(DswLayout.clampRightbar(9999, 200), 140);
      expect(DswLayout.clampRightbar(9999, 0), 0);
    });
  });

  group('ColumnGeometry.solve', () {
    /// 测试中间区有足够空间时两侧各得其所。
    /// - 手段：视口 1920、侧边栏偏好 280、右侧栏偏好 864。
    /// - 判断：三栏分别为 280 / 776 / 864。
    test('空间充足时两侧都拿到偏好值', () {
      expect(
        ColumnGeometry.solve(viewport: 1920, sidebar: 280, rightbar: 864),
        const ColumnGeometry(sidebar: 280, center: 776, rightbar: 864),
      );
    });

    /// 测试两侧都关闭时只剩左侧图标轨道。
    /// - 手段：侧边栏与右侧栏偏好都为 0。
    /// - 判断：侧边栏为 56，中间区吃掉其余宽度，右侧栏为 0。
    test('两侧都关闭时只留左侧轨道', () {
      expect(
        ColumnGeometry.solve(viewport: 1920, sidebar: 0, rightbar: 0),
        const ColumnGeometry(sidebar: 56, center: 1864, rightbar: 0),
      );
    });

    /// 测试侧边栏夹取与右侧栏 70% 上限。
    /// - 手段：分别传入超大偏好与极小偏好。
    /// - 判断：结果与 DSH 参考值一致。
    test('夹取侧边栏并限制右侧栏不超过视口 70%', () {
      expect(
        ColumnGeometry.solve(viewport: 3000, sidebar: 9999, rightbar: 9999),
        const ColumnGeometry(sidebar: 420, center: 480, rightbar: 2100),
      );
      expect(
        ColumnGeometry.solve(viewport: 1920, sidebar: 1, rightbar: 1),
        const ColumnGeometry(sidebar: 264, center: 1356, rightbar: 300),
      );
    });

    /// 测试各种视口下「右侧栏先压缩、再丢轨道」的降级顺序。
    /// - 手段：逐条喂入 DSH 测试里的视口 / 侧边栏偏好，右侧栏偏好固定 864。
    /// - 判断：右侧栏与中间区宽度与 DSH 参考表逐项相等。
    test('按 DSH 参考表压缩右侧栏、再让出轨道', () {
      const List<(double, double, double, double)> cases =
          <(double, double, double, double)>[
            (1300, 280, 620, 400),
            (1100, 280, 420, 400),
            (1120, 420, 300, 400),
            (1119, 420, 0, 699),
            (1024, 420, 0, 604),
            (756, 0, 300, 400),
            (755, 0, 0, 699),
            (455, 0, 0, 399),
            (20, 0, 0, 0),
          ];

      for (final (double viewport, double sidebar, double rightbar, double center)
          in cases) {
        final ColumnGeometry solved = ColumnGeometry.solve(
          viewport: viewport,
          sidebar: sidebar,
          rightbar: 864,
        );
        expect(solved.sidebar, sidebar == 0 ? 56 : sidebar);
        expect(solved.rightbar, rightbar, reason: '视口 $viewport');
        expect(solved.center, center, reason: '视口 $viewport');
      }
    });

    /// 测试宽侧边栏不会为了保留右侧栏而让步。
    /// - 手段：视口 1024、侧边栏偏好 420、右侧栏偏好 500。
    /// - 判断：侧边栏保持 420，右侧栏轨道为 0。
    test('宽侧边栏不让步于右侧栏', () {
      expect(
        ColumnGeometry.solve(viewport: 1024, sidebar: 420, rightbar: 500),
        const ColumnGeometry(sidebar: 420, center: 604, rightbar: 0),
      );
    });

    /// 测试视口变宽后被压缩的右侧栏恢复偏好值。
    /// - 手段：同一偏好 864，在 1100 与 1920 两个视口下求解。
    /// - 判断：分别得到 420 与 864。
    test('视口变宽时恢复仍然打开的偏好', () {
      expect(
        ColumnGeometry.solve(viewport: 1100, sidebar: 280, rightbar: 864).rightbar,
        420,
      );
      expect(
        ColumnGeometry.solve(viewport: 1920, sidebar: 280, rightbar: 864).rightbar,
        864,
      );
    });

    /// 测试关闭的右侧栏轨道不会因为视口变宽而自己打开。
    /// - 手段：右侧栏偏好为 0，分别在 755 与 1920 两个视口下求解。
    /// - 判断：两次的 rightbar 都是 0。
    test('关闭的右侧栏轨道保持关闭', () {
      expect(
        ColumnGeometry.solve(viewport: 755, sidebar: 0, rightbar: 0).rightbar,
        0,
      );
      expect(
        ColumnGeometry.solve(viewport: 1920, sidebar: 0, rightbar: 0).rightbar,
        0,
      );
    });
  });
}
