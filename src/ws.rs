//! 最小 WebSocket 服务端实现（RFC 6455 子集）。
//!
//! 只支持：HTTP/1.1 升级、二进制帧、ping/pong/close、分片重组。
//! 不支持：扩展协商（permessage-deflate 等）、文本语义。
//!
//! 设计要点：
//!   * 缓冲区只存索引（start/end），不存切片，避免自引用和借用检查器打架。
//!   * 入站（客户端→服务端）帧必定带掩码，读进目标缓冲后原地 XOR，零额外拷贝。
//!   * 出站（服务端→客户端）帧不带掩码，在缓冲区预留 10 字节帧头，
//!     让帧头与负载在内存里连续，一次 write_all 发出。
//!   * 握手阶段多读的字节（客户端可能提前发帧）显式交给 WsReader，
//!     绝不藏在某个私有缓冲里——这是 Go 版踩过的最大的坑。
//!   * 读端和写端各持一个独立 TcpStream 句柄：阻塞读绝不能持有写锁，
//!     否则对端不发数据时连 pong 都发不出去。

use std::io::{self, ErrorKind, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::{base64, sha1};

const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

#[allow(dead_code)]
const OP_TEXT: u8 = 0x1;
#[allow(dead_code)]
const OP_BINARY: u8 = 0x2;
const OP_CLOSE: u8 = 0x8;
const OP_PING: u8 = 0x9;
const OP_PONG: u8 = 0xA;

/// 出站帧头的预留空间：最长头部是 2 + 8 字节。
const HDR_MAX: usize = 10;
/// 出站负载缓冲大小。32 KiB 在吞吐上与大缓冲无差异，但每连接省一半常驻内存。
const OUT_CAP: usize = 32 * 1024;
/// 入站原始字节缓冲大小。
const IN_CAP: usize = 64 * 1024;

fn err<T>(msg: &'static str) -> io::Result<T> {
    Err(io::Error::new(ErrorKind::InvalidData, msg))
}

/// 写端句柄。读端回 pong 也要用它，所以用 Arc<Mutex<>> 共享。
/// 锁只覆盖「一次完整的帧写出」，绝不会跨越阻塞读。
#[derive(Clone)]
struct Sink {
    inner: Arc<Mutex<TcpStream>>,
}

impl Sink {
    fn lock(&self) -> std::sync::MutexGuard<'_, TcpStream> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn send_control(&self, opcode: u8, payload: &[u8]) -> io::Result<()> {
        let mut buf = [0u8; HDR_MAX + 125];
        let h = write_header(&mut buf, opcode, payload.len());
        buf[h..h + payload.len()].copy_from_slice(payload);
        let mut g = self.lock();
        g.write_all(&buf[..h + payload.len()])
    }
}

fn header_len(n: usize) -> usize {
    if n < 126 {
        2
    } else if n < 65536 {
        4
    } else {
        10
    }
}

/// 把帧头写在 `dst[0..]`，返回帧头长度。
fn write_header(dst: &mut [u8], opcode: u8, n: usize) -> usize {
    let h = header_len(n);
    dst[0] = 0x80 | opcode; // FIN = 1
    match h {
        2 => dst[1] = n as u8,
        4 => {
            dst[1] = 126;
            dst[2..4].copy_from_slice(&(n as u16).to_be_bytes());
        }
        _ => {
            dst[1] = 127;
            dst[2..10].copy_from_slice(&(n as u64).to_be_bytes());
        }
    }
    h
}

/// 帧头必须紧贴在偏移 `end` 之前结束，让 [帧头|负载] 连续。
fn write_header_at(dst: &mut [u8], end: usize, opcode: u8, n: usize) -> usize {
    let h = header_len(n);
    let mut tmp = [0u8; HDR_MAX];
    write_header(&mut tmp, opcode, n);
    dst[end - h..end].copy_from_slice(&tmp[..h]);
    h
}

fn unmask(buf: &mut [u8], mask: [u8; 4], mut pos: usize) -> usize {
    for b in buf.iter_mut() {
        *b ^= mask[pos & 3];
        pos += 1;
    }
    pos & 3
}

/// 出站写端：一次 write_all 发出「帧头 + 负载」。
pub struct WsWriter {
    sink: Sink,
    out: Vec<u8>,
    closed: bool,
}

impl WsWriter {
    /// 取得可写负载区（前面 10 字节已预留给帧头）。
    pub fn buf(&mut self) -> &mut [u8] {
        &mut self.out[HDR_MAX..]
    }

    /// 把 `buf()` 里前 `n` 字节作为一条二进制消息发出。
    pub fn flush(&mut self, n: usize) -> io::Result<()> {
        if n == 0 {
            return Ok(());
        }
        let h = write_header_at(&mut self.out, HDR_MAX, OP_BINARY, n);
        let start = HDR_MAX - h;
        let mut g = self.sink.lock();
        g.write_all(&self.out[start..HDR_MAX + n])
    }

