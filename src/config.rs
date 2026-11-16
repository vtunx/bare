//! 启动配置。兼容 sing-box 的配置子集，方便直接复用现有 JSON。
//!
//! 读这几个字段就够：
//!   inbounds[].type           "vless"（也接受 "vmess"，为了复用旧文件）
//!   inbounds[].listen
//!   inbounds[].listen_port
//!   inbounds[].users[].id     （也接受 .uuid）
//!   inbounds[].transport.path
//!   outbounds[].type          "direct"
//!   outbounds[].domain_strategy
//!
//! 其余字段静默忽略。

use crate::json;
use crate::bare::parse_uuid;

#[derive(Clone)]
pub struct Config {
    pub listen: String,
    pub port: u16,
    pub uuids: Vec<[u8; 16]>,
    pub ws_path: String,
    pub prefer_v6: bool,
}

pub fn load(path: &str) -> Result<Config, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("read {}: {}", path, e))?;
    let root = json::parse(&text)?;

    let mut cfg = Config {
        listen: "::".to_string(),
        port: 0,
        uuids: Vec::new(),
        ws_path: String::new(),
        prefer_v6: false,
    };

    let inbounds = root
        .get("inbounds")
        .and_then(|v| v.as_arr())
        .ok_or_else(|| "config: missing inbounds".to_string())?;

    let mut found = false;
    for ib in inbounds {
        let kind = ib.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if kind != "vless" && kind != "vmess" {
            continue;
        }
        found = true;

        if let Some(l) = ib.get("listen").and_then(|v| v.as_str()) {
            if !l.is_empty() {
                cfg.listen = l.to_string();
            }
        }
        if let Some(p) = ib.get("listen_port").and_then(|v| v.as_usize()) {
            cfg.port = p as u16;
        }
        if let Some(users) = ib.get("users").and_then(|v| v.as_arr()) {
            for u in users {
                let id = u
                    .get("id")
                    .and_then(|v| v.as_str())
                    .or_else(|| u.get("uuid").and_then(|v| v.as_str()));
                if let Some(s) = id {
                    match parse_uuid(s) {
                        Some(u8v) => cfg.uuids.push(u8v),
                        None => return Err(format!("config: bad uuid '{}'", s)),
                    }
                }
            }
        }
        if let Some(tr) = ib.get("transport") {
            if tr.get("type").and_then(|v| v.as_str()) == Some("ws") {
                if let Some(p) = tr.get("path").and_then(|v| v.as_str()) {
                    cfg.ws_path = p.to_string();
                }
            }
        }
        break;
    }

    if !found {
        return Err("config: no inbound".to_string());
    }
    if cfg.port == 0 {
        return Err("config: missing listen_port".to_string());
    }
    if cfg.uuids.is_empty() {
        return Err("config: missing uuid".to_string());
    }

    if let Some(obs) = root.get("outbounds").and_then(|v| v.as_arr()) {
        for ob in obs {
            if ob.get("type").and_then(|v| v.as_str()) == Some("direct") {
                if ob.get("domain_strategy").and_then(|v| v.as_str()) == Some("prefer_ipv6") {
                    cfg.prefer_v6 = true;
                }
            }
        }
    }

    Ok(cfg)
}
