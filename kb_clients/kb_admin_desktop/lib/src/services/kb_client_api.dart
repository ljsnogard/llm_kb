// `kb_core` 连接的 Dart 侧门面。
//
// 界面**不直接**调用生成的 FRB 代码，而是经过这里的 [KbClientApi]：
//
// - widget 测试可以塞一个假的实现，完全不碰原生库（`RustLib.init()`）；
// - FRB 的接口形状若要调整，改动收在 [FrbKbClientApi] 一个类里。
//
// `lib/src/rust/` 下的东西都是生成的，别手改；重新生成的办法见
// 工程根目录的 `README.md`「重新生成 FRB 绑定」一节。

import '../rust/api/kb.dart' as frb;

/// 一条连接方式的扁平视图（就是 Rust 侧 `ConnectionView` 的镜像）。
typedef ConnectionView = frb.ConnectionView;

/// 配置文件快照。
typedef ConfigView = frb.ConfigView;

/// 一次连接的尝试结果。
typedef ConnectReport = frb.ConnectReport;

/// 一个工作区。
typedef WorkspaceView = frb.WorkspaceView;

/// 一个会话（只有摘要）。
typedef SessionView = frb.SessionView;

/// 列工作区 / 列会话的结果。
typedef WorkspacesReport = frb.WorkspacesReport;
typedef SessionsReport = frb.SessionsReport;

/// 新建工作区 / 会话的结果，以及"删除"这类没有返回值的操作的结果。
typedef WorkspaceReport = frb.WorkspaceReport;
typedef SessionReport = frb.SessionReport;
typedef OpReport = frb.OpReport;

/// 会话正文：摘要 + 全部消息（`TurnView` 是一条消息）。
typedef SessionDetailReport = frb.SessionDetailReport;
typedef TurnView = frb.TurnView;

/// 连接相关的原生接口。
///
/// 每个方法都对应 `rust/src/api/kb.rs` 里的一个函数；原生侧写成同步函数 +
/// `block_on`，所以 Dart 侧统一是 `Future`（FRB 把它放到自己的线程池上跑）。
abstract class KbClientApi {
  /// 配置文件的路径。
  Future<String> configFilePath();

  /// 读配置；`exists == false` 表示首次运行。
  Future<ConfigView> loadConfig();

  /// 写配置；返回空串表示成功。
  Future<String> saveConfig({
    required String defaultName,
    required List<ConnectionView> connections,
  });

  /// 三种连接方式的 `kind` 取值。
  Future<List<String>> connectionKinds();

  /// 某种连接方式的说明。
  Future<String> connectionKindDescription(String kind);

  /// 首次运行时预填的本机连接方式。
  Future<ConnectionView> suggestLocalConnection(String name);

  /// 连上 `kb_core`（系统层 + 应用层握手）。
  Future<ConnectReport> connect(ConnectionView profile);

  /// 断开（本机启动方式会结束那个 `kb_core` 子进程）。
  Future<void> disconnect();

  /// 列出当前连接下的工作区。
  Future<WorkspacesReport> listWorkspaces();

  /// 列出某个工作区下的会话。
  Future<SessionsReport> listSessions(String workspaceId);

  /// 在 `kb_core` 上新建一个工作区。
  ///
  /// `path` 是 **`kb_core` 所在主机上**的目录：客户端这边不做任何本地文件系统
  /// 操作，只把「名字 + 路径」提交给服务端。
  Future<WorkspaceReport> addWorkspace({
    required String name,
    required String path,
  });

  /// 删除一个工作区（服务端会级联删除它名下的会话）。
  Future<OpReport> removeWorkspace(String workspaceId);

  /// 在某个工作区下新建一个会话。
  Future<SessionReport> createSession({
    required String workspaceId,
    required String title,
  });

  /// 删除一个会话。
  Future<OpReport> removeSession({
    required String workspaceId,
    required String sessionId,
  });

  /// 读取一个会话的完整内容（摘要 + 全部消息）。
  Future<SessionDetailReport> getSession({
    required String workspaceId,
    required String sessionId,
  });

  /// 就某个会话提问，回来后拿到**提问之后**的会话内容。
  ///
  /// `kb_core` 现在跑的是临时模拟的 LLM（把问题逆序输出），所以返回值里已经
  /// 带着刚产生的两条消息，不需要再调 [getSession]。
  Future<SessionDetailReport> ask({
    required String workspaceId,
    required String sessionId,
    required String turnId,
    required String question,
  });
}

/// 真实现：转发给生成的原生绑定。
class FrbKbClientApi implements KbClientApi {
  /// 构造真实现。
  const FrbKbClientApi();

  @override
  Future<String> configFilePath() => frb.configFilePath();

  @override
  Future<ConfigView> loadConfig() => frb.loadConfig();

  @override
  Future<String> saveConfig({
    required String defaultName,
    required List<ConnectionView> connections,
  }) => frb.saveConfig(defaultName: defaultName, connections: connections);

  @override
  Future<List<String>> connectionKinds() => frb.connectionKinds();

  @override
  Future<String> connectionKindDescription(String kind) =>
      frb.connectionKindDescription(kind: kind);

  @override
  Future<ConnectionView> suggestLocalConnection(String name) =>
      frb.suggestedLocalConnection(name: name);

  @override
  Future<ConnectReport> connect(ConnectionView profile) =>
      frb.connectTo(profile: profile);

  @override
  Future<void> disconnect() => frb.disconnect();

  @override
  Future<WorkspacesReport> listWorkspaces() => frb.listWorkspaces();

  @override
  Future<SessionsReport> listSessions(String workspaceId) =>
      frb.listSessions(workspaceId: workspaceId);

  @override
  Future<WorkspaceReport> addWorkspace({
    required String name,
    required String path,
  }) => frb.addWorkspace(name: name, path: path);

  @override
  Future<OpReport> removeWorkspace(String workspaceId) =>
      frb.removeWorkspace(workspaceId: workspaceId);

  @override
  Future<SessionReport> createSession({
    required String workspaceId,
    required String title,
  }) => frb.createSession(workspaceId: workspaceId, title: title);

  @override
  Future<OpReport> removeSession({
    required String workspaceId,
    required String sessionId,
  }) => frb.removeSession(workspaceId: workspaceId, sessionId: sessionId);

  @override
  Future<SessionDetailReport> getSession({
    required String workspaceId,
    required String sessionId,
  }) => frb.getSession(workspaceId: workspaceId, sessionId: sessionId);

  @override
  Future<SessionDetailReport> ask({
    required String workspaceId,
    required String sessionId,
    required String turnId,
    required String question,
  }) => frb.ask(
    workspaceId: workspaceId,
    sessionId: sessionId,
    turnId: turnId,
    question: question,
  );
}
