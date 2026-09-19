import Cocoa
import FlutterMacOS

class MainFlutterWindow: NSWindow {
  override func awakeFromNib() {
    let flutterViewController = FlutterViewController()
    let windowFrame = self.frame
    self.contentViewController = flutterViewController
    self.setFrame(windowFrame, display: true)

    // 最小窗口尺寸：与 Dart 侧 `DswLayout.minWindowWidth` / `minWindowHeight` 一致
    // （折叠轨道 56 + 中间区最小 400 + 右侧栏最小 300 ≈ 800 宽；列头 + 输入区 ≈ 520 高）。
    // 改动时三端一并改。
    self.minSize = NSSize(width: 800, height: 520)

    RegisterGeneratedPlugins(registry: flutterViewController)

    super.awakeFromNib()
  }
}
