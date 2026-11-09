//! ema-panel – backend of the alarm keypad, as Home Assistant add-on or standalone container.
//!
//! * A single background task owns the panel session and polls it (read-only).
//! * Commands (key presses) are only ever queued by an explicit `POST /api/key`,
//!   and only when `control_enabled` is on and the key is on the fixed whitelist.
//! * The web UI (one HTML file) is embedded in the binary.

mod config;
mod panel;
mod probe;
mod settings;
mod systems;

use axum::{
    extract::{Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use settings::{Mode, Settings};
use config::Options;
use panel::{Button, Led, LedVal, Panel};
use serde::{Deserialize, Serialize};
use serde_json::json;
use systems::{System, SystemInput};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

const INDEX_HTML: &str = include_str!("../web/index.html");
const MANIFEST: &str = include_str!("../web/manifest.webmanifest");
const SW_JS: &str = include_str!("../web/sw.js");
const ICON_SVG: &str = include_str!("../web/icon.svg");
const ICON_192: &[u8] = include_bytes!("../web/icon-192.png");
const ICON_512: &[u8] = include_bytes!("../web/icon-512.png");
const ICON_MASKABLE: &[u8] = include_bytes!("../web/icon-maskable-512.png");
const ICON_APPLE: &[u8] = include_bytes!("../web/apple-touch-icon.png");
const COOKIE: &str = "ema_session";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const MAX_RAW: usize = 24_000;
/// The panel allows only one user at its keypad at a time, so the add-on only holds a
/// session while somebody asked for it and lets go after this much time without activity.
const SESSION_MS: u64 = 5 * 60 * 1000;

// ---------------------------------------------------------------- shared state

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    connected: bool,
    /// which system is (being) connected
    system_id: Option<String>,
    system_name: Option<String>,
    /// someone asked for a connection (and it has not timed out / been closed yet)
    active: bool,
    /// when the wanted connection expires (ms since epoch), 0 when inactive
    expires_at: u64,
    /// inactivity time after which the connection is closed
    idle_ms: u64,
    mode: &'static str,
    error: Option<String>,
    control: bool,
    locked: bool,
    line1: String,
    line2: String,
    /// per-cell LCD content: a string (character) or a number (custom glyph index)
    cells1: Vec<panel::Cell>,
    cells2: Vec<panel::Cell>,
    /// 5x8 bitmaps of the custom glyphs (rows as bytes), from ui_12540_sz
    glyphs: Vec<Vec<u8>>,
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
    /// UI-editable settings (control switch, poll interval, password), persisted in /data
    settings: Settings,
    /// failed logins in a row (slows down guessing)
    login_fails: AtomicU64,
    /// set while /api/probe runs so the poller leaves the panel alone
    paused: AtomicBool,
    /// the managed list of alarm systems (persisted in /data)
    systems: systems::Store,
    /// the system a connection is wanted for
    target: RwLock<Option<System>>,
    /// connection wanted until this time (ms since epoch); 0 = not wanted, poller stays idle
    wanted_until: AtomicU64,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn wanted(shared: &Shared) -> bool {
    shared.wanted_until.load(Ordering::Relaxed) > now_ms()
}

/// Keep the connection for another `SESSION_MS` (connect, key press, explicit extend).
fn touch(shared: &Shared) {
    shared.wanted_until.store(now_ms() + SESSION_MS, Ordering::Relaxed);
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
    let mut cur: Option<System> = None;
    let mut panel = Panel::new("", 0, "1");
    let mut init: Option<panel::InitInfo> = None;
    let mut errors = 0u32;
    let mut backoff = 2u64;
    let mut fast = 0u32;
    let mut idle_done = true;

    loop {
        // ---- nobody asked for a connection: hold no session, show nothing stale
        if !wanted(&shared) {
            if !idle_done {
                println!("connection closed (disconnect or {} s without activity)", SESSION_MS / 1000);
                init = None;
                cur = None;
                errors = 0;
                backoff = 2;
                while rx.try_recv().is_ok() {} // never replay old commands later
                let mut s = shared.snap.write().unwrap();
                *s = Snapshot { mode: "plaintext", control: shared.settings.control(), ..Default::default() };
                idle_done = true;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
            continue;
        }
        idle_done = false;

        // ---- which system? a different (or edited) one means a fresh session
        let target = shared.target.read().unwrap().clone();
        let Some(target) = target else {
            shared.wanted_until.store(0, Ordering::Relaxed);
            continue;
        };
        if cur.as_ref() != Some(&target) {
            println!("polling {} ({}:{}, group {})", target.name, target.host, target.port, target.group);
            panel = Panel::new(&target.host, target.port, &target.group);
            init = None;
            errors = 0;
            backoff = 2;
            while rx.try_recv().is_ok() {}
            *shared.snap.write().unwrap() = Snapshot { mode: "plaintext", control: shared.settings.control(), ..Default::default() };
            cur = Some(target);
        }
        if shared.paused.load(Ordering::Relaxed) {
            init = None; // sessions are cheap, a fresh one after the probe is safest
            tokio::time::sleep(Duration::from_millis(250)).await;
            continue;
        }
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
                    // like the original app: fetch the custom glyph bitmaps once per session
                    match panel.read_signs().await {
                        Ok(g) => shared.snap.write().unwrap().glyphs = g,
                        Err(e) => eprintln!("glyphs not available: {e}"),
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
            shared.settings.poll_ms()
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
    s.control = shared.settings.control();
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
    s.control = shared.settings.control();
    s.session_id = Some(session_id.to_string());
    s.counter = counter.to_string();
    s.updated_at = now_ms();

    if let Some((c, xml)) = ctl {
        s.locked = c.locked;
        s.line1 = c.line1;
        s.line2 = c.line2;
        s.cells1 = c.cells1;
        s.cells2 = c.cells2;
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
            "panelConfigured": !sh.systems.list().is_empty(),
            "connected": connected,
            "control": sh.settings.control(),
            "mode": "plaintext",
            "runMode": sh.settings.mode,
            "authRequired": sh.settings.auth_required(),
        }))
        .into_response(),
    )
}

async fn state(State(sh): State<Arc<Shared>>) -> Response {
    let mut snap = sh.snap.read().unwrap().clone();
    snap.control = sh.settings.control();
    snap.mode = "plaintext";
    let until = sh.wanted_until.load(Ordering::Relaxed);
    snap.active = until > now_ms();
    snap.expires_at = if snap.active { until } else { 0 };
    snap.idle_ms = SESSION_MS;
    if snap.active {
        if let Some(t) = sh.target.read().unwrap().as_ref() {
            snap.system_id = Some(t.id.clone());
            snap.system_name = Some(t.name.clone());
        }
    }
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
            "host": sh.target.read().unwrap().as_ref().map(|t| format!("{}:{}", t.host, t.port)).unwrap_or_default(),
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
    let Some(t) = sh.target.read().unwrap().clone().or_else(|| sh.systems.first()) else {
        return reject(StatusCode::SERVICE_UNAVAILABLE, "Noch keine Anlage angelegt.");
    };
    sh.paused.store(true, Ordering::Relaxed);
    tokio::time::sleep(Duration::from_secs(3)).await;
    let r = probe::run(&t.host, t.port, &t.group).await;
    sh.paused.store(false, Ordering::Relaxed);
    no_store(Json(r).into_response())
}

#[derive(Deserialize, Default)]
struct ConnectReq {
    id: Option<String>,
}

/// Ask the add-on to open (or keep open) the session to a system. JSON only, like /api/key.
async fn connect(State(sh): State<Arc<Shared>>, Json(req): Json<ConnectReq>) -> Response {
    let chosen = match req.id.as_deref().filter(|i| !i.is_empty()) {
        Some(id) => match sh.systems.get(id) {
            Some(s) => Some(s),
            None => return reject(StatusCode::NOT_FOUND, "Anlage nicht gefunden."),
        },
        None => sh.target.read().unwrap().clone().and_then(|t| sh.systems.get(&t.id)).or_else(|| sh.systems.first()),
    };
    let Some(sys) = chosen else {
        return reject(StatusCode::SERVICE_UNAVAILABLE, "Noch keine Anlage angelegt. Lege zuerst eine an.");
    };
    if !wanted(&sh) || sh.target.read().unwrap().as_ref().map(|t| t.id != sys.id).unwrap_or(true) {
        println!("connection requested for {}", sys.name);
    }
    *sh.target.write().unwrap() = Some(sys);
    touch(&sh);
    no_store(Json(json!({ "ok": true, "expiresAt": sh.wanted_until.load(Ordering::Relaxed) })).into_response())
}

/// Let go of the panel so someone else (e.g. the original app) can use the keypad.
async fn disconnect(State(sh): State<Arc<Shared>>, Json(_): Json<serde_json::Value>) -> Response {
    sh.wanted_until.store(0, Ordering::Relaxed);
    no_store(Json(json!({ "ok": true })).into_response())
}

// ---- managing the list of systems (names/addresses only, never touches the panel)

async fn systems_list(State(sh): State<Arc<Shared>>) -> Response {
    let active = if wanted(&sh) { sh.target.read().unwrap().as_ref().map(|t| t.id.clone()) } else { None };
    no_store(Json(json!({ "ok": true, "systems": sh.systems.list(), "activeId": active })).into_response())
}

async fn systems_save(State(sh): State<Arc<Shared>>, Json(input): Json<SystemInput>) -> Response {
    match sh.systems.upsert(input) {
        Ok(sys) => {
            println!("system saved: {} ({}:{})", sys.name, sys.host, sys.port);
            // an edited active system is picked up by the poller (it reconnects when the entry changed)
            if let Some(t) = sh.target.write().unwrap().as_mut() {
                if t.id == sys.id {
                    *t = sys.clone();
                }
            }
            no_store(Json(json!({ "ok": true, "system": sys })).into_response())
        }
        Err(e) => reject(StatusCode::BAD_REQUEST, &e),
    }
}

#[derive(Deserialize)]
struct IdReq {
    id: String,
}

async fn systems_delete(State(sh): State<Arc<Shared>>, Json(req): Json<IdReq>) -> Response {
    match sh.systems.delete(&req.id) {
        Ok(()) => {
            let was_target = sh.target.read().unwrap().as_ref().map(|t| t.id == req.id).unwrap_or(false);
            if was_target {
                sh.wanted_until.store(0, Ordering::Relaxed);
                *sh.target.write().unwrap() = None;
            }
            no_store(Json(json!({ "ok": true })).into_response())
        }
        Err(e) => reject(StatusCode::NOT_FOUND, &e),
    }
}

#[derive(Deserialize)]
struct OrderReq {
    ids: Vec<String>,
}

async fn systems_order(State(sh): State<Arc<Shared>>, Json(req): Json<OrderReq>) -> Response {
    match sh.systems.reorder(&req.ids) {
        Ok(()) => no_store(Json(json!({ "ok": true })).into_response()),
        Err(e) => reject(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
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
    if !sh.settings.control() {
        return reject(StatusCode::FORBIDDEN, "Steuern ist in den Einstellungen ausgeschaltet.");
    }
    if !wanted(&sh) || !sh.snap.read().unwrap().connected {
        return reject(StatusCode::SERVICE_UNAVAILABLE, "Keine Verbindung zur Anlage.");
    }
    touch(&sh); // a key press counts as activity
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

// ---------------------------------------------------------------- settings (UI)

fn now_s() -> u64 {
    now_ms() / 1000
}

async fn settings_get(State(sh): State<Arc<Shared>>) -> Response {
    no_store(
        Json(json!({
            "ok": true,
            "version": VERSION,
            "runMode": sh.settings.mode,
            "control": sh.settings.control(),
            "pollIntervalMs": sh.settings.poll_ms(),
            "passwordSet": sh.settings.has_password(),
            "authRequired": sh.settings.auth_required(),
        }))
        .into_response(),
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsReq {
    control: Option<bool>,
    poll_interval_ms: Option<u64>,
    /// current password, needed to turn control ON while a login is set
    password: Option<String>,
}

async fn settings_save(State(sh): State<Arc<Shared>>, Json(req): Json<SettingsReq>) -> Response {
    if let Some(on) = req.control {
        if on && !sh.settings.control() && sh.settings.auth_required() {
            let ok = req.password.as_deref().map(|p| sh.settings.check_password(p)).unwrap_or(false);
            if !ok {
                slow_down(&sh).await;
                return reject(StatusCode::FORBIDDEN, "Passwort falsch.");
            }
        }
        if on != sh.settings.control() {
            println!("{}", if on { "WARNUNG: Steuern wurde in der Oberfläche eingeschaltet." } else { "Steuern wurde in der Oberfläche ausgeschaltet (nur lesen)." });
        }
        if let Err(e) = sh.settings.set_control(on) {
            return reject(StatusCode::INTERNAL_SERVER_ERROR, &e);
        }
        sh.snap.write().unwrap().control = on;
    }
    if let Some(ms) = req.poll_interval_ms {
        if let Err(e) = sh.settings.set_poll(ms) {
            return reject(StatusCode::INTERNAL_SERVER_ERROR, &e);
        }
    }
    settings_get(State(sh)).await
}

// ---------------------------------------------------------------- login (standalone only)

fn cookie_token(headers: &HeaderMap) -> Option<String> {
    headers.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(';')).find_map(|c| {
        let (k, v) = c.trim().split_once('=')?;
        (k == COOKIE).then(|| v.to_string())
    })
}

fn logged_in(sh: &Shared, headers: &HeaderMap) -> bool {
    !sh.settings.auth_required() || cookie_token(headers).map(|t| sh.settings.verify_token(&t, now_s())).unwrap_or(false)
}

/// `Secure` only when the browser reached us over HTTPS (e.g. Cloudflare Tunnel, reverse proxy).
fn https(headers: &HeaderMap) -> bool {
    headers.get("x-forwarded-proto").and_then(|v| v.to_str().ok()).map(|v| v.eq_ignore_ascii_case("https")).unwrap_or(false)
}

fn with_cookie(mut r: Response, headers: &HeaderMap, value: &str, max_age: u64) -> Response {
    let c = format!(
        "{COOKIE}={value}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Strict{}",
        if https(headers) { "; Secure" } else { "" }
    );
    if let Ok(v) = HeaderValue::from_str(&c) {
        r.headers_mut().append(header::SET_COOKIE, v);
    }
    r
}

/// After a wrong password: wait, longer with every failure in a row (max 10 s).
async fn slow_down(sh: &Shared) {
    let n = sh.login_fails.fetch_add(1, Ordering::Relaxed) + 1;
    eprintln!("wrong password ({n} in a row)");
    tokio::time::sleep(Duration::from_millis((n * 700).min(10_000))).await;
}

async fn auth_status(State(sh): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    no_store(
        Json(json!({
            "ok": true,
            "runMode": sh.settings.mode,
            "authRequired": sh.settings.auth_required(),
            "passwordSet": sh.settings.has_password(),
            "loggedIn": logged_in(&sh, &headers),
        }))
        .into_response(),
    )
}

#[derive(Deserialize)]
struct LoginReq {
    password: String,
}

async fn login(State(sh): State<Arc<Shared>>, headers: HeaderMap, Json(req): Json<LoginReq>) -> Response {
    if !sh.settings.auth_required() {
        return no_store(Json(json!({ "ok": true })).into_response());
    }
    if !sh.settings.check_password(&req.password) {
        slow_down(&sh).await;
        return reject(StatusCode::UNAUTHORIZED, "Passwort falsch.");
    }
    sh.login_fails.store(0, Ordering::Relaxed);
    let token = sh.settings.issue_token(now_s());
    with_cookie(no_store(Json(json!({ "ok": true })).into_response()), &headers, &token, settings::SESSION_DAYS * 86_400)
}

async fn logout(headers: HeaderMap, Json(_): Json<serde_json::Value>) -> Response {
    with_cookie(no_store(Json(json!({ "ok": true })).into_response()), &headers, "", 0)
}

#[derive(Deserialize)]
struct PasswordReq {
    /// needed when a password is already set
    current: Option<String>,
    /// new password; empty or missing removes the login
    new: Option<String>,
}

async fn set_password(State(sh): State<Arc<Shared>>, headers: HeaderMap, Json(req): Json<PasswordReq>) -> Response {
    if sh.settings.mode != Mode::Standalone {
        return reject(StatusCode::BAD_REQUEST, "In Home Assistant meldet dich Home Assistant an, ein eigenes Passwort ist nicht nötig.");
    }
    if sh.settings.has_password() && !req.current.as_deref().map(|p| sh.settings.check_password(p)).unwrap_or(false) {
        slow_down(&sh).await;
        return reject(StatusCode::FORBIDDEN, "Aktuelles Passwort falsch.");
    }
    let new = req.new.as_deref().filter(|p| !p.is_empty());
    if let Err(e) = sh.settings.set_password(new) {
        return reject(StatusCode::BAD_REQUEST, &e);
    }
    sh.login_fails.store(0, Ordering::Relaxed);
    println!("{}", if new.is_some() { "login password set" } else { "login password removed" });
    let r = no_store(Json(json!({ "ok": true, "authRequired": sh.settings.auth_required() })).into_response());
    match new {
        // the cookie key changed: give this browser a fresh login so it stays signed in
        Some(_) => with_cookie(r, &headers, &sh.settings.issue_token(now_s()), settings::SESSION_DAYS * 86_400),
        None => with_cookie(r, &headers, "", 0),
    }
}

/// Every `/api/` route except health and login needs a valid cookie once a password is set.
async fn require_login(State(sh): State<Arc<Shared>>, req: Request, next: Next) -> Response {
    let p = req.uri().path();
    let open = !p.starts_with("/api/") || p == "/api/health" || p == "/api/auth/status" || p == "/api/auth/login" || p == "/api/auth/logout";
    if open || logged_in(&sh, req.headers()) {
        next.run(req).await
    } else {
        reject(StatusCode::UNAUTHORIZED, "Bitte anmelden.")
    }
}

// ---------------------------------------------------------------- static files (PWA)

fn asset(ct: &'static str, body: impl IntoResponse, cache: &'static str) -> Response {
    let mut r = body.into_response();
    r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static(ct));
    r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    r
}

const DAY: &str = "public, max-age=86400";

async fn manifest() -> Response {
    asset("application/manifest+json", MANIFEST, DAY)
}
async fn sw() -> Response {
    asset("text/javascript; charset=utf-8", SW_JS, "no-cache")
}
async fn icon_svg() -> Response {
    asset("image/svg+xml", ICON_SVG, DAY)
}
async fn icon_192() -> Response {
    asset("image/png", ICON_192, DAY)
}
async fn icon_512() -> Response {
    asset("image/png", ICON_512, DAY)
}
async fn icon_maskable() -> Response {
    asset("image/png", ICON_MASKABLE, DAY)
}
async fn icon_apple() -> Response {
    asset("image/png", ICON_APPLE, DAY)
}

// ---------------------------------------------------------------- main

#[tokio::main]
async fn main() {
    let opts = Options::load();
    let options_present = std::path::Path::new(&config::options_path()).exists();
    let mode = Mode::detect(&config::options_path());
    let data = config::data_dir();
    if let Err(e) = std::fs::create_dir_all(&data) {
        eprintln!("data folder {data} not usable: {e}");
    }
    let settings = Settings::load(&format!("{data}/settings.json"), mode, &opts, options_present);
    println!(
        "ema-panel {VERSION} · mode={mode:?} data={data} control={} poll={}ms login={}",
        settings.control(),
        settings.poll_ms(),
        if settings.auth_required() { "on" } else { "off" }
    );
    if settings.control() {
        println!("WARNUNG: Steuern ist aktiviert – Tastendrücke werden an die Anlage gesendet.");
    } else {
        println!("Nur-Lesen-Modus: es werden keine Befehle an die Anlage gesendet.");
    }
    if mode == Mode::Standalone && !settings.has_password() {
        println!("Hinweis: kein Passwort gesetzt. Wer die Seite erreicht, sieht die Anlage. Passwort in den Einstellungen festlegen.");
    }

    // first start only: the options (HA) or env vars become the first system, afterwards the UI manages the list
    let seed = (!opts.panel_host.is_empty()).then(|| System {
        id: "default".into(),
        name: "Meine Anlage".into(),
        host: opts.panel_host.clone(),
        port: opts.panel_port,
        group: opts.user_group.clone(),
    });
    let store = systems::Store::load(&std::env::var("SYSTEMS_PATH").unwrap_or_else(|_| format!("{data}/systems.json")), seed);
    println!("{} system(s) configured", store.list().len());

    let (tx, rx) = mpsc::unbounded_channel();
    let shared = Arc::new(Shared {
        snap: RwLock::new(Snapshot { mode: "plaintext", control: settings.control(), ..Default::default() }),
        tx,
        settings,
        login_fails: AtomicU64::new(0),
        paused: AtomicBool::new(false),
        wanted_until: AtomicU64::new(0),
        systems: store,
        target: RwLock::new(None),
    });

    tokio::spawn(run_poller(shared.clone(), rx));

    let app = Router::new()
        .route("/", get(index))
        .route("/index.html", get(index))
        .route("/manifest.webmanifest", get(manifest))
        .route("/sw.js", get(sw))
        .route("/icon.svg", get(icon_svg))
        .route("/icon-192.png", get(icon_192))
        .route("/icon-512.png", get(icon_512))
        .route("/icon-maskable-512.png", get(icon_maskable))
        .route("/apple-touch-icon.png", get(icon_apple))
        .route("/api/health", get(health))
        .route("/api/state", get(state))
        .route("/api/connect-test", get(connect_test))
        .route("/api/probe", get(probe_route))
        .route("/api/key", post(key))
        .route("/api/connect", post(connect))
        .route("/api/systems", get(systems_list).post(systems_save))
        .route("/api/systems/delete", post(systems_delete))
        .route("/api/systems/order", post(systems_order))
        .route("/api/disconnect", post(disconnect))
        .route("/api/settings", get(settings_get).post(settings_save))
        .route("/api/auth/status", get(auth_status))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/password", post(set_password))
        .layer(middleware::from_fn_with_state(shared.clone(), require_login))
        .with_state(shared);

    let port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8099);
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await.expect("bind");
    println!("listening on :{port}");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let ctrl_c = async {
                let _ = tokio::signal::ctrl_c().await;
            };
            #[cfg(unix)]
            let term = async {
                if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                    s.recv().await;
                }
            };
            #[cfg(not(unix))]
            let term = std::future::pending::<()>();
            tokio::select! { _ = ctrl_c => {}, _ = term => {} }
        })
        .await
        .expect("server");
}
