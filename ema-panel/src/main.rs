//! ema-panel – Home Assistant add-on backend.
//!
//! * A single background task owns the panel session and polls it (read-only).
//! * Commands (key presses) are only ever queued by an explicit `POST /api/key`,
//!   and only when `control_enabled` is on and the key is on the fixed whitelist.
//! * The web UI (one HTML file) is embedded in the binary.

mod config;
mod panel;
mod probe;

use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use config::Options;
use panel::{Button, Led, LedVal, Panel};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

const INDEX_HTML: &str = include_str!("../web/index.html");
const VERSION: &str = env!("CARGO_PKG_VERSION");
const MAX_RAW: usize = 24_000;

// ---------------------------------------------------------------- shared state

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    connected: bool,
    mode: &'static str,
    error: Option<String>,
    control: bool,
    locked: bool,
    line1: String,
    line2: String,
    revision: Option<String>,
    cursor_pos: Option<String>,
    cursor_blink: Option<String>,
    leds: Vec<Led>,
    buttons: Vec<Button>,
    indicators: Vec<Button>,
    session_id: Option<String>,
    counter: String,
    updated_at: u64,
    raw: Raw,
}

#[derive(Debug, Clone, Default, Serialize)]
struct Raw {
    init: String,
    io: String,
    control: String,
}

enum Cmd {
    /// Keypad key code (sent on ui_12540).
    Key(String),
    /// Function button code "01".."08" (sent on ui_io).
    Taster(String),
}

