//! Protocol client for the panel's XML-over-HTTP interface.
//!
//! One `Panel` owns exactly one session (session id + running counter). All requests
//! are plain HTTP GETs:
//!   GET /<page>?B-GR=<group>&S-ID=<session>&CNT=<counter>[&T<code>...]
//! The counter for the next request is taken from each response.
//!
//! SAFETY: the only way anything is *sent* to the panel is the optional `&T<code>`
//! suffix, and only codes from the fixed whitelist below can ever be built.

use roxmltree::{Document, Node};
use serde::Serialize;
use std::time::Duration;

// ---------------------------------------------------------------- key tables

/// Keypad key id (as sent by the UI) -> panel key code. `None` = not allowed.
/// Verified against the original app (German labels): EIN=42, AUS=41.
pub fn keypad_code(key: &str) -> Option<&'static str> {
    Some(match key {
        "0" => "30",
        "1" => "31",
        "2" => "32",
        "3" => "33",
        "4" => "34",
        "5" => "35",
        "6" => "36",
        "7" => "37",
        "8" => "38",
        "9" => "39",
        "UP" | "▲" => "45",
        "DOWN" | "▼" => "48",
        "AUS" => "41",
        "EIN" => "42",
        "B" => "43",
        "G" => "44",
        "M" => "4A",
        "A" => "4B",
        "F" => "46",
        "E" => "49",
        "Revision" => "47",
        _ => return None,
    })
}

/// Function button 1..=8 -> code "01".."08".
pub fn taster_code(index_1_based: u8) -> Option<String> {
    (1..=8).contains(&index_1_based).then(|| format!("{index_1_based:02}"))
}

// ---------------------------------------------------------------- data model

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LedVal {
    pub color: String, // red | green | yellow | grey
    pub state: String, // on | off | blink
}

#[derive(Debug, Clone, Serialize)]
pub struct Led {
    pub name: String,
    pub tag: String,
    pub color: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Button {
    pub index: u8,
    pub name: String,
    pub tag: String,
    pub led: Option<LedVal>,
}

#[derive(Debug, Clone, Default)]
pub struct InitInfo {
    pub session_id: String,
    pub counter: String,
    pub led_names: Vec<String>,
    /// (tag, display name) for the 11 button-page entries: 8 Taster + 3 indicators.
    pub button_names: Vec<(String, String)>,
    pub page_control: bool,
    pub page_buttons: bool,
    pub page_leds: bool,
}

#[derive(Debug, Clone, Default)]
pub struct IoInfo {
    pub counter: String,
    pub buttons: Vec<LedVal>,
    pub leds: Vec<LedVal>,
}

#[derive(Debug, Clone, Default)]
pub struct ControlInfo {
    pub counter: String,
    pub locked: bool,
    pub leds: Vec<(String, LedVal)>,
    pub line1: String,
    pub line2: String,
    pub cursor_pos: Option<String>,
    pub cursor_blink: Option<String>,
    pub revision: Option<String>,
}

// ---------------------------------------------------------------- parsing

fn find<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    node.descendants().find(|n| n.is_element() && n.tag_name().name() == name)
}

fn text_of(node: Node) -> String {
    node.text().unwrap_or("").trim().to_string()
}

fn child_text(node: Node, name: &str) -> Option<String> {
    find(node, name).map(text_of)
}

fn element_children<'a, 'i>(node: Node<'a, 'i>) -> Vec<Node<'a, 'i>> {
    node.children().filter(|n| n.is_element()).collect()
}

/// "GREEN_ON" / "RED_BLINK_0" / "YELLOW_OFF" -> LedVal.
pub fn parse_led(v: &str) -> LedVal {
    let up = v.trim().to_ascii_uppercase();
    let color = match up.split('_').next().unwrap_or("") {
        "RED" => "red",
        "GREEN" => "green",
        "YELLOW" => "yellow",
        _ => "grey",
    };
    let state = if up.contains("BLINK") {
        "blink"
    } else if up.ends_with("_ON") || up.contains("_ON_") {
        "on"
    } else {
        "off"
    };
    LedVal { color: color.into(), state: state.into() }
}

