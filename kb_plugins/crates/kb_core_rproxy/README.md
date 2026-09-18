# kb_core_rproxy

`kb_core` 的**局域网应用层网关**。

```text
远程客户端 ──TCP──► kb_core_rproxy ──IPC──► kb_core（本机子进程）
                    （本进程）
```

它做的事只有三件：

1. **启动**一个 `kb_core` 子进程（`--handshake-prompt=stdio`），读它公布的那一行
   通知，**记录 IPC 端点文件名**——这是**系统层握手**。这一段由
   [`kb_core_starter`](../../../kb_svc/crates/kb_core_starter/) 提供
   （它原来长在本 crate 里，现在作为服务侧基础设施独立成可复用的 crate：
   桌面客户端要起本机 `kb_core` 时用的是同一段代码）；
2. 连上 `kb_core`（`kb_svc_servo_ipc::Client`）；
3. 监听 TCP，把远程客户端发来的 `RequestEnvelope` **原样**转发过去，把
   `ReplyEnvelope` 原样送回。它**不解释任何业务**——不懂工作区，也不懂会话。

## ⚠️ 没有鉴权、没有 TLS

它存在的目的是**局域网跨机测试**，所以缺省监听 `0.0.0.0:8788`，
**刻意不只绑 `127.0.0.1`**。代价很直接：

> 任何能访问到该端口的人都能读写知识库。

这条不是"以后顺手加"的东西——**在受信网络之外使用之前必须先做鉴权**。
只想给本机用时显式传 `--listen 127.0.0.1:8788`。

## 跑起来

```bash
# 网关（它会自己去启动 kb_core）
cargo run -p kb_core_rproxy -- \
  --listen 0.0.0.0:8788 \
  --runtime-dir /tmp/kb-rp/run \
  --storage-dir /tmp/kb-rp/data

# 另一个终端：一个最小的"远程客户端"（参考实现 + 验证手段）
cargo run -p kb_core_rproxy --example probe -- 127.0.0.1:8788
```

`probe` 的输出：

```text
已连上 127.0.0.1:8788
握手 OK：服务端版本 0.1.0，协议 v1
已建立工作区: probe 建的工作区 (w-9704f49f6af2413ab0bf59ff6993976c)
共 1 个工作区
  w-9704f49f6af2413ab0bf59ff6993976c	probe 建的工作区	/tmp/probe
```

## 两层握手，各管一段

| 层面 | 解决什么 | 在本 crate 里的位置 | 消息由谁定 |
| :--- | :--- | :--- | :--- |
| **系统层** | 找得到、连得上 | `kb_core_starter`：启动 `kb_core`，读它 stdout 上的那一行通知（异步、可取消） | **`abs_kb_svc` 的 `IpcReadyNotice`（协议 v1）**；怎么把消息送到由传输实现决定 |
| **应用层** | 谈得成 | `main.rs`：**等真有远程客户端连上来**才发起 `Request::Hello` | `abs_kb_svc` 的协议 |

- stdio 通知**默认关闭**：只有 `--handshake-prompt=stdio` 时 `kb_core` 才会往
  stdout 打那一行；交互式跑 `kb_core` 时 stdout 保持干净；
- 网关**不在启动时**做应用层握手——启动阶段只保证"找得到"；
- 远程客户端自己再发一次 `Hello` 也没问题：它会照常被转发，服务端会再答一次。

## TCP 帧

```text
[u32 BE 长度][1 字节种类][postcard 载荷]
     长度 = 种类 + 载荷的字节数
     种类：0 = 请求（上行）；1 = 应答（下行）；2 = 事件（下行，预留）
     载荷：RequestEnvelope / ReplyEnvelope / Event 的 postcard 编码
```

- 用 postcard 是因为它正是 ipc-channel 内部的编解码器，协议类型已有
  "用 postcard 真解码"的往返测试；
- 单帧载荷上限 8 MiB（对端可以谎报长度，必须有上限）；
- **协议类型一个都没改**——换的是搬运方式。

## 当前的能力边界

| 限制 | 说明 |
| :--- | :--- |
| 一次只服务一个远程客户端 | 上一个断开才接受下一个；并发要等 `kb_core` 那侧也能并发服务连接 |
| 事件还没转发 | 服务端目前不推事件（生成相关域未落地）；帧格式已经留好种类位 |
| 背压只做在上行 | 上行走 `buffex::circular_buff` 的有界环（满则读侧停止取数据，把背压交给 TCP 窗口）；下行是严格的一问一答，攒不出队列，暂不做 |
| 无鉴权、无 TLS | 见上 |

## 背压：上行有界环

```text
TCP 读半 ──► buffex::circular_buff（64 KiB 有界）──► 帧解码 ──► IPC ──► 应答 ──► TCP 写半
   feed_uplink                                    forward_uplink
```

环满了写侧就等，于是**读侧不再从 socket 取数据**，TCP 窗口自然关上——背压交给
传输层，而不是让内存无声涨上去。

上行做环、下行不做的理由：同一时刻只有一个请求在途，应答是**一问一答**产出的，
攒不出队列；而上行是"对端可以拼命塞"的方向。

> ⚠️ 这个库有两个反直觉的停等语义（都实测过，写在 `ring_` 的模块文档里）：
> 只 drop 写端**不等于** EOF（必须显式 `close()`）；而且**满环上已停等的写者，
> 不会因为读端关闭而被释放**。所以本 crate 的收尾约定是：转发侧无论因为什么收工，
> 都先发一个中止信号让读侧任务整体被丢弃，而不是指望环形缓冲自己解开。

## 相关文档

- `dev-notes/kb_core_rproxy-20260917-1749.md`：可行性研究与已拍板的决定；
- `abs_kb_svc/src/v1/desktop/handshake_.rs`：两个层面握手的完整说明；
- `kb_svc/crates/kb_svc_servo_ipc/README`（crate 文档）：本机 IPC 的引导机制。
