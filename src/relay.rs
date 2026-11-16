//! 双向搬运：把 WS 连接和上游 TCP 连起来。
//!
//! 两个方向各占一个线程，各自独占一个句柄，全程无锁：
//!   * 上游 → 客户端：读进 WsWriter 的负载区（帧头已预留），一次 write_all 发出
//!   * 客户端 → 上游：读进 32 KiB 暂存，直接写上游
//!
//! 上游 EOF 时给客户端发 WS Close 帧；客户端 EOF 时半关闭上游写端。

use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::thread;

use crate::ws::{WsReader, WsWriter};

const UP_BUF: usize = 32 * 1024;

pub fn run(mut reader: WsReader, mut writer: WsWriter, up: TcpStream) -> std::io::Result<()> {
    let mut up_read = up.try_clone()?;
    let mut up_write = up;

    let up_to_ws = thread::spawn(move || {
        loop {
            let n = {
                let buf = writer.buf();
                match up_read.read(buf) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(_) => break,
                }
            };
            if writer.flush(n).is_err() {
                break;
            }
        }
        writer.close();
    });

    let mut tmp = vec![0u8; UP_BUF];
    loop {
        match reader.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                if up_write.write_all(&tmp[..n]).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    let _ = up_write.shutdown(Shutdown::Write);
    let _ = up_to_ws.join();
    Ok(())
}