/// Map a character code of the panel's LCD to a Unicode char.
/// Printable ASCII is identical; the upper half follows Latin-1 plus the common
/// HD44780 "A00" umlaut positions (0xE1/0xEF/0xF5/0xE2), which don't clash with
/// anything a German panel would show. Codes < 0x20 are custom glyphs -> blank.
fn lcd_char(b: u8) -> char {
    match b {
        0x20..=0x7E => b as char,
        0xE1 => 'ä',
        0xEF => 'ö',
        0xF5 => 'ü',
        0xE2 => 'ß',
        0xA0..=0xFF => b as char, // Latin-1
        _ => ' ',
    }
}

/// Display text arrives as comma-separated hex codes ("48,61,6C,..."). Anything
/// that doesn't look like that is returned unchanged.
pub fn decode_lcd(raw: &str) -> String {
    let t = raw.trim();
    if t.is_empty() {
        return String::new();
    }
    let parts: Vec<&str> = t.split(',').map(str::trim).collect();
    let all_hex = parts.iter().all(|p| (1..=2).contains(&p.len()) && p.bytes().all(|c| c.is_ascii_hexdigit()));
    if !all_hex {
        return t.to_string();
    }
    parts
        .iter()
        .map(|p| lcd_char(u8::from_str_radix(p, 16).unwrap_or(0x20)))
        .collect::<String>()
        .trim_end()
        .to_string()
}

fn parse_doc(xml: &str) -> Result<Document<'_>, String> {
    Document::parse(xml).map_err(|e| format!("XML nicht lesbar: {e}"))
}

pub fn parse_init(xml: &str) -> Result<InitInfo, String> {
    let doc = parse_doc(xml)?;
    let root = find(doc.root_element(), "ui_init").unwrap_or_else(|| doc.root_element());
    let init = find(root, "Init").ok_or("Antwort enthält kein <Init>")?;
    let session_id = child_text(init, "Session_ID").ok_or("keine Session_ID in der Antwort")?;
    let counter = child_text(init, "Counter").ok_or("kein Counter in der Antwort")?;
    if session_id.is_empty() {
        return Err("leere Session_ID".into());
    }
    let mut info = InitInfo { session_id, counter, page_control: true, page_buttons: true, page_leds: true, ..Default::default() };

    let decode_name = |s: String| if s.contains(',') { decode_lcd(&s) } else { s };

    if let Some(lp) = find(root, "LED_Page") {
        info.led_names = element_children(lp).into_iter().map(|n| decode_name(text_of(n))).collect();
    }
    if let Some(bp) = find(root, "Button_Page") {
        for n in element_children(bp) {
            let tag = n.tag_name().name().to_string();
            let val = text_of(n);
            match tag.as_str() {
                "Page_12540" => info.page_control = !val.eq_ignore_ascii_case("off"),
                "Page_Button" => info.page_buttons = !val.eq_ignore_ascii_case("off"),
                "Page_LED" => info.page_leds = !val.eq_ignore_ascii_case("off"),
                t if t.starts_with("Page_") => {}
                _ => info.button_names.push((tag, decode_name(val))),
            }
        }
    }
    Ok(info)
}

pub fn parse_io(xml: &str) -> Result<IoInfo, String> {
    let doc = parse_doc(xml)?;
    let root = find(doc.root_element(), "ui_io").ok_or("Antwort enthält kein <ui_io>")?;
    let counter = child_text(root, "Counter").ok_or("kein Counter in der Antwort")?;
    let vals = |name: &str| -> Vec<LedVal> {
        find(root, name)
            .map(|n| element_children(n).into_iter().map(|c| parse_led(&text_of(c))).collect())
            .unwrap_or_default()
    };
    Ok(IoInfo { counter, buttons: vals("Button_Page"), leds: vals("LED_Page") })
}

