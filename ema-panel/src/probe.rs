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
        let mut s = TcpStream::connect(addr)
            .await
            .map_err(|e| format!("connect: {e}"))?;
        s.write_all(request.as_bytes())
            .await
            .map_err(|e| format!("write: {e}"))?;
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
                    return if buf.is_empty() {
                        Err(format!("read: {e}"))
                    } else {
                        Ok(buf)
                    };
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
        Ok(Ok(bytes)) if bytes.is_empty() => {
            json!({ "ok": false, "ms": ms, "error": "connection closed without any response" })
        }
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
    let init = raw_get(
        addr,
        &request("ui_init.xml", group, "00", "FFFF", addr, "close"),
        Duration::from_secs(6),
    )
    .await;
    let body = init.get("body").and_then(|b| b.as_str()).unwrap_or("");
    let (sid, mut cnt) = match (tag(body, "Session_ID"), tag(body, "Counter")) {
        (Some(a), Some(b)) => (a.to_string(), crate::panel::next_counter(b)),
        _ => return json!({ "sequence": label, "init": init }),
    };
    log.push(json!({ "step": "ui_init.xml", "sid": sid, "cnt": cnt }));
    for (page, delay, style) in steps {
        if *delay > 0 {
            tokio::time::sleep(Duration::from_millis(*delay)).await;
        }
        let r = raw_get(
            addr,
            &request(page, group, &sid, &cnt, addr, style),
            Duration::from_secs(8),
        )
        .await;
        let ok = r.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        let mut entry = json!({ "step": page, "style": style, "delay": delay, "usedCnt": cnt.clone(), "ok": ok, "ms": r.get("ms") });
        if ok {
            let b = r.get("body").and_then(|b| b.as_str()).unwrap_or("");
            if let Some(c) = tag(b, "Counter") {
                cnt = crate::panel::next_counter(c);
            }
            entry["head"] = json!(r
                .get("head")
                .and_then(|h| h.as_str())
                .unwrap_or("")
                .lines()
                .next()
                .unwrap_or(""));
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
        (
            "init -> special signs -> io -> control",
            vec![
                ("ui_12540_sz.xml", 0, "close"),
                ("ui_io.xml", 700, "close"),
                ("ui_12540.xml", 700, "close"),
            ],
        ),
        ("init -> wait 1 s -> io", vec![("ui_io.xml", 1000, "close")]),
        (
            "init -> io with android-like headers",
            vec![("ui_io.xml", 0, "android")],
        ),
        ("init -> io with http/1.0", vec![("ui_io.xml", 0, "http10")]),
    ];
    let mut out = Vec::new();
    for (label, steps) in plan {
        out.push(sequence(&addr, group, label, &steps).await);
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
    json!({ "note": "read-only: only plain GETs without key/button codes; poller paused while probing", "results": out })
}

// ---------------------------------------------------------------- session/lock test

/// One probe session: plain GETs only, the counter follows the answered value plus one.
struct Sess {
    addr: String,
    name: &'static str,
    group: String,
    sid: String,
    cnt: String,
}

impl Sess {
    async fn open(
        addr: &str,
        name: &'static str,
        group: &str,
        log: &mut Vec<Value>,
    ) -> Option<Sess> {
        let r = raw_get(
            addr,
            &request("ui_init.xml", group, "00", "FFFF", addr, "close"),
            Duration::from_secs(6),
        )
        .await;
        let body = r
            .get("body")
            .and_then(|b| b.as_str())
            .unwrap_or("")
            .to_string();
        let flags: Vec<String> = [
            "Page_12540",
            "Page_12540_Revision",
            "Page_Button",
            "Page_LED",
        ]
        .iter()
        .filter_map(|f| tag(&body, f).map(|v| format!("{f}={v}")))
        .collect();
        match (tag(&body, "Session_ID"), tag(&body, "Counter")) {
            (Some(sid), Some(c)) => {
                log.push(json!({ "session": name, "step": "init", "group": group, "sid": sid, "flags": flags }));
                Some(Sess {
                    addr: addr.into(),
                    name,
                    group: group.into(),
                    sid: sid.into(),
                    cnt: crate::panel::next_counter(c),
                })
            }
            _ => {
                log.push(json!({ "session": name, "step": "init", "group": group, "failed": true, "answer": r }));
                None
            }
        }
    }

    async fn get(&mut self, page: &str) -> Result<String, String> {
        let r = raw_get(
            &self.addr,
            &request(page, &self.group, &self.sid, &self.cnt, &self.addr, "close"),
            Duration::from_secs(8),
        )
        .await;
        if !r.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
            return Err(r
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("error")
                .to_string());
        }
        let b = r
            .get("body")
            .and_then(|b| b.as_str())
            .unwrap_or("")
            .to_string();
        if let Some(c) = tag(&b, "Counter") {
            self.cnt = crate::panel::next_counter(c);
        }
        Ok(b)
    }

    async fn io(&mut self, label: &str, log: &mut Vec<Value>) {
        let e = match self.get("ui_io.xml").await {
            Ok(b) => {
                json!({ "ok": true, "ledValues": b.matches("_ON").count() + b.matches("_OFF").count() + b.matches("_BLINK").count() })
            }
            Err(e) => json!({ "ok": false, "error": e }),
        };
        log.push(json!({ "session": self.name, "step": format!("io {label}"), "result": e }));
    }

    async fn keypad(&mut self, label: &str, log: &mut Vec<Value>) {
        let e = match self.get("ui_12540.xml").await {
            Ok(b) => {
                let leds: Vec<String> = [
                    "Betrieb",
                    "S_Alarm",
                    "S_Stoerung",
                    "Sys_Stoerung",
                    "Einbruch",
                    "Sabotage",
                    "Ueberfall",
                    "Pol_Not",
                ]
                .iter()
                .filter_map(|n| {
                    let open = format!("<{n}>");
                    let at = b.find(&open)?;
                    tag(&b[at..], "LED").map(|v| format!("{n}={v}"))
                })
                .collect();
                json!({ "ok": true, "locked": tag(&b, "Locked"), "statusLeds": leds, "hasDisplay": b.contains("<Text_80_01>"),
                        "head": if leds.is_empty() { b.chars().take(300).collect::<String>() } else { String::new() } })
            }
            Err(e) => json!({ "ok": false, "error": e }),
        };
        log.push(json!({ "session": self.name, "step": format!("keypad {label}"), "result": e }));
    }
}

