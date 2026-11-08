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

/// One step of a probe sequence: (page, delay before the request in ms, request style).
type Step = (&'static str, u64, &'static str);

async fn sequence(addr: &str, group: &str, label: &str, steps: &[Step]) -> Value {
    let mut log = Vec::new();
    let init = raw_get(addr, &request("ui_init.xml", group, "00", "FFFF", addr, "close"), Duration::from_secs(6)).await;
    let body = init.get("body").and_then(|b| b.as_str()).unwrap_or("");
    let (sid, mut cnt) = match (tag(body, "Session_ID"), tag(body, "Counter")) {
        (Some(a), Some(b)) => (a.to_string(), b.to_string()),
        _ => return json!({ "sequence": label, "init": init }),
    };
    log.push(json!({ "step": "ui_init.xml", "sid": sid, "cnt": cnt }));
    for (page, delay, style) in steps {
        if *delay > 0 {
            tokio::time::sleep(Duration::from_millis(*delay)).await;
        }
        let r = raw_get(addr, &request(page, group, &sid, &cnt, addr, style), Duration::from_secs(8)).await;
        let ok = r.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        let mut entry = json!({ "step": page, "style": style, "delay": delay, "usedCnt": cnt.clone(), "ok": ok, "ms": r.get("ms") });
        if ok {
            let b = r.get("body").and_then(|b| b.as_str()).unwrap_or("");
            if let Some(c) = tag(b, "Counter") {
                cnt = c.to_string();
            }
            entry["head"] = json!(r.get("head").and_then(|h| h.as_str()).unwrap_or("").lines().next().unwrap_or(""));
            entry["body"] = json!(b.chars().take(260).collect::<String>());
        } else {
            entry["error"] = r.get("error").cloned().unwrap_or(Value::Null);
        }
        log.push(entry);
    }
    json!({ "sequence": label, "log": log })
}

pub async fn run(host: &str, port: u16, group: &str) -> Value {
    let addr = format!("{host}:{port}");
    let plan: Vec<(&str, Vec<Step>)> = vec![
        ("init -> control page", vec![("ui_12540.xml", 0, "close")]),
        ("init -> special signs -> io -> control", vec![("ui_12540_sz.xml", 0, "close"), ("ui_io.xml", 700, "close"), ("ui_12540.xml", 700, "close")]),
        ("init -> wait 1 s -> io", vec![("ui_io.xml", 1000, "close")]),
        ("init -> io with android-like headers", vec![("ui_io.xml", 0, "android")]),
        ("init -> io with http/1.0", vec![("ui_io.xml", 0, "http10")]),
    ];
    let mut out = Vec::new();
    for (label, steps) in plan {
        out.push(sequence(&addr, group, label, &steps).await);
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
    json!({ "note": "read-only: only plain GETs without key/button codes; poller paused while probing", "results": out })
}