pub fn parse_control(xml: &str) -> Result<ControlInfo, String> {
    let doc = parse_doc(xml)?;
    let root = find(doc.root_element(), "ui_12540").ok_or("Antwort enthält kein <ui_12540>")?;
    let counter = child_text(root, "Counter").ok_or("kein Counter in der Antwort")?;
    let mut info = ControlInfo { counter, ..Default::default() };
    info.locked = child_text(root, "Locked").map(|v| v.eq_ignore_ascii_case("yes")).unwrap_or(false);
    if let Some(led) = root.children().find(|n| n.is_element() && n.tag_name().name() == "LED") {
        for c in element_children(led) {
            let tag = c.tag_name().name().to_string();
            let v = find(c, "LED").map(text_of).unwrap_or_else(|| text_of(c));
            info.leds.push((tag, parse_led(&v)));
        }
    }
    if let Some(d) = find(root, "Display") {
        info.line1 = decode_lcd(&child_text(d, "Text_80_01").unwrap_or_default());
        info.line2 = decode_lcd(&child_text(d, "Text_80_02").unwrap_or_default());
        info.cursor_pos = child_text(d, "Cursor_Pos");
        info.cursor_blink = child_text(d, "Cursor_Blink");
    }
    info.revision = child_text(root, "Revision");
    Ok(info)
}

/// German label for the fixed status LEDs of the keypad page.
pub fn led_label(tag: &str) -> String {
    match tag {
        "Betrieb" => "Betrieb",
        "S_Alarm" => "Sum. Alarm",
        "S_Stoerung" => "Sum. Störung",
        "Sys_Stoerung" => "Sys. Störung",
        "Einbruch" => "Einbruch",
        "Sabotage" => "Sabotage",
        "Ueberfall" => "Überfall",
        "Pol_Not" => "Pol.-Notruf",
        other => other,
    }
    .to_string()
}

// ---------------------------------------------------------------- transport

pub struct Panel {
    http: reqwest::Client,
    base: String,
    group: String,
    sid: String,
    cnt: String,
}

impl Panel {
    pub fn new(host: &str, port: u16, group: &str) -> Panel {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(4))
            .timeout(Duration::from_secs(8))
            .pool_max_idle_per_host(0)
            .build()
            .expect("http client");
        Panel { http, base: format!("http://{host}:{port}"), group: group.to_string(), sid: "00".into(), cnt: "FFFF".into() }
    }

    pub fn session_id(&self) -> &str {
        &self.sid
    }
    pub fn counter(&self) -> &str {
        &self.cnt
    }

    /// One GET. `command` is an already-validated `T..` suffix or empty.
    async fn get(&self, page: &str, command: &str) -> Result<(u16, String), String> {
        let mut url = format!("{}/{page}?B-GR={}&S-ID={}&CNT={}", self.base, self.group, self.sid, self.cnt);
        if !command.is_empty() {
            url.push('&');
            url.push_str(command);
        }
        let resp = self
            .http
            .get(&url)
            .header("Connection", "close")
            .send()
            .await
            .map_err(|e| format!("Anlage nicht erreichbar: {}", describe(&e)))?;
        let status = resp.status().as_u16();
        let bytes = resp.bytes().await.map_err(|e| format!("Lesefehler: {}", describe(&e)))?;
        // The panel serves ISO-8859-1. Valid UTF-8 with multi-byte sequences can't be
        // Latin-1 text in practice, so accept it as UTF-8 and fall back to Latin-1.
        let body: String = match std::str::from_utf8(&bytes) {
            Ok(s) => s.to_string(),
            Err(_) => bytes.iter().map(|&b| b as char).collect(),
        };
        if !(200..300).contains(&status) {
            return Err(format!("Anlage antwortet mit HTTP {status}"));
        }
        Ok((status, body))
    }

    /// Start a new session. Returns (parsed info, raw xml).
    pub async fn init(&mut self) -> Result<(InitInfo, String), String> {
        self.sid = "00".into();
        self.cnt = "FFFF".into();
        let (_, xml) = self.get("ui_init.xml", "").await?;
        let info = parse_init(&xml)?;
        self.sid = info.session_id.clone();
        self.cnt = info.counter.clone();
        Ok((info, xml))
    }

    /// Read ui_io (LED/button colours). `taster_cmd` is e.g. "T01" or "".
    pub async fn read_io(&mut self, taster_cmd: &str) -> Result<(IoInfo, String), String> {
        let (_, xml) = self.get("ui_io.xml", taster_cmd).await?;
        let info = parse_io(&xml)?;
        self.cnt = info.counter.clone();
        Ok((info, xml))
    }

    /// Read ui_12540 (LCD + status LEDs). `key_cmd` is e.g. "T37T38" or "".
    pub async fn read_control(&mut self, key_cmd: &str) -> Result<(ControlInfo, String), String> {
        let (_, xml) = self.get("ui_12540.xml", key_cmd).await?;
        let info = parse_control(&xml)?;
        self.cnt = info.counter.clone();
        Ok((info, xml))
    }
}

