//! 极简 VLESS + WebSocket 服务端。
//!
//! 只做一件事：VLESS 入站 → direct 出站。没有规则引擎、没有 DNS 模块、
//! 没有 TLS 栈、没有任何第三方依赖。
//!
//! 用法：
//!   bare [config.json]

mod base64;
mod config;
mod dial;
mod json;
mod relay;
mod sha1;
mod bare;
mod ws;

use std::io;
use std::net::{TcpListener, TcpStream};
use std::thread;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "config.json".to_string());

    let cfg = match config::load(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };

    let addr = if cfg.listen.contains(':') && !cfg.listen.starts_with('[') {
        // 裸 IPv6 地址要加方括号
        format!("[{}]:{}", cfg.listen, cfg.port)
    } else {
        format!("{}:{}", cfg.listen, cfg.port)
    };

    let ln = match TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("bind {}: {}", addr, e);
            std::process::exit(1);
        }
    };

    println!(
        "listening on {} (ws path={:?}, ipv6={})",
        addr, cfg.ws_path, cfg.prefer_v6
    );

    for incoming in ln.incoming() {
        match incoming {
            Ok(stream) => {
                let cfg = cfg.clone();
                thread::spawn(move || {
                    let peer = stream
                        .peer_addr()
                        .map(|a| a.to_string())
                        .unwrap_or_else(|_| "?".to_string());
                    if let Err(e) = serve(stream, &cfg) {
                        eprintln!("{}: {}", peer, e);
                    }
                });
            }
            Err(_) => continue,
        }
    }
}

fn serve(stream: TcpStream, cfg: &config::Config) -> io::Result<()> {
    let (mut reader, mut writer) = ws::accept(stream, &cfg.ws_path)?;

    let req = bare::read_request(&mut reader)?;

    if !cfg.uuids.iter().any(|u| *u == req.uuid) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("bare: unknown uuid {}", bare::uuid_hex(&req.uuid)),
        ));
    }
    if req.command != bare::CMD_TCP {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("bare: unsupported command {}", req.command),
        ));
    }

    let target = bare::target(&req.addr, req.port);
    let up = dial::connect(&target, cfg.prefer_v6)?;

    // 连接成功后才回响应头，失败时让客户端直接看到断开
    writer.send_small(&bare::RESPONSE)?;

    relay::run(reader, writer, up)
}