    /// 发一条小的二进制消息（VLESS 响应头用）。
    pub fn send_small(&mut self, data: &[u8]) -> io::Result<()> {
        let mut tmp = [0u8; HDR_MAX + 16];
        let h = write_header(&mut tmp, OP_BINARY, data.len());
        tmp[h..h + data.len()].copy_from_slice(data);
        let mut g = self.sink.lock();
        g.write_all(&tmp[..h + data.len()])
    }

    pub fn close(&mut self) {
        if !self.closed {
            self.closed = true;
            let _ = self.sink.send_control(OP_CLOSE, &[0x03, 0xE8]);
        }
    }
}

/// 入站读端。
pub struct WsReader {
    /// 独立的读句柄，读操作不加任何锁
    stream: TcpStream,
    /// 仅用于回 pong / close
    sink: Sink,
    buf: Vec<u8>,
    start: usize,
    end: usize,
    payload_left: usize,
    mask: [u8; 4],
    mask_pos: usize,
    masked: bool,
    closed: bool,
}

impl WsReader {
    fn new(stream: TcpStream, sink: Sink, leftover: &[u8]) -> Self {
        let mut buf = vec![0u8; IN_CAP];
        let n = leftover.len().min(IN_CAP);
        buf[..n].copy_from_slice(&leftover[..n]);
        WsReader {
            stream,
            sink,
            buf,
            start: 0,
            end: n,
            payload_left: 0,
            mask: [0; 4],
            mask_pos: 0,
            masked: false,
            closed: false,
        }
    }

    /// 保证 `buf[start..end]` 至少还有 `n` 字节；不够就从 socket 读。
    fn ensure(&mut self, n: usize) -> io::Result<()> {
        if self.end - self.start >= n {
            return Ok(());
        }
        if self.start > 0 {
            self.buf.copy_within(self.start..self.end, 0);
            self.end -= self.start;
            self.start = 0;
        }
        while self.end < n {
            if self.end == self.buf.len() {
                return err("ws: frame header too large");
            }
            let got = self.stream.read(&mut self.buf[self.end..])?;
            if got == 0 {
                return err("ws: eof");
            }
            self.end += got;
        }
        Ok(())
    }

    /// 读下一帧的头，设置 payload_left / mask。控制帧就地处理。
    fn next_frame(&mut self) -> io::Result<()> {
        loop {
            self.ensure(2)?;
            let b0 = self.buf[self.start];
            let b1 = self.buf[self.start + 1];
            let opcode = b0 & 0x0F;
            let masked = b1 & 0x80 != 0;
            let mut len = (b1 & 0x7F) as usize;
            let mut hdr = 2;

            if len == 126 {
                self.ensure(4)?;
                len = u16::from_be_bytes([
                    self.buf[self.start + 2],
                    self.buf[self.start + 3],
                ]) as usize;
                hdr = 4;
            } else if len == 127 {
                self.ensure(10)?;
                let mut v = [0u8; 8];
                v.copy_from_slice(&self.buf[self.start + 2..self.start + 10]);
                len = u64::from_be_bytes(v) as usize;
                hdr = 10;
            }

            let mut mask = [0u8; 4];
            if masked {
                self.ensure(hdr + 4)?;
                mask.copy_from_slice(&self.buf[self.start + hdr..self.start + hdr + 4]);
                hdr += 4;
            }
            self.start += hdr;

            match opcode {
                OP_CLOSE => {
                    self.closed = true;
                    let _ = self.sink.send_control(OP_CLOSE, &[0x03, 0xE8]);
                    return err("ws: closed");
                }
                OP_PING => {
                    let n = len.min(125);
                    self.ensure(n)?;
                    let mut payload = [0u8; 125];
                    payload[..n].copy_from_slice(&self.buf[self.start..self.start + n]);
                    if masked {
                        unmask(&mut payload[..n], mask, 0);
                    }
                    self.start += n;
                    self.payload_left = 0;
                    let _ = self.sink.send_control(OP_PONG, &payload[..n]);
                    continue;
                }
                OP_PONG => {
                    let n = len.min(125);
                    self.ensure(n)?;
                    self.start += n;
                    continue;
                }
                _ => {}
            }

            // 数据帧：continuation(0) / text(1) / binary(2) 一律当字节流
            self.payload_left = len;
            self.mask = mask;
            self.mask_pos = 0;
            self.masked = masked;
            if len == 0 {
                continue;
            }
            return Ok(());
        }
    }
}