fn describe(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "Zeitüberschreitung".into()
    } else if e.is_connect() {
        "Verbindung abgelehnt/keine Route".into()
    } else {
        e.to_string()
    }
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    const INIT: &str = "<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?>\n<ui_init>\n <Init>\n  <Counter>0001</Counter>\n  <Session_ID>AB12</Session_ID>\n </Init>\n <LED_Page>\n  <L1>Alarm</L1>\n  <L2>Strom</L2>\n </LED_Page>\n <Button_Page>\n  <Page_12540>On</Page_12540>\n  <Page_12540_Revision>On</Page_12540_Revision>\n  <Page_Button>On</Page_Button>\n  <Page_LED>Off</Page_LED>\n  <B1>Garage</B1>\n  <B2>Licht</B2>\n </Button_Page>\n</ui_init>";

    #[test]
    fn init_parses() {
        let i = parse_init(INIT).unwrap();
        assert_eq!(i.session_id, "AB12");
        assert_eq!(i.counter, "0001");
        assert_eq!(i.led_names, vec!["Alarm", "Strom"]);
        assert_eq!(i.button_names, vec![("B1".into(), "Garage".into()), ("B2".into(), "Licht".into())]);
        assert!(i.page_control && i.page_buttons && !i.page_leds);
    }

    #[test]
    fn io_parses() {
        let x = "<ui_io><Counter>0002</Counter><Button_Page><A>GREEN_ON</A><B>RED_BLINK_0</B><C>GREY</C></Button_Page><LED_Page><L>YELLOW_OFF</L></LED_Page></ui_io>";
        let i = parse_io(x).unwrap();
        assert_eq!(i.counter, "0002");
        assert_eq!(i.buttons[0], LedVal { color: "green".into(), state: "on".into() });
        assert_eq!(i.buttons[1], LedVal { color: "red".into(), state: "blink".into() });
        assert_eq!(i.buttons[2].state, "off");
        assert_eq!(i.leds[0], LedVal { color: "yellow".into(), state: "off".into() });
    }

    #[test]
    fn control_parses_and_decodes_lcd() {
        let x = "<ui_12540><Locked>NO</Locked><Counter>0003</Counter><LED><Betrieb><LED>GREEN_ON</LED></Betrieb><Einbruch><LED>RED_OFF</LED></Einbruch></LED><Display><Text_80_01>41,6E,6C,61,67,65,20,E1</Text_80_01><Text_80_02>4F,4B</Text_80_02><Cursor_Pos>3</Cursor_Pos><Cursor_Blink>0</Cursor_Blink></Display><Blink>00,00</Blink></ui_12540>";
        let c = parse_control(x).unwrap();
        assert_eq!(c.counter, "0003");
        assert!(!c.locked);
        assert_eq!(c.leds.len(), 2);
        assert_eq!(c.leds[0].0, "Betrieb");
        assert_eq!(c.leds[0].1.state, "on");
        assert_eq!(c.line1, "Anlage ä");
        assert_eq!(c.line2, "OK");
    }

    #[test]
    fn plain_text_passes_through() {
        assert_eq!(decode_lcd("Hallo Welt"), "Hallo Welt");
    }

    #[test]
    fn missing_session_is_error() {
        assert!(parse_init("<ui_init><Init><Counter>1</Counter></Init></ui_init>").is_err());
        assert!(parse_io("<nope/>").is_err());
    }

    #[test]
    fn key_whitelist() {
        assert_eq!(keypad_code("EIN"), Some("42"));
        assert_eq!(keypad_code("AUS"), Some("41"));
        assert_eq!(keypad_code("7"), Some("37"));
        assert_eq!(keypad_code("Revision"), Some("47"));
        assert_eq!(keypad_code("PANIC"), None);
        assert_eq!(keypad_code("42"), None);
        assert_eq!(keypad_code(""), None);
        assert_eq!(taster_code(1).as_deref(), Some("01"));
        assert_eq!(taster_code(8).as_deref(), Some("08"));
        assert_eq!(taster_code(0), None);
        assert_eq!(taster_code(9), None);
    }
}