/// Answers: does an io-only session take the keypad? Does a locked keypad answer still carry
/// the status LEDs? What does a second user group get? Only plain GETs, never a key code.
pub async fn lock_test(host: &str, port: u16, group: &str, group2: &str) -> Value {
    let addr = format!("{host}:{port}");
    let mut log = Vec::new();
    let pause = || tokio::time::sleep(Duration::from_millis(800));
    // 1) C polls only the button/LED page
    let mut c = Sess::open(&addr, "C", group, &mut log).await;
    if let Some(c) = c.as_mut() {
        for i in 1..=4 {
            c.io(&format!("#{i}"), &mut log).await;
            pause().await;
        }
    }
    // 2) A opens the keypad afterwards: locked or free?
    let mut a = Sess::open(&addr, "A", group, &mut log).await;
    if let Some(a) = a.as_mut() {
        for i in 1..=3 {
            a.keypad(&format!("#{i}"), &mut log).await;
            pause().await;
        }
    }
    // 3) C asks for the keypad while A has it: locked, and are the status LEDs still there?
    if let Some(c) = c.as_mut() {
        if let Some(a) = a.as_mut() {
            a.keypad("keepalive", &mut log).await;
        }
        c.keypad("second user", &mut log).await;
        c.io("after", &mut log).await;
    }
    // 4) a second user group
    if !group2.is_empty() && group2 != group {
        if let Some(mut b) = Sess::open(&addr, "B", group2, &mut log).await {
            b.io("group2", &mut log).await;
            if let Some(a) = a.as_mut() {
                a.keypad("keepalive", &mut log).await;
            }
            b.keypad("group2", &mut log).await;
        }
    }
    json!({ "note": "read-only: plain GETs only, no key or button codes", "group": group, "group2": group2, "log": log })
}
