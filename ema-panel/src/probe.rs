//! Read-only transport diagnostics (`GET /api/probe`).
//!
//! Opens fresh sessions and issues plain GET requests for the two state pages with a few
//! different request styles, and reports what came back on the wire. It never appends a key
//! or button code, so it cannot make the panel do anything.

use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

async fn raw_get(addr: &str, request: &str, wait: Duration) -> Value {
    let started = Instant::now();
    let work = async {
        let mut s = TcpStream::connect(addr).await.map_err(|e| format!("connect: {e}"))?;
        s.write_all(request.as_bytes()).await.map_err(|e| format!("write: {e}"))?;
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            match s.read(&mut chunk).await {
                Ok(0) => break,
                Ok(n) => {
                    buf.extend_from_slice(&chunk[..n]);
                    if buf.len() > 64 * 1024 {
                        break;
                    }
                }
                Err(e) => {
                    return if buf.is_empty() { Err(format!("read: {e}")) } else { Ok(buf) };
                }
            }
        }
        Ok::<Vec<u8>, String>(buf)
    };
    let res = tokio::time::timeout(wait, work).await;
    let ms = started.elapsed().as_millis() as u64;
    match res {
        Err(_) => json!({ "ok": false, "ms": ms, "error": "timeout" }),
        Ok(Err(e)) => json!({ "ok": false, "ms": ms, "error": e }),
        Ok(Ok(bytes)) if bytes.is_empty() => json!({ "ok": false, "ms": ms, "error": "connection closed without any response" }),
        Ok(Ok(bytes)) => {
            let text: String = bytes.iter().map(|&b| b as char).collect();
            let (head, body) = text.split_once("\r\n\r\n").unwrap_or((text.as_str(), ""));
            json!({
                "ok": true,
                "ms": ms,
                "bytes": bytes.len(),
                "head": head.chars().take(400).collect::<String>(),
                "body": body.chars().take(700).collect::<String>(),
            })
        }
    }
}

fn tag<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let a = xml.find(&open)? + open.len();
    let b = xml[a..].find(&close)? + a;
    Some(xml[a..b].trim())
}

fn request(page: &str, group: &str, sid: &str, cnt: &str, host: &str, style: &str) -> String {
    let path = format!("/{page}?B-GR={group}&S-ID={sid}&CNT={cnt}");
    match style {
        "http10" => format!("GET {path} HTTP/1.0\r\n\r\n"),
        "android" => format!(
            "GET {path} HTTP/1.1\r\nUser-Agent: Dalvik/2.1.0 (Linux; U; Android 13)\r\nHost: {host}\r\nConnection: Keep-Alive\r\nAccept-Encoding: gzip\r\n\r\n"
        ),
        _ => format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"),
    }
}

pub async fn run(host: &str, port: u16, group: &str) -> Value {
    let addr = format!("{host}:{port}");
    let hostport = addr.clone();
    let mut out = Vec::new();
    // (label, page, request style, delay after init in ms)
    let variants: [(&str, &str, &str, u64); 6] = [
        ("io, http/1.1 close, immediate", "ui_io.xml", "close", 0),
        ("io, http/1.0, immediate", "ui_io.xml", "http10", 0),
        ("io, android-like headers, immediate", "ui_io.xml", "android", 0),
        ("io, http/1.1 close, after 1.5 s", "ui_io.xml", "close", 1500),
        ("control page, http/1.1 close, immediate", "ui_12540.xml", "close", 0),
        ("control page, android-like headers, after 1.5 s", "ui_12540.xml", "android", 1500),
    ];
    for (label, page, style, delay) in variants {
        let init = raw_get(&addr, &request("ui_init.xml", group, "00", "FFFF", &hostport, "close"), Duration::from_secs(6)).await;
        let mut entry = json!({ "variant": label });
        let body = init.get("body").and_then(|b| b.as_str()).unwrap_or("");
        // the stored body is cut for display; re-parse from the head+body only if complete
        let sid = tag(body, "Session_ID").map(str::to_string);
        let cnt = tag(body, "Counter").map(str::to_string);
        match (sid, cnt) {
            (Some(sid), Some(cnt)) => {
                if delay > 0 {
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                }
                entry["session"] = json!({ "sid": sid, "cnt": cnt });
                entry["result"] = raw_get(&addr, &request(page, group, &sid, &cnt, &hostport, style), Duration::from_secs(8)).await;
            }
            _ => entry["init"] = init,
        }
        out.push(entry);
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    json!({ "note": "read-only: only plain GETs without key/button codes", "results": out })
}
