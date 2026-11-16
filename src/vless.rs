//! VLESS 协议：请求头解析、响应头生成。
//!
//! 请求头布局：
//!   version(1) | uuid(16) | addons_len(1) | addons(N) |
//!   command(1) | port(2) | addr_type(1) | address(S)
//!
//! 之后紧接的就是 payload，没有任何分块、padding 或加密层。
//! 响应头只有 2 字节：{0, 0}（version 0、addons 长度 0）。
//!
//! 注意：VLESS 本身不加密，uuid 是明文。见 README 的安全说明。

use std::io::{self, ErrorKind, Read};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};

pub const CMD_TCP: u8 = 1;
#[allow(dead_code)]
pub const CMD_UDP: u8 = 2;
#[allow(dead_code)]
pub const CMD_MUX: u8 = 3;

pub enum Addr {
    V4(Ipv4Addr),
    V6(Ipv6Addr),
    Domain(String),
}

pub struct Request {
    pub uuid: [u8; 16],
    pub command: u8,
    pub port: u16,
    pub addr: Addr,
}

fn bad<T>(msg: &'static str) -> io::Result<T> {
    Err(io::Error::new(ErrorKind::InvalidData, msg))
}

/// 从流中精确读出 VLESS 请求头。
///
/// 只消费头部字节，一个字都不多读——这样 payload 自然留在流里，
/// 不存在「被隐藏缓冲吃掉」的问题。
pub fn read_request<R: Read>(rd: &mut R) -> io::Result<Request> {
    // version + uuid + addons_len 一次读完
    let mut head = [0u8; 18];
    rd.read_exact(&mut head)?;

    let version = head[0];
    if version != 0 {
        return bad("vless: unsupported version");
    }
    let mut uuid = [0u8; 16];
    uuid.copy_from_slice(&head[1..17]);
    let addons_len = head[17] as usize;

    if addons_len > 0 {
        // 目前只需要跳过：flow / early-data 之类的扩展本实现不参与
        let mut skip = vec![0u8; addons_len];
        rd.read_exact(&mut skip)?;
    }

    // command + port + addr_type
    let mut meta = [0u8; 4];
    rd.read_exact(&mut meta)?;
    let command = meta[0];
    let port = u16::from_be_bytes([meta[1], meta[2]]);
    let addr_type = meta[3];

    let addr = match addr_type {
        1 => {
            let mut b = [0u8; 4];
            rd.read_exact(&mut b)?;
            Addr::V4(Ipv4Addr::from(b))
        }
        2 => {
            let mut l = [0u8; 1];
            rd.read_exact(&mut l)?;
            let n = l[0] as usize;
            if n == 0 {
                return bad("vless: empty domain");
            }
            let mut d = vec![0u8; n];
            rd.read_exact(&mut d)?;
            match String::from_utf8(d) {
                Ok(s) => Addr::Domain(s),
                Err(_) => return bad("vless: bad domain"),
            }
        }
        3 => {
            let mut b = [0u8; 16];
            rd.read_exact(&mut b)?;
            Addr::V6(Ipv6Addr::from(b))
        }
        _ => return bad("vless: unknown address type"),
    };

    Ok(Request {
        uuid,
        command,
        port,
        addr,
    })
}

/// 响应头：version 0 + addons 长度 0。
pub const RESPONSE: [u8; 2] = [0, 0];

/// 把目标地址转成可连接的形态。域名留给 dial 阶段解析。
pub fn target(addr: &Addr, port: u16) -> Target {
    match addr {
        Addr::V4(ip) => Target::Ip(SocketAddr::from((*ip, port))),
        Addr::V6(ip) => Target::Ip(SocketAddr::from((*ip, port))),
        Addr::Domain(d) => Target::Domain(d.clone(), port),
    }
}

pub enum Target {
    Ip(SocketAddr),
    Domain(String, u16),
}

/// uuid 的 16 进制小写形式，用于日志与配置比对。
pub fn uuid_hex(u: &[u8; 16]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(36);
    for (i, b) in u.iter().enumerate() {
        if i == 4 || i == 6 || i == 8 || i == 10 {
            s.push('-');
        }
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0F) as usize] as char);
    }
    s
}

/// 解析配置里的 uuid 字符串。
pub fn parse_uuid(s: &str) -> Option<[u8; 16]> {
    let b = s.as_bytes();
    if b.len() != 36 {
        return None;
    }
    let mut out = [0u8; 16];
    let mut idx = 0usize;
    let mut i = 0usize;
    while i < 36 {
        if i == 8 || i == 13 || i == 18 || i == 23 {
            if b[i] != b'-' {
                return None;
            }
            i += 1;
            continue;
        }
        let hi = hex_val(b[i])?;
        let lo = hex_val(b[i + 1])?;
        if idx >= 16 {
            return None;
        }
        out[idx] = hi << 4 | lo;
        idx += 1;
        i += 2;
    }
    if idx == 16 {
        Some(out)
    } else {
        None
    }
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}