struct Shared {
    snap: RwLock<Snapshot>,
    tx: mpsc::UnboundedSender<Cmd>,
    opts: Options,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn clip(s: &str) -> String {
    if s.len() > MAX_RAW {
        let mut end = MAX_RAW;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s[..end].to_string()
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------- poller

async fn run_poller(shared: Arc<Shared>, mut rx: mpsc::UnboundedReceiver<Cmd>) {
    let o = &shared.opts;
    if o.panel_host.is_empty() {
        set_error(&shared, "Kein Panel-Host in den Add-on-Optionen eingetragen.".into(), false);
        return;
    }
    println!("polling panel {}:{} (group {})", o.panel_host, o.panel_port, o.user_group);
    let mut panel = Panel::new(&o.panel_host, o.panel_port, &o.user_group);
    let mut init: Option<panel::InitInfo> = None;
    let mut errors = 0u32;
    let mut backoff = 2u64;
    let mut fast = 0u32;

    loop {
        // ---- (re)start a session when needed
        if init.is_none() {
            match panel.init().await {
                Ok((info, xml)) => {
                    println!("session started ({} led names, {} button names)", info.led_names.len(), info.button_names.len());
                    {
                        let mut s = shared.snap.write().unwrap();
                        s.raw.init = clip(&xml);
                        s.session_id = Some(info.session_id.clone());
                    }
                    init = Some(info);
                    errors = 0;
                    backoff = 2;
                }
                Err(e) => {
                    set_error(&shared, e, false);
                    tokio::time::sleep(Duration::from_secs(backoff)).await;
                    backoff = (backoff * 2).min(30);
                    continue;
                }
            }
        }
        let info = init.clone().unwrap();

        // ---- take queued commands (never retried: a lost press is better than a doubled one)
        let mut key_cmd = String::new();
        let mut taster_cmd = String::new();
        while let Ok(c) = rx.try_recv() {
            fast = 4;
            match c {
                Cmd::Key(code) => {
                    key_cmd.push('T');
                    key_cmd.push_str(&code);
                }
                Cmd::Taster(code) => {
                    taster_cmd.push('T');
                    taster_cmd.push_str(&code);
                }
            }
        }

        // ---- one poll cycle
        let res: Result<(), String> = async {
            let mut io = None;
            let mut ctl = None;
            if info.page_buttons || info.page_leds || !taster_cmd.is_empty() {
                io = Some(panel.read_io(&taster_cmd).await?);
            }
            if info.page_control || !key_cmd.is_empty() {
                ctl = Some(panel.read_control(&key_cmd).await?);
            }
            apply(&shared, &info, io, ctl, panel.session_id(), panel.counter());
            Ok(())
        }
        .await;

        match res {
            Ok(()) => errors = 0,
            Err(e) => {
                errors += 1;
                eprintln!("poll error ({errors}): {e}");
                set_error(&shared, e, errors < 3);
                if errors >= 3 {
                    init = None; // new session
                    errors = 0;
                }
            }
        }

        let wait = if fast > 0 {
            fast -= 1;
            300
        } else {
            o.poll_interval_ms
        };
        tokio::time::sleep(Duration::from_millis(wait)).await;
    }
}

fn set_error(shared: &Shared, msg: String, keep_connected: bool) {
    let mut s = shared.snap.write().unwrap();
    s.error = Some(msg);
    if !keep_connected {
        s.connected = false;
    }
    s.control = shared.opts.control_enabled;
}

fn apply(
    shared: &Shared,
    info: &panel::InitInfo,
    io: Option<(panel::IoInfo, String)>,
    ctl: Option<(panel::ControlInfo, String)>,
    session_id: &str,
    counter: &str,
) {
    let mut s = shared.snap.write().unwrap();
    s.connected = true;
    s.error = None;
    s.mode = "plaintext";
    s.control = shared.opts.control_enabled;
    s.session_id = Some(session_id.to_string());
    s.counter = counter.to_string();
    s.updated_at = now_ms();

    if let Some((c, xml)) = ctl {
        s.locked = c.locked;
        s.line1 = c.line1;
        s.line2 = c.line2;
        s.cursor_pos = c.cursor_pos;
        s.cursor_blink = c.cursor_blink;
        if c.revision.is_some() {
            s.revision = c.revision;
        }
        s.raw.control = clip(&xml);
        // status LEDs of the keypad page come first, then the panel's own LED page
        let mut leds: Vec<Led> = c
            .leds
            .into_iter()
            .map(|(tag, v)| Led { name: panel::led_label(&tag), tag, color: v.color, state: v.state })
            .collect();
        s.leds.retain(|l| l.tag.starts_with("page:"));
        leds.extend(s.leds.drain(..));
        s.leds = leds;
    }

    if let Some((io, xml)) = io {
        s.raw.io = clip(&xml);
        // LED page -> named by the panel
        s.leds.retain(|l| !l.tag.starts_with("page:"));
        for (i, v) in io.leds.iter().enumerate() {
            let name = info.led_names.get(i).cloned().unwrap_or_else(|| format!("LED {}", i + 1));
            s.leds.push(Led { name, tag: format!("page:{}", i + 1), color: v.color.clone(), state: v.state.clone() });
        }
        // button page: first 8 = Taster, last 3 = display-only indicators
        let mk = |i: usize, v: Option<&LedVal>| {
            let (tag, name) = info
                .button_names
                .get(i)
                .cloned()
                .unwrap_or_else(|| (format!("B{}", i + 1), format!("Taste {}", i + 1)));
            Button { index: (i + 1) as u8, name, tag, led: v.cloned() }
        };
        s.buttons = (0..8).filter(|i| *i < info.button_names.len().max(io.buttons.len())).map(|i| mk(i, io.buttons.get(i))).collect();
        s.indicators = (8..11).filter(|i| *i < info.button_names.len().max(io.buttons.len())).map(|i| mk(i, io.buttons.get(i))).collect();
    }
}

// ---------------------------------------------------------------- http

fn no_store(mut r: Response) -> Response {
    r.headers_mut().insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    r
}

async fn index() -> Response {
    no_store(Html(INDEX_HTML).into_response())
}

async fn health(State(sh): State<Arc<Shared>>) -> Response {
    let connected = sh.snap.read().unwrap().connected;
    no_store(
        Json(json!({
            "ok": true,
            "version": VERSION,
            "panelConfigured": !sh.opts.panel_host.is_empty(),
            "connected": connected,
            "control": sh.opts.control_enabled,
            "mode": "plaintext",
        }))
        .into_response(),
    )
}

async fn state(State(sh): State<Arc<Shared>>) -> Response {
    let mut snap = sh.snap.read().unwrap().clone();
    snap.control = sh.opts.control_enabled;
    snap.mode = "plaintext";
    snap.raw = Raw::default(); // keep the poll payload small; raw is in /api/connect-test
    no_store(Json(snap).into_response())
}

/// Read-only diagnostics: last raw XML of each page (no new session, no commands).
async fn connect_test(State(sh): State<Arc<Shared>>) -> Response {
    let s = sh.snap.read().unwrap().clone();
    let pages: Vec<_> = [("ui_init.xml", &s.raw.init), ("ui_io.xml", &s.raw.io), ("ui_12540.xml", &s.raw.control)]
        .into_iter()
        .filter(|(_, x)| !x.is_empty())
        .map(|(p, x)| json!({ "page": p, "status": 200, "xml": x }))
        .collect();
    let state = json!({ "line1": s.line1, "line2": s.line2, "revision": s.revision, "cursorPos": s.cursor_pos, "cursorBlink": s.cursor_blink });
    no_store(
        Json(json!({
            "ok": s.connected,
            "host": format!("{}:{}", sh.opts.panel_host, sh.opts.panel_port),
            "mode": "plaintext",
            "sessionId": s.session_id,
            "counter": s.counter,
            "error": s.error,
            "state": state,
            "pages": pages,
        }))
        .into_response(),
    )
}

/// Read-only transport diagnostics (plain GETs without key codes, see probe.rs).
async fn probe_route(State(sh): State<Arc<Shared>>) -> Response {
    let r = probe::run(&sh.opts.panel_host, sh.opts.panel_port, &sh.opts.user_group).await;
    no_store(Json(r).into_response())
}

#[derive(Deserialize)]
struct KeyReq {
    key: Option<String>,
    taster: Option<u8>,
}

fn reject(code: StatusCode, msg: &str) -> Response {
    no_store((code, Json(json!({ "ok": false, "error": msg }))).into_response())
}

/// The only route that can make the panel do something. Layers of protection:
/// 1) add-on option `control_enabled` (default off)  2) connected session required
/// 3) fixed key whitelist (no panic/Überfall keys exist)  4) JSON content-type only
/// (a cross-site form post can't trigger it) 5) every press is logged.
async fn key(State(sh): State<Arc<Shared>>, Json(req): Json<KeyReq>) -> Response {
    if !sh.opts.control_enabled {
        return reject(StatusCode::FORBIDDEN, "Steuern ist in den Add-on-Optionen ausgeschaltet.");
    }
    if !sh.snap.read().unwrap().connected {
        return reject(StatusCode::SERVICE_UNAVAILABLE, "Keine Verbindung zur Anlage.");
    }
    let cmd = match (req.key.as_deref(), req.taster) {
        (Some(k), None) => match panel::keypad_code(k) {
            Some(code) => {
                println!("COMMAND key={k} code={code}");
                Cmd::Key(code.to_string())
            }
            None => return reject(StatusCode::BAD_REQUEST, "Unbekannte Taste."),
        },
        (None, Some(t)) => match panel::taster_code(t) {
            Some(code) => {
                println!("COMMAND taster={t} code={code}");
                Cmd::Taster(code)
            }
            None => return reject(StatusCode::BAD_REQUEST, "Unbekannter Taster."),
        },
        _ => return reject(StatusCode::BAD_REQUEST, "Genau ein Feld 'key' oder 'taster' angeben."),
    };
    if sh.tx.send(cmd).is_err() {
        return reject(StatusCode::INTERNAL_SERVER_ERROR, "Befehlswarteschlange nicht verfügbar.");
    }
    no_store(Json(json!({ "ok": true })).into_response())
}

// ---------------------------------------------------------------- main

#[tokio::main]
async fn main() {
    let opts = Options::load();
    println!(
        "ema-panel {VERSION} · host='{}' port={} group={} control={} poll={}ms",
        opts.panel_host, opts.panel_port, opts.user_group, opts.control_enabled, opts.poll_interval_ms
    );
    if opts.control_enabled {
        println!("WARNUNG: Steuern ist aktiviert – Tastendrücke werden an die Anlage gesendet.");
    } else {
        println!("Nur-Lesen-Modus: es werden keine Befehle an die Anlage gesendet.");
    }

    let (tx, rx) = mpsc::unbounded_channel();
    let shared = Arc::new(Shared {
        snap: RwLock::new(Snapshot { mode: "plaintext", control: opts.control_enabled, error: Some("Verbinde …".into()), ..Default::default() }),
        tx,
        opts: opts.clone(),
    });

    tokio::spawn(run_poller(shared.clone(), rx));

    let app = Router::new()
        .route("/", get(index))
        .route("/api/health", get(health))
        .route("/api/state", get(state))
        .route("/api/connect-test", get(connect_test))
        .route("/api/probe", get(probe_route))
        .route("/api/key", post(key))
        .with_state(shared);

    let port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8099);
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await.expect("bind");
    println!("listening on :{port}");
    axum::serve(listener, app).with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await
    .expect("server");
}
