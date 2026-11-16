# bare

一个只做 **VLESS over WebSocket → TCP 直连出站** 的 Rust 服务端。零第三方依赖，不包含规则引擎、异步运行时或 TLS 栈。

适合需要小二进制、功能范围固定，并由外部组件处理 TLS 的部署。**bare 不提供链路加密；不要把示例 UUID 当成密钥使用。**

## 快速开始

需要 Rust 工具链和本机链接器：

```sh
git clone https://github.com/cleeate/bare.git
cd bare
cargo build --release --locked
cp examples/server.json config.json
```

生成自己的 UUID，替换 `config.json` 中的示例值：

```sh
# Linux
cat /proc/sys/kernel/random/uuid
./target/release/bare config.json
```

省略配置路径时，默认读取当前目录的 `config.json`。示例默认监听 `127.0.0.1:28444`，供本地客户端或反向代理访问。需要直接对外监听时，自行改成 `0.0.0.0` 或 `::`，并配置防火墙。

## 配置

完整示例见 [examples/server.json](examples/server.json)。兼容 sing-box 配置中的少量字段，并不实现 sing-box 的全部配置语义。

- `inbounds`：使用第一个匹配的入站。
- `listen`：监听地址，省略时为 `::`。
- `listen_port`：监听端口。
- `users[].id`：UUID，也接受 `users[].uuid`，支持多个用户。
- `transport.type`：使用 `ws`；`transport.path` 是 WebSocket 路径，省略或为空时不限制路径。
出站始终直连，没有可配置的出站字段（包括 `domain_strategy`）。IPv4 / IPv6 的选择与地址顺序完全交给系统（getaddrinfo / gai.conf）：系统按 RFC 3484 排序，`connect` 会依次尝试各地址，某个不可达时自动换下一个。

其他配置字段通常被忽略，包括 `log.disabled`，它不能用来关闭日志。历史兼容逻辑也接受入站 `type: vmess`，但线上协议始终是 VLESS，示例请统一使用 `vless`。

## 客户端

本地明文连接示例见 [examples/client-singbox.json](examples/client-singbox.json)。先替换 UUID，使其与服务端一致：

```sh
sing-box run -c examples/client-singbox.json
curl --proxy socks5h://127.0.0.1:1080 https://www.cloudflare.com/cdn-cgi/trace
```

远程部署时，客户端的地址、端口、UUID 和 WebSocket 路径必须与实际入口一致。

已验证的客户端行为：

- sing-box 1.14.4：可使用明文 WS。
- Xray 26.11.9：拒绝“公网地址 + VLESS + 无 TLS”的配置；回环地址可用于本地测试。这个结论针对该版本，不代表所有历史或未来版本。
- Xray 与 sing-box：通过外部 TLS 入口连接 bare 均已验证。客户端设置 TLS，bare 仍接收本地明文 WS。

## 外部 TLS 与临时隧道

典型连接方式：

```text
客户端 ── WSS/TLS ── TLS 入口 ── WS ── bare ── TCP ── 目标
```

TLS 可由反向代理或隧道入口提供，无需编入 bare。以 Cloudflare Quick Tunnel 为临时测试示例：

```sh
cloudflared tunnel --url http://127.0.0.1:28444
```

使用命令输出的域名作为客户端地址，端口 `443`，开启 TLS；SNI 和 WebSocket Host 填该域名，路径保持 `/vl`。不要跳过证书验证。Quick Tunnel 没有可用性保证，重建隧道通常会更换域名；使用时应自行确认服务商条款。

这类 TLS 入口保护客户端到入口的连接；入口能够读取流量，入口到 bare 的链路仍须放在可信环境中。

## Linux 静态构建

以 Debian/Ubuntu 的 x86-64 musl 目标为例，在已安装 rustup 的前提下：

```sh
sudo apt-get install musl-tools
rustup target add x86_64-unknown-linux-musl
cargo build --release --locked --target x86_64-unknown-linux-musl
```

产物：`target/x86_64-unknown-linux-musl/release/bare`。musl 是静态链接的 C 运行库；产物无需系统提供动态 libc。其他 CPU 架构需要对应目标和交叉链接器，未在这里验证。

release profile 使用 `opt-level = 3`、LTO、单 codegen unit、`panic = "abort"` 和 strip。没有 Cargo 第三方依赖。

## 发布 Release

仓库提供名为 **Release** 的手动 workflow（`.github/workflows/release.yml`）：

1. 在 GitHub 仓库页面进入 **Actions → Release → Run workflow**。
2. 在 **version** 输入版本号，例如 `0.2.0`，开头的 `v` 可省略，也支持 `0.2.0-rc.1` 这类后缀。
3. workflow 在 `ubuntu-24.04` 上同时编译 `x86_64-unknown-linux-musl`（amd64）和 `aarch64-unknown-linux-musl`（arm64）两个目标，用 Rust 自带的 `rust-lld` 做自包含静态链接（无需外部交叉编译器），检查产物为静态链接且架构正确，再创建 tag `v<version>` 的 Release，上传：
   - `bare-amd64-linux-musl`
   - `bare-arm64-linux-musl`
   - `SHA256SUMS`（两个二进制的 sha256）

该 workflow 仅由 `workflow_dispatch` 触发，不会因 push 或 tag 自动运行；需要 `contents: write` 权限，由 GitHub 自动提供。二进制与源码中的 Cargo 版本号不会自动同步，tag/release 版本号以输入值为准。

## 测量与验证

开发时在一台 Linux x86-64 主机上记录过：

- 静态 musl 产物：533,288 字节（约 521 KiB）。
- 一次测试结束后的 RSS：500 kB。此值不代表高并发内存上限。
- sing-box 客户端、256 MiB 环回下载：两次约 482 和 530 MiB/s。
- 32 MiB 文件与直连 MD5 相同，8 路并发 HTTP 请求全部成功。
- 外部 TLS 隧道：Xray 与 sing-box 均能访问，32 MiB 文件完整性检查通过。

这些是特定环境的观测值，不是性能保证。吞吐受客户端内核、CPU、源服务器和链路影响；不同内核、不同测试轮次的数字不应当作严格对照实验。仓库暂未提供可复现的完整基准套件，也没有自动化协议回归测试。

## 范围与限制

- 仅支持 TCP；不支持 UDP、mux、Vision、TLS 或 WebSocket early data。
- 只使用一个入站，出站始终直连；没有路由规则或 DNS 策略引擎。
- 阻塞 I/O，每连接使用两个线程；尚无连接数限制或高并发资源保证。
- WebSocket 实现是 RFC 6455 子集，包含握手、二进制帧、分片和控制帧处理，未经过完整协议一致性测试。
- VLESS UUID 是凭据，不是加密。明文连接上的 UUID 和数据都可能被读取；WebSocket masking 也不是加密。
- 手写 JSON、SHA-1、Base64 和协议解析器，尚未经独立安全审计。SHA-1 只用于 WebSocket 握手，不提供内容安全保护。

模块位于 `src/`：`bare.rs` 解析 VLESS 请求，`ws.rs` 处理 WebSocket，`relay.rs` 搬运数据，`dial.rs` 建立出站，`config.rs` 和 `json.rs` 读取配置。

## License

[MIT](LICENSE)
