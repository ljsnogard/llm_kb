# kb_client_conn_mgr

`kb_admin_desktop` 的**连接管理器**：按一条连接方式把客户端接上 `kb_core`，
然后暴露一组窄接口（工作区 / 会话的增删查）。

```text
读配置（kb_client_config）
   └─ 首次运行：界面问用户怎么连 → 写配置
连接（本 crate）
   ├─ ① 系统层握手：起本机 kb_core / 附着到本机 kb_core / 连远程网关
   └─ ② 应用层握手：Request::Hello → Reply::Hello
工作区
   ├─ list_workspaces()
   ├─ add_workspace(request)     目录是 kb_core 所在主机上的路径
   ├─ remove_workspace(id)       服务端级联删除它名下的会话
   └─ rename_workspace(id, name) 只改展示名，磁盘目录不动
会话
   ├─ list_sessions(workspace_id)
   ├─ create_session(request)    request.turns 可以带"第一个问题"（草稿流程用）
   ├─ remove_session(workspace_id, session_id)
   └─ rename_session(workspace_id, session_id, title)
正文与生成
   ├─ get_session(workspace_id, session_id)   完整正文
   └─ ask(request)                            同步一问一答（kb_core 里是临时模拟的 LLM）
```

## 三种连接方式

| `kind` | 系统层怎么走 | 说明 |
| :--- | :--- | :--- |
| `local-launch` | `kb_core_starter::start`（起子进程 + 读就绪通知），再连 IPC | 子进程由 `KbClient` 持有，**它被丢弃时子进程被结束** |
| `local-attach` | 直接连已经在跑的本机 IPC | 服务端由别人（手工 / rproxy）拉起时用 |
| `tcp` | 连 `kb_core_rproxy` 的 TCP 端口 | 跨机；**无鉴权、无 TLS，仅受信网络** |

`connect` 把两种握手都走完才算"连上"；任何一步失败都返回 `ClientError`，
并且不会留下半个连接。

## 形状

```rust
let token = TimeoutToken::after(profile.handshake_timeout());
let client = connect(profile).may_cancel_with(token).await?;

let token = TimeoutToken::after(client.request_timeout());
let workspaces = client.list_workspaces().may_cancel_with(token).await?;

// 增删走同一套窄接口；标识一律由 kb_core 分配。
let workspace = client
    .add_workspace(AddWorkspaceRequest {
        local_id: LocalId::generate(),
        name: "笔记".to_string(),
        path: "/srv/notes".to_string(),   // kb_core 主机上的目录
    })
    .may_cancel_with(token)
    .await?;
```

`local_id` 是协议里的簿记字段：线上仍然带着它往返一次，但**这些方法不把它
返回给调用方**——调用方本来就知道自己发的是哪一个，它要的是服务端分配的
`workspace_id` / `session_id`。

- **不挑异步运行时**：所有公开异步入口都由 `gen_mcf2::gen_may_cancel_future`
  展开成「不可取消 / 可取消」两条路径；future 里没有阻塞调用——阻塞的
  `Client::connect`（本机 IPC，含重试）与 `TcpClient::connect`（TCP）都搬到
  **专职线程**上，future 只轮询完成量 + 取消令牌；
- **超时由调用方决定**：本 crate 不自带定时器，只提供
  [`TimeoutToken`](src/timeout_.rs)（"到点就取消"，一条线程 + `oneshot`）。
  等多久是策略，由界面按配置里的时限决定；
- **取消之后子进程会被收掉**：`local-launch` 起出来的 `kb_core` 由
  `kb_core_starter` 的守卫持有，取消、出错、future 被丢弃三条路径都会结束它。

## 远程路径的传输

`TcpClient` 与 `kb_svc_servo_ipc::Client` **刻意同构**：写线程发帧、读线程按
`request_id` 把应答交给等待者、事件帧按种类分派（现在没人订阅，只记日志）。
好处是这一个 crate 不依赖任何异步运行时的 `net` 模块，也不把 `ipc-channel`
拖进远程路径。

帧格式本身在 [`kb_core_rproxy_wire`](../../../kb_plugins/crates/kb_core_rproxy_wire/)，
与网关**共用同一份编解码**。

## 它不做什么

- 不读配置文件（那是 `kb_client_config`）；
- 不做界面（FRB 那一层再把结果翻成扁平 DTO）；
- 不实现设置（`TrSettingsService`）与目录（`TrDirectoryService`）——协议那边还没落地；
- **生成只做到"同步一问一答"**（`ask` → 提问之后的 `SessionDetail`）：流式增量与
  取消还没定义，见 `abs_kb_svc_v1_desktop::TrGeneration` 的文档；
- **只做改名，不做换路径**：`rename_workspace` 只改展示名；工作区换目录需要界面与
  目录校验，还没做。会话改名与工作区改名的协议形状见
  [`dev-notes/kb_admin_desktop-20260919-1237.md`](../../../dev-notes/kb_admin_desktop-20260919-1237.md) §1。

## 验证

```bash
cargo test -p kb_client_conn_mgr
```

2 个单元测试（超时令牌）+ 8 个集成测试（假网关：正常往返、工作区 / 会话增删、
改名、读正文与提问、业务错误透传、取消、坏帧判死、不可取消路径）+ 1 个文档测试。

真进程的端到端验证用附带的小 CLI：

```bash
# 本机·启动
cargo run -p kb_client_conn_mgr --example connect -- launch ./target/debug/kb-core /tmp/kb/run /tmp/kb/data
# 本机·附着（先手工起一个 kb-core）
cargo run -p kb_client_conn_mgr --example connect -- attach /tmp/kb/run
# 远程（先起 kb-core-rproxy）
cargo run -p kb_client_conn_mgr --example connect -- tcp 127.0.0.1:8788
```

三种方式都实测跑通过：`local-launch` 起进程 → IPC → 握手 → 列工作区；
`local-attach` 连已在跑的实例；`tcp` 经真 rproxy 列出了一个真工作区与会话。

## 相关文档

- [`kb_client_config`](../kb_client_config/README.md)：连接方式从哪来；
- [`kb_core_starter`](../../../kb_plugins/crates/kb_core_starter/README.md)：系统层握手的本机那一半；
- [`kb_core_rproxy_wire`](../../../kb_plugins/crates/kb_core_rproxy_wire/README.md)：远程那一半的帧格式；
- `dev-notes/kb_admin_desktop-20260918-1034.md`：三种连接方式与配置的决策过程。
