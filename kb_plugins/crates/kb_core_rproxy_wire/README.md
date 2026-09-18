# kb_core_rproxy_wire

`kb_core_rproxy` 的 **TCP 帧格式**：网关（服务端）与任何远程客户端共用同一份编解码。

```text
[u32 BE 长度][1 字节种类][postcard 载荷]
     长度 = 种类 + 载荷的字节数
     种类：0 = 请求（上行）；1 = 应答（下行）；2 = 事件（下行，预留）
     载荷：RequestEnvelope / ReplyEnvelope / Event 的 postcard 编码
```

## 为什么单独一个 crate

这段格式原来长在 `kb_core_rproxy` 的 `src/frame_.rs` 里，而那个 crate 只有
`[[bin]]`、没有 lib target。客户端要照着它实现时只能**抄一份**——线格式两处各写
一遍，改一处忘一处就是"跑起来才发现对端解不开"。

抽出来之后：

| 谁 | 用哪些 |
| :--- | :--- |
| `kb_core_rproxy`（服务端） | `decode_request` + `encode_reply` / `encode_event` |
| 远程客户端（`kb_client_conn_mgr::TcpClient`） | `encode_request` + `decode_frame` |

网关那边另有几行 compio 的写包装（把这里的字节写进 socket），IO 与格式因此分开。

## 只有纯编解码

不碰 socket、不依赖任何异步运行时。所以"网关用 compio、客户端用 `std::net`"
这件事不影响这里。

## 为什么解码是"缓冲区驱动"

一帧可能分几次到达（TCP 是字节流），所以 `decode_frame` / `decode_request` 不直接
读 socket，而是先看缓冲区里够不够一帧：不够就返回 `Ok(None)` 让调用方继续喂。

**解码按种类分派**：下行既可能是应答、也可能是事件，调用方**不能**假定"读到的
下一帧就是我的应答"。

单帧载荷上限 8 MiB（`MAX_FRAME_BYTES`）：对端可以谎报长度，没有上限的话一个字节
的头部就能让我们分配几个 G。

## 验证

```bash
cargo test -p kb_core_rproxy_wire
```

6 个测试：分片到达、连续帧、非法长度（超限 / 为零）、非法种类、三种种类分派、
未知种类值被拒、应答的 `request_id` 往返。

## 相关文档

- [`kb_core_rproxy`](../kb_core_rproxy/README.md)：服务端；
- [`kb_client_conn_mgr`](../../../kb_clients/crates/kb_client_conn_mgr/README.md)：客户端。
