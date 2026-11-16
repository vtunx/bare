//! base64 编码。
//!
//! 只需要 encode：服务端只在 WS 握手里把 SHA1 摘要编成 Sec-WebSocket-Accept，
//! 从不解析客户端发来的 base64，所以 decode 不实现，省掉一半代码。

const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// 把 `input` 编码后追加写入 `out`。
pub fn encode_into(input: &[u8], out: &mut Vec<u8>) {
    let mut i = 0usize;
    while i + 3 <= input.len() {
        let n = (input[i] as u32) << 16 | (input[i + 1] as u32) << 8 | input[i + 2] as u32;
        out.push(TABLE[((n >> 18) & 63) as usize]);
        out.push(TABLE[((n >> 12) & 63) as usize]);
        out.push(TABLE[((n >> 6) & 63) as usize]);
        out.push(TABLE[(n & 63) as usize]);
        i += 3;
    }
    match input.len() - i {
        1 => {
            let n = (input[i] as u32) << 16;
            out.push(TABLE[((n >> 18) & 63) as usize]);
            out.push(TABLE[((n >> 12) & 63) as usize]);
            out.push(b'=');
            out.push(b'=');
        }
        2 => {
            let n = (input[i] as u32) << 16 | (input[i + 1] as u32) << 8;
            out.push(TABLE[((n >> 18) & 63) as usize]);
            out.push(TABLE[((n >> 12) & 63) as usize]);
            out.push(TABLE[((n >> 6) & 63) as usize]);
            out.push(b'=');
        }
        _ => {}
    }
}