impl Read for WsReader {
    fn read(&mut self, dst: &mut [u8]) -> io::Result<usize> {
        if dst.is_empty() || self.closed {
            return Ok(0);
        }

        loop {
            if self.payload_left > 0 {
                // 快路径：内部缓冲已空，直接读进目标缓冲再原地解掩码
                if self.start == self.end {
                    let n = self.payload_left.min(dst.len());
                    let got = self.stream.read(&mut dst[..n])?;
                    if got == 0 {
                        self.closed = true;
                        return err("ws: eof mid-frame");
                    }
                    if self.masked {
                        self.mask_pos = unmask(&mut dst[..got], self.mask, self.mask_pos);
                    }
                    self.payload_left -= got;
                    return Ok(got);
                }

                // 慢路径：内部缓冲还有数据，先搬出去
                let avail = self.end - self.start;
                let n = self.payload_left.min(dst.len()).min(avail);
                dst[..n].copy_from_slice(&self.buf[self.start..self.start + n]);
                self.start += n;
                self.payload_left -= n;
                if self.masked {
                    self.mask_pos = unmask(&mut dst[..n], self.mask, self.mask_pos);
                }
                return Ok(n);
            }

            if let Err(e) = self.next_frame() {
                if self.closed {
                    return Ok(0);
                }
                return Err(e);
            }
        }
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
}

fn header_value<'a>(lines: &[&'a str], name: &str) -> Option<&'a str> {
    for line in lines {
        if let Some(idx) = line.find(':') {
            if line[..idx].trim().eq_ignore_ascii_case(name) {
                return Some(line[idx + 1..].trim());
            }
        }
    }
    None
}

/// 完成 HTTP 升级握手，返回拆分好的读写端。
pub fn accept(stream: TcpStream) -> io::Result<(WsReader, WsWriter)> {
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_secs(20)))?;

    let mut reader_side = stream.try_clone()?;
    let mut raw: Vec<u8> = Vec::with_capacity(1024);
    let mut tmp = [0u8; 1024];
    let hdr_end;

    loop {
        let n = reader_side.read(&mut tmp)?;
        if n == 0 {
            return err("http: eof during handshake");
        }
        raw.extend_from_slice(&tmp[..n]);
        if let Some(p) = find(&raw, b"\r\n\r\n") {
            hdr_end = p + 4;
            break;
        }
        if raw.len() > 16384 {
            return err("http: header too large");
        }
    }

    let head = match std::str::from_utf8(&raw[..hdr_end]) {
        Ok(s) => s,
        Err(_) => {
            let _ = reader_side.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n");
            return err("http: bad header encoding");
        }
    };
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let parts: Vec<&str> = request_line.split(' ').collect();
    if parts.len() < 2 || parts[0] != "GET" {
        let _ = reader_side.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n");
        return err("http: not a GET");
    }

    let rest: Vec<&str> = lines.collect();
    let upgrade_ok = header_value(&rest, "upgrade")
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);
    let conn_ok = header_value(&rest, "connection")
        .map(|v| v.to_ascii_lowercase().contains("upgrade"))
        .unwrap_or(false);
    let key = match header_value(&rest, "sec-websocket-key") {
        Some(k) if !k.is_empty() => k,
        _ => {
            let _ = reader_side.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n");
            return err("http: missing sec-websocket-key");
        }
    };
    if !upgrade_ok || !conn_ok {
        let _ = reader_side.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n");
        return err("http: not a websocket upgrade");
    }

    let mut concat = Vec::with_capacity(key.len() + WS_GUID.len());
    concat.extend_from_slice(key.as_bytes());
    concat.extend_from_slice(WS_GUID.as_bytes());
    let digest = sha1::digest(&concat);

    let mut accept_val = Vec::with_capacity(28);
    base64::encode_into(&digest, &mut accept_val);

    let mut resp = Vec::with_capacity(160);
    resp.extend_from_slice(b"HTTP/1.1 101 Switching Protocols\r\n");
    resp.extend_from_slice(b"Upgrade: websocket\r\n");
    resp.extend_from_slice(b"Connection: Upgrade\r\n");
    resp.extend_from_slice(b"Sec-WebSocket-Accept: ");
    resp.extend_from_slice(&accept_val);
    resp.extend_from_slice(b"\r\n\r\n");
    reader_side.write_all(&resp)?;

    stream.set_read_timeout(None)?;
    reader_side.set_read_timeout(None)?;

    // 握手期间多读到的字节属于后续帧，显式交给读端
    let leftover = raw[hdr_end..].to_vec();

    let sink = Sink {
        inner: Arc::new(Mutex::new(stream)),
    };
    Ok((
        WsReader::new(reader_side, sink.clone(), &leftover),
        WsWriter {
            sink,
            out: vec![0u8; HDR_MAX + OUT_CAP],
            closed: false,
        },
    ))
}
