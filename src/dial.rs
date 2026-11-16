//! 出站连接：把 VLESS 解出的目标地址连上去。
//!
//! `prefer_ipv6` 的语义与 sing-box 的 direct 出站一致：
//! 系统解析出多个地址时，优先用 IPv6，没有 IPv6 再退回 IPv4。

use std::io;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::bare::Target;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

pub fn connect(target: &Target, prefer_v6: bool) -> io::Result<TcpStream> {
    let stream = match target {
        Target::Ip(addr) => TcpStream::connect_timeout(addr, CONNECT_TIMEOUT)?,
        Target::Domain(host, port) => {
            let addrs: Vec<_> = (host.as_str(), *port).to_socket_addrs()?.collect();
            if addrs.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "dial: no address resolved",
                ));
            }
            let pick = if prefer_v6 {
                addrs
                    .iter()
                    .find(|a| a.is_ipv6())
                    .or_else(|| addrs.iter().find(|a| a.is_ipv4()))
            } else {
                addrs
                    .iter()
                    .find(|a| a.is_ipv4())
                    .or_else(|| addrs.iter().find(|a| a.is_ipv6()))
            }
            .or_else(|| addrs.first())
            .copied()
            .unwrap();

            TcpStream::connect_timeout(&pick, CONNECT_TIMEOUT)?
        }
    };

    stream.set_nodelay(true)?;
    Ok(stream)
}
