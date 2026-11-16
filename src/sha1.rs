//! SHA-1（FIPS 180-1），只用于 WebSocket 握手的 Sec-WebSocket-Accept。
//!
//! 手写而非引 crate，是为了保持零依赖。SHA-1 是纯函数、无状态陷阱，
//! 比手写 AEAD 简单得多：只要 padding 和字节序对，就没有隐蔽的坑。

pub struct Sha1 {
    h: [u32; 5],
    buf: [u8; 64],
    buf_len: usize,
    total: u64,
}

impl Sha1 {
    pub fn new() -> Self {
        Sha1 {
            h: [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0],
            buf: [0u8; 64],
            buf_len: 0,
            total: 0,
        }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.total = self.total.wrapping_add(data.len() as u64);

        // 先把上次的残留填满
        if self.buf_len > 0 {
            let need = 64 - self.buf_len;
            let take = need.min(data.len());
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&data[..take]);
            self.buf_len += take;
            data = &data[take..];
            if self.buf_len == 64 {
                let block = self.buf;
                self.compress(&block);
                self.buf_len = 0;
            }
        }

        // 整块处理
        while data.len() >= 64 {
            let mut block = [0u8; 64];
            block.copy_from_slice(&data[..64]);
            self.compress(&block);
            data = &data[64..];
        }

        // 余量留着
        if !data.is_empty() {
            self.buf[..data.len()].copy_from_slice(data);
            self.buf_len = data.len();
        }
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let (mut a, mut b, mut c, mut d, mut e) =
            (self.h[0], self.h[1], self.h[2], self.h[3], self.h[4]);

        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let tmp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = tmp;
        }

        self.h[0] = self.h[0].wrapping_add(a);
        self.h[1] = self.h[1].wrapping_add(b);
        self.h[2] = self.h[2].wrapping_add(c);
        self.h[3] = self.h[3].wrapping_add(d);
        self.h[4] = self.h[4].wrapping_add(e);
    }

    pub fn finish(mut self) -> [u8; 20] {
        let bits = self.total.wrapping_mul(8);
        self.update_raw(&[0x80]);
        while self.buf_len != 56 {
            self.update_raw(&[0x00]);
        }
        self.update_raw(&bits.to_be_bytes());

        let mut out = [0u8; 20];
        for i in 0..5 {
            out[i * 4..i * 4 + 4].copy_from_slice(&self.h[i].to_be_bytes());
        }
        out
    }

    /// 只用于收尾填充，不计入 total。
    fn update_raw(&mut self, data: &[u8]) {
        for &b in data {
            self.buf[self.buf_len] = b;
            self.buf_len += 1;
            if self.buf_len == 64 {
                let block = self.buf;
                self.compress(&block);
                self.buf_len = 0;
            }
        }
    }
}

pub fn digest(data: &[u8]) -> [u8; 20] {
    let mut h = Sha1::new();
    h.update(data);
    h.finish()
}
