# bare

只做 **VLESS over WebSocket → TCP 直连** 的 Rust 服务端。零第三方依赖，无规则引擎、异步运行时或 TLS 栈。

> **bare 不提供链路加密**，UUID 是唯一凭据。生产环境请用外部 TLS，不要把示例 UUID 当密钥。

想要一键部署（脚本 / Docker 镜像 / 可选 Cloudflare Tunnel），见 **[bare-node](https://github.com/cleeate/bare-node)**。本仓库只提供 bare 本体。

## 快速开始

需要 Rust 工具链：

```sh
git clone https://github.com/cleeate/bare.git && cd bare
cargo build --release --locked
cp examples/server.json config.json
cat /proc/sys/kernel/random/uuid   # 生成并替换 config.json 里的示例 UUID
./target/release/bare config.json
```

省略路径时默认读当前目录 `config.json`。示例监听 `127.0.0.1:28444`；对外监听需改 `0.0.0.0` 或 `::` 并配防火墙。

## 配置

完整示例见 [examples/server.json](examples/server.json)，兼容 sing-box 配置的少量字段：

- `listen` / `listen_port`：监听地址与端口（地址省略时为 `::`）。
- `users[].id`：UUID，支持多个用户。

只取第一个匹配的入站（`type: vless`）；出站始终直连，无可配置字段（含 `domain_strategy`）——IP 版本与地址顺序交给系统 getaddrinfo，多地址会依次尝试。其余字段（含 `log.disabled`）被忽略。

## 客户端

本地明文示例见 [examples/client-singbox.json](examples/client-singbox.json)，替换 UUID 使两端一致后：

```sh
sing-box run -c examples/client-singbox.json
curl --proxy socks5h://127.0.0.1:1080 https://www.cloudflare.com/cdn-cgi/trace
```

- sing-box 1.14.4：可连明文 WS。
- Xray 26.11.9：拒绝公网地址 + VLESS + 无 TLS，仅回环可测；套外部 TLS 后两家内核都能连。

## 外部 TLS

bare 不含 TLS，由反向代理或隧道提供。临时测试可用 Cloudflare Quick Tunnel：

```sh
cloudflared tunnel --url http://127.0.0.1:28444
```

用输出的域名作客户端地址，端口 443，开 TLS，SNI 与 WS Host 填该域名。服务端不校验 WS 路径，客户端用任意路径均可。隧道域名随重启变化，免费档条款禁止大流量代理。入口能读流量，入口到 bare 的链路要放在可信环境。

## Linux 静态构建

```sh
sudo apt-get install musl-tools
rustup target add x86_64-unknown-linux-musl
cargo build --release --locked --target x86_64-unknown-linux-musl
```

产物 `target/x86_64-unknown-linux-musl/release/bare`，不依赖动态 libc。release 用 `opt-level = "z"`、LTO、单 codegen unit、`panic = "abort"`、strip。

## 发布 Release

**Actions → Release → Run workflow**，输入版本号（如 `0.2.0`，`v` 可省略）。仅手动触发。CI 在 `ubuntu-24.04` 用 `rust-lld` 自包含静态编译 amd64 + arm64，校验架构与静态链接后创建 tag `v<版本>` 的 Release，上传：

- `bare-amd64-linux-musl`
- `bare-arm64-linux-musl`
- `SHA256SUMS`

## 测量

特定环境观测值，非性能保证：

- 静态产物 496,424 字节（约 485 KiB）。
- 空闲 RSS 约 450 kB；8 路并发增量约 300 kB（每连接常驻 OUT 32 KiB + UP 16 KiB）。
- sing-box 客户端、256 MiB 下载约 480–530 MiB/s；32 MiB 文件 md5 与直连一致，8 路并发全成功。

## 范围与限制

- 仅 TCP；无 UDP / mux / TLS / early data，无路由规则或 DNS 策略。
- 阻塞 I/O，每连接两个线程，无连接数上限。
- WS 为 RFC 6455 子集（握手、二进制帧、分片、控制帧），未经完整一致性测试。
- UUID 是凭据不是加密，WS masking 也不是加密。
- 手写 JSON / SHA-1 / Base64 / 协议解析，未经独立安全审计（SHA-1 仅用于握手）。

## License

[MIT](LICENSE)
