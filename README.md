# vless

单目标 VLESS + WebSocket 服务端，用 Rust 重写，**零第三方依赖**。

只做一件事：VLESS 入站 → direct 出站。没有规则引擎、没有 DNS 模块、没有 TLS 栈、没有 async 运行时。

## 数字

| | Go 版 vm2 (VMess) | **vless (Rust)** |
|---|---|---|
| 二进制（静态 musl） | 2.59 MB | **521 KB** |
| 常驻内存 | 7 MB | **0.5 MB** |
| 第三方依赖 | 0 | **0** |
| 吞吐（同客户端、同环回） | 255–295 MB/s | **480–530 MB/s** |

环回直连天花板 855 MB/s，所以代理跑在直连的 50% 左右——这是用户态代理的正常水平，因为每个字节要跨两次内核边界，而 WebSocket 必须重新分帧，`splice`/`sendfile` 这类零拷贝全都用不了。

## 为什么这么小

Rust 没有 GC 运行时，标准库静态链接后下限只有 200–300 KB。剩下的全是有用代码：

- `ws.rs` —— RFC 6455 子集：握手、帧编解码、分片重组、ping/pong/close
- `vless.rs` —— VLESS 头解析，60 行左右的字节搬运
- `sha1.rs` / `base64.rs` —— 手写，只为握手算 `Sec-WebSocket-Accept`
- `json.rs` —— 手写解析器，只为读启动配置
- `relay.rs` / `dial.rs` —— 双向搬运与出站连接

VLESS 本身协议极简：没有 KDF、没有 AEAD 头认证、没有分块、没有 padding、没有掩码。对比之下，同一个作者写的 VMess 版本要多 900 行代码，多 2 MB 体积。

## 必须知道的限制

**Xray 内核拒绝 VLESS + 无 TLS + 公网地址。**

Xray 在配置校验阶段就会报错：

```
vless without TLS or other encryption is prohibited
unless the server address is a private IP or domain
```

源码里 `requiresTransportSecurity()` 把公网 IP 和公网域名都判为不安全地址。所以 **v2rayN 默认的 Xray 内核无法使用本服务端**，除非给它配 TLS。

可用的客户端是 sing-box 内核（已实测通过）：

- v2rayN 7.x：设置 → 核心类型 → `sing-box`
- Hiddify、NekoBox、sing-box 官方 GUI

## 安全提醒

**VLESS 本身不加密，UUID 是明文传输的。**

16 字节 UUID 固定出现在连接开头的偏移 1–17 上，任何能看到流量的人都能直接读出来，然后拿去用你的服务器。这一点比 VMess 差——VMess 即便 `security: none`，UUID 也藏在加密的 authID 里。

所以本服务的定位只能是「可信链路上的极简管道」。真要加密，别在应用层做 TLS，直接在网络层铺 WireGuard——内核实现的加密几乎不耗 CPU，而 TLS 会给二进制加 1.5 MB 以上，把体积优势全赔回去。

## 配置

兼容 sing-box 配置子集，可直接复用现有 JSON：

```json
{
  "log": { "disabled": true },
  "inbounds": [
    {
      "type": "vless",
      "listen": "::",
      "listen_port": 28444,
      "users": [{ "id": "0a6d8fef-de3c-48b4-afec-713af83f5227" }],
      "transport": { "type": "ws", "path": "/vl" }
    }
  ],
  "outbounds": [{ "type": "direct", "domain_strategy": "prefer_ipv6" }]
}
```

支持的字段就这些：`listen`、`listen_port`、`users[].id`（也接受 `uuid`）、`transport.path`、`domain_strategy`（`prefer_ipv6` / `prefer_ipv4` / 其他等同系统默认）。其余字段静默忽略。

## 构建

```bash
cargo build --release --target x86_64-unknown-linux-musl
```

产物在 `target/x86_64-unknown-linux-musl/release/vless`，纯静态、无 libc 依赖。

`Cargo.toml` 里的 release profile 已配好：`opt-level = 3`、`lto = true`、`codegen-units = 1`、`panic = "abort"`、`strip = true`。其中 `opt-level` 在 3 和 "z" 之间只差 16 KB，但 3 的吞吐高约 9%。

## 运行

```bash
./vless config.json     # 省略参数则读当前目录的 config.json
```

## 兼容性

已用真实客户端内核端到端验证：

- sing-box 1.14.4，VLESS + WS，经公网 IP 与 127.0.0.1 双向验证
- `prefer_ipv6` 正确生效，域名解析优先取 IPv6
- 4 MB / 32 MB / 256 MB 单流逐字节一致，8 路并发正常
- 多站点访问（GitHub、Google、Cloudflare）

**不支持**：TLS、UDP 转发、VMess/mux、alterId 旧协议。

## 已知取舍

- UDP 未实现（VLESS 的 UDP 走独立帧格式，本场景用不上）
- 只支持一个入站、一个 direct 出站
- 不校验 `Sec-WebSocket-Protocol`、不做 `max_early_data`
- 每连接两个线程（读、写各一），阻塞 IO。几百连接量级完全够用；不引入 async 运行时正是体积优势的来源

## License

MIT
