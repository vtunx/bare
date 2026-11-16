//! 出站连接：把 VLESS 解出的目标地址连上去。
//!
//! IP 版本与地址顺序完全交给系统（getaddrinfo）处理：bare 不做 DNS 策略，
//! 也不自己筛选 IPv4/IPv6。系统按 RFC 3484 / gai.conf 排序地址，
//! TcpStream::connect 会依次尝试直到连接成功。

use std::io;
use std::net::TcpStream;

use crate::bare::Target;

pub fn connect(target: &Target) -> io::Result<TcpStream> {
    let stream = match target {
        Target::Ip(addr) => TcpStream::connect(addr)?,
        Target::Domain(host, port) => TcpStream::connect((host.as_str(), *port))?,
    };
    stream.set_nodelay(true)?;
    Ok(stream)
}
