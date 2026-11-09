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
    (1..=8)
        .contains(&index_1_based)
        .then(|| format!("{index_1_based:02}"))
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

/// One LCD cell: a printable character, or the index (0..15) of a custom glyph whose
/// bitmap comes from `ui_12540_sz.xml`.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(untagged)]
pub enum Cell {
    Glyph(u8),
    Ch(String),
}

#[derive(Debug, Clone, Default)]
pub struct ControlInfo {
    pub cells1: Vec<Cell>,
    pub cells2: Vec<Cell>,
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
    node.descendants()
        .find(|n| n.is_element() && n.tag_name().name() == name)
}

fn text_of(node: Node) -> String {
    node.text().unwrap_or("").trim().to_string()
}

fn child_text(node: Node, name: &str) -> Option<String> {
    find(node, name).map(text_of)
}

/// Display name of an init entry: `<Button nr="1"><Text>Name</Text></Button>` (real panel)
/// or plain `<B1>Name</B1>`.
fn label_of(node: Node) -> String {
    node.children()
        .find(|n| n.is_element() && n.tag_name().name() == "Text")
        .map(text_of)
        .unwrap_or_else(|| text_of(node))
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
    LedVal {
        color: color.into(),
        state: state.into(),
    }
}

/// Map a character code of the panel's LCD (HD44780 "A00" ROM) to a Unicode char.
/// Printable ASCII is identical; 0x7E/0x7F are arrows, 0xA1..0xDF are half-width katakana
/// (U+FF61..), and the German umlauts sit at 0xE1/0xEF/0xF5 with ß at 0xE2.
fn lcd_char(b: u8) -> char {
    match b {
        0x7E => '→',
        0x7F => '←',
        0x20..=0x7D => b as char,
        0xE1 => 'ä',
        0xEF => 'ö',
        0xF5 => 'ü',
        0xE2 => 'ß',
        0xA1..=0xDF => char::from_u32(0xFF61 + (b as u32 - 0xA1)).unwrap_or(' '),
        0xA0 | 0xE0..=0xFF => b as char, // Latin-1 for the rest
        _ => ' ',
    }
}

fn hex_codes(raw: &str) -> Option<Vec<u8>> {
    let t = raw.trim();
    if t.is_empty() {
        return None;
    }
    let parts: Vec<&str> = t.split(',').map(str::trim).collect();
    if !parts
        .iter()
        .all(|p| (1..=2).contains(&p.len()) && p.bytes().all(|c| c.is_ascii_hexdigit()))
    {
        return None;
    }
    Some(
        parts
            .iter()
            .map(|p| u8::from_str_radix(p, 16).unwrap_or(0x20))
            .collect(),
    )
}

/// Per-cell version of the display text; codes below 0x10 become glyph indices.
pub fn decode_cells(raw: &str) -> Vec<Cell> {
    match hex_codes(raw) {
        Some(codes) => codes
            .into_iter()
            .map(|b| {
                if b < 0x10 {
                    Cell::Glyph(b)
                } else {
                    Cell::Ch(lcd_char(b).to_string())
                }
            })
            .collect(),
        None => raw
            .trim()
            .chars()
            .map(|c| Cell::Ch(c.to_string()))
            .collect(),
    }
}

/// The 5x8 bitmaps of the custom glyphs from `ui_12540_sz.xml`: up to 16 entries of 8 row bytes.
pub fn parse_signs(xml: &str) -> Result<(String, Vec<Vec<u8>>), String> {
    let doc = parse_doc(xml)?;
    let root =
        find(doc.root_element(), "ui_12540_sz").ok_or("Antwort enthält kein <ui_12540_sz>")?;
    let counter = child_text(root, "Counter").ok_or("kein Counter in der Antwort")?;
    let glyphs = root
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "Code")
        .take(16)
        .map(|n| {
            let mut rows = hex_codes(&text_of(n)).unwrap_or_default();
            rows.resize(8, 0);
            rows
        })
        .collect();
    Ok((counter, glyphs))
}

/// Display text arrives as comma-separated hex codes ("48,61,6C,..."). Anything
/// that doesn't look like that is returned unchanged. Custom glyphs become blanks here;
/// use `decode_cells` for the faithful version.
pub fn decode_lcd(raw: &str) -> String {
    match hex_codes(raw) {
        Some(codes) => codes
            .into_iter()
            .map(|b| if b < 0x10 { ' ' } else { lcd_char(b) })
            .collect::<String>()
            .trim_end()
            .to_string(),
        None => raw.trim().to_string(),
    }
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
    let mut info = InitInfo {
        session_id,
        counter,
        page_control: true,
        page_buttons: true,
        page_leds: true,
        ..Default::default()
    };

    let decode_name = |s: String| if s.contains(',') { decode_lcd(&s) } else { s };

    if let Some(lp) = find(root, "LED_Page") {
        info.led_names = element_children(lp)
            .into_iter()
            .map(|n| decode_name(label_of(n)))
            .collect();
    }
    if let Some(bp) = find(root, "Button_Page") {
        for n in element_children(bp) {
            let tag = n.tag_name().name().to_string();
            let val = label_of(n);
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
            .map(|n| {
                element_children(n)
                    .into_iter()
                    .map(|c| parse_led(&text_of(c)))
                    .collect()
            })
            .unwrap_or_default()
    };
    Ok(IoInfo {
        counter,
        buttons: vals("Button_Page"),
        leds: vals("LED_Page"),
    })
}

pub fn parse_control(xml: &str) -> Result<ControlInfo, String> {
    let doc = parse_doc(xml)?;
    let root = find(doc.root_element(), "ui_12540").ok_or("Antwort enthält kein <ui_12540>")?;
    let counter = child_text(root, "Counter").ok_or("kein Counter in der Antwort")?;
    let mut info = ControlInfo {
        counter,
        ..Default::default()
    };
    info.locked = child_text(root, "Locked")
        .map(|v| v.eq_ignore_ascii_case("yes"))
        .unwrap_or(false);
    if let Some(led) = root
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "LED")
    {
        for c in element_children(led) {
            let tag = c.tag_name().name().to_string();
            let v = find(c, "LED").map(text_of).unwrap_or_else(|| text_of(c));
            info.leds.push((tag, parse_led(&v)));
        }
    }
    if let Some(d) = find(root, "Display") {
        let (r1, r2) = (
            child_text(d, "Text_80_01").unwrap_or_default(),
            child_text(d, "Text_80_02").unwrap_or_default(),
        );
        info.line1 = decode_lcd(&r1);
        info.line2 = decode_lcd(&r2);
        info.cells1 = decode_cells(&r1);
        info.cells2 = decode_cells(&r2);
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

/// English label for the same LEDs (Home Assistant entity names when the language is English).
pub fn led_label_en(tag: &str) -> String {
    match tag {
        "Betrieb" => "Operation",
        "S_Alarm" => "Common alarm",
        "S_Stoerung" => "Common fault",
        "Sys_Stoerung" => "System fault",
        "Einbruch" => "Intrusion",
        "Sabotage" => "Tamper",
        "Ueberfall" => "Hold-up",
        "Pol_Not" => "Police call",
        other => other,
    }
    .to_string()
}

// ---------------------------------------------------------------- transport

/// The panel numbers its answers; the next request must carry the answered counter plus one
/// (4 hex digits, upper case). `FFFF` is kept as is. Sending the answered counter unchanged
/// makes the panel close the connection without any response.
pub fn next_counter(answered: &str) -> String {
    match u32::from_str_radix(answered.trim(), 16) {
        Ok(v) if v != 0xFFFF => format!("{:04X}", v + 1),
        _ => answered.trim().to_uppercase(),
    }
}

pub struct Panel {
    http: reqwest::Client,
    base: String,
    group: String,
    sid: String,
    cnt: String,
    /// AES key (16/24/32 bytes) when the panel is set up with one; None = plain text.
    key: Option<Vec<u8>>,
}

// ---------------------------------------------------------------- AES (like the original app)
//
// With a key the app sends `<page>?B-GR=<group>&` followed by the upper-case hex of
// AES-ECB-PKCS7("S-ID=..&CNT=..[&T..]"), and the panel answers `<crypt>HEX</crypt>`,
// whose decrypted content (Cp1250) is the usual XML.

pub fn key_bytes(hex: &str) -> Option<Vec<u8>> {
    let h = hex.trim();
    if h.is_empty() {
        return None;
    }
    let b = hex_decode(h)?;
    matches!(b.len(), 16 | 24 | 32).then_some(b)
}

fn hex_decode(h: &str) -> Option<Vec<u8>> {
    if h.len() % 2 != 0 {
        return None;
    }
    (0..h.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(h.get(i..i + 2)?, 16).ok())
        .collect()
}

fn hex_upper(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect()
}

fn ecb<F: FnMut(&mut [u8; 16])>(data: &mut [u8], mut f: F) {
    for chunk in data.chunks_exact_mut(16) {
        let mut block = [0u8; 16];
        block.copy_from_slice(chunk);
        f(&mut block);
        chunk.copy_from_slice(&block);
    }
}

pub fn encrypt_hex(key: &[u8], plain: &str) -> Result<String, String> {
    use aes::cipher::{generic_array::GenericArray, BlockEncrypt, KeyInit};
    let mut data = plain.as_bytes().to_vec();
    let pad = 16 - data.len() % 16;
    data.extend(std::iter::repeat(pad as u8).take(pad));
    macro_rules! run {
        ($c:ty) => {{
            let c = <$c>::new_from_slice(key).map_err(|_| "AES-Schlüssel hat die falsche Länge")?;
            ecb(&mut data, |b| {
                c.encrypt_block(GenericArray::from_mut_slice(b))
            });
        }};
    }
    match key.len() {
        16 => run!(aes::Aes128),
        24 => run!(aes::Aes192),
        32 => run!(aes::Aes256),
        _ => return Err("AES-Schlüssel hat die falsche Länge".into()),
    }
    Ok(hex_upper(&data))
}

pub fn decrypt_hex(key: &[u8], hex: &str) -> Result<String, String> {
    use aes::cipher::{generic_array::GenericArray, BlockDecrypt, KeyInit};
    let mut data = hex_decode(hex.trim()).ok_or("verschlüsselte Antwort ist kein Hex")?;
    if data.is_empty() || data.len() % 16 != 0 {
        return Err("verschlüsselte Antwort hat eine falsche Länge".into());
    }
    macro_rules! run {
        ($c:ty) => {{
            let c = <$c>::new_from_slice(key).map_err(|_| "AES-Schlüssel hat die falsche Länge")?;
            ecb(&mut data, |b| {
                c.decrypt_block(GenericArray::from_mut_slice(b))
            });
        }};
    }
    match key.len() {
        16 => run!(aes::Aes128),
        24 => run!(aes::Aes192),
        32 => run!(aes::Aes256),
        _ => return Err("AES-Schlüssel hat die falsche Länge".into()),
    }
    let pad = *data.last().unwrap() as usize;
    if pad == 0
        || pad > 16
        || data.len() < pad
        || !data[data.len() - pad..].iter().all(|&b| b as usize == pad)
    {
        return Err("Entschlüsseln fehlgeschlagen: AES-Schlüssel falsch?".into());
    }
    data.truncate(data.len() - pad);
    // Cp1250: the letters the panel uses (äöüß ÄÖÜ) sit where Latin-1 has them
    Ok(data.iter().map(|&b| b as char).collect())
}

/// `<crypt>HEX</crypt>` -> the inner hex, if the answer is encrypted.
fn crypt_payload(body: &str) -> Option<&str> {
    let a = body.find("<crypt>")? + "<crypt>".len();
    let b = body[a..].find("</crypt>")? + a;
    Some(body[a..b].trim())
}

impl Panel {
    pub fn new(host: &str, port: u16, group: &str) -> Panel {
        Panel::with_key(host, port, group, "")
    }

    /// `key_hex` empty = plain text, otherwise the panel's AES key as hex.
    pub fn with_key(host: &str, port: u16, group: &str, key_hex: &str) -> Panel {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(4))
            .timeout(Duration::from_secs(8))
            .pool_max_idle_per_host(0)
            .build()
            .expect("http client");
        Panel {
            http,
            base: format!("http://{host}:{port}"),
            group: group.to_string(),
            sid: "00".into(),
            cnt: "FFFF".into(),
            key: key_bytes(key_hex),
        }
    }

    pub fn session_id(&self) -> &str {
        &self.sid
    }
    pub fn counter(&self) -> &str {
        &self.cnt
    }

    /// One GET. `command` is an already-validated `T..` suffix or empty.
    async fn get(&self, page: &str, command: &str) -> Result<(u16, String), String> {
        let mut rest = format!("S-ID={}&CNT={}", self.sid, self.cnt);
        if !command.is_empty() {
            rest.push('&');
            rest.push_str(command);
        }
        let url = match &self.key {
            Some(k) => format!(
                "{}/{page}?B-GR={}&{}",
                self.base,
                self.group,
                encrypt_hex(k, &rest)?
            ),
            None => format!("{}/{page}?B-GR={}&{rest}", self.base, self.group),
        };
        let resp = self
            .http
            .get(&url)
            .header("Connection", "close")
            .send()
            .await
            .map_err(|e| format!("Anlage nicht erreichbar: {}", describe(&e)))?;
        let status = resp.status().as_u16();
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("Lesefehler: {}", describe(&e)))?;
        // The panel serves ISO-8859-1. Valid UTF-8 with multi-byte sequences can't be
        // Latin-1 text in practice, so accept it as UTF-8 and fall back to Latin-1.
        let body: String = match std::str::from_utf8(&bytes) {
            Ok(s) => s.to_string(),
            Err(_) => bytes.iter().map(|&b| b as char).collect(),
        };
        if !(200..300).contains(&status) {
            return Err(format!("Anlage antwortet mit HTTP {status}"));
        }
        let body = match (crypt_payload(&body), &self.key) {
            (Some(hex), Some(k)) => decrypt_hex(k, hex)?,
            (Some(_), None) => return Err("Die Anlage antwortet verschlüsselt: bitte den AES-Schlüssel bei der Anlage eintragen.".into()),
            (None, _) => body,
        };
        Ok((status, body))
    }

    /// Start a new session. Returns (parsed info, raw xml).
    pub async fn init(&mut self) -> Result<(InitInfo, String), String> {
        self.sid = "00".into();
        self.cnt = "FFFF".into();
        let (_, xml) = self.get("ui_init.xml", "").await?;
        let info = parse_init(&xml)?;
        self.sid = info.session_id.clone();
        self.cnt = next_counter(&info.counter);
        Ok((info, xml))
    }

    /// Read ui_io (LED/button colours). `taster_cmd` is e.g. "T01" or "".
    pub async fn read_io(&mut self, taster_cmd: &str) -> Result<(IoInfo, String), String> {
        let (_, xml) = self.get("ui_io.xml", taster_cmd).await?;
        let info = parse_io(&xml)?;
        self.cnt = next_counter(&info.counter);
        Ok((info, xml))
    }

    /// Read the custom glyph bitmaps (ui_12540_sz). The original app does this once after init.
    pub async fn read_signs(&mut self) -> Result<Vec<Vec<u8>>, String> {
        let (_, xml) = self.get("ui_12540_sz.xml", "").await?;
        let (counter, glyphs) = parse_signs(&xml)?;
        self.cnt = next_counter(&counter);
        Ok(glyphs)
    }

    /// Read ui_12540 (LCD + status LEDs). `key_cmd` is e.g. "T37T38" or "".
    pub async fn read_control(&mut self, key_cmd: &str) -> Result<(ControlInfo, String), String> {
        let (_, xml) = self.get("ui_12540.xml", key_cmd).await?;
        let info = parse_control(&xml)?;
        self.cnt = next_counter(&info.counter);
        Ok((info, xml))
    }
}

fn describe(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "Zeitüberschreitung".into()
    } else if e.is_connect() {
        "Verbindung abgelehnt/keine Route".into()
    } else {
        // include the whole cause chain, "error sending request" alone says nothing
        let mut out = e.to_string();
        let mut src = std::error::Error::source(e);
        while let Some(c) = src {
            out.push_str(" <- ");
            out.push_str(&c.to_string());
            src = c.source();
        }
        out
    }
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aes_matches_openssl_and_roundtrips() {
        let k = key_bytes("000102030405060708090a0b0c0d0e0f").unwrap();
        let enc = encrypt_hex(&k, "S-ID=01&CNT=40E5&T06").unwrap();
        assert_eq!(
            enc,
            "9306CB0EF8B97488B6A1360ED0B91D1D23BE75F9F4187D2B3A2C27F7EAB1012E"
        );
        assert_eq!(decrypt_hex(&k, &enc).unwrap(), "S-ID=01&CNT=40E5&T06");
        let k256 = key_bytes(&"1F".repeat(32)).unwrap();
        let xml = "<ui_io><Counter>0001</Counter><Text>Haustür</Text></ui_io>";
        let latin1: String = xml.chars().collect();
        let bytes: Vec<u8> = latin1.chars().map(|c| c as u32 as u8).collect();
        // encrypt the Latin-1 bytes like the panel would and read them back
        let e = encrypt_hex(&k256, &String::from_utf8_lossy(&bytes)).unwrap();
        assert!(decrypt_hex(&k256, &e).is_ok());
        assert!(
            decrypt_hex(&key_bytes(&"20".repeat(32)).unwrap(), &e).is_err()
                || decrypt_hex(&key_bytes(&"20".repeat(32)).unwrap(), &e).unwrap() != xml
        );
        assert!(key_bytes("0011").is_none());
        assert!(key_bytes("").is_none());
        assert_eq!(crypt_payload("<?xml?><crypt> AB12 </crypt>"), Some("AB12"));
    }

    const INIT: &str = "<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?>\n<ui_init>\n <Init>\n  <Counter>0001</Counter>\n  <Session_ID>AB12</Session_ID>\n </Init>\n <LED_Page>\n  <L1>Alarm</L1>\n  <L2>Strom</L2>\n </LED_Page>\n <Button_Page>\n  <Page_12540>On</Page_12540>\n  <Page_12540_Revision>On</Page_12540_Revision>\n  <Page_Button>On</Page_Button>\n  <Page_LED>Off</Page_LED>\n  <B1>Garage</B1>\n  <B2>Licht</B2>\n </Button_Page>\n</ui_init>";

    #[test]
    fn counter_is_answered_plus_one() {
        assert_eq!(next_counter("2410"), "2411");
        assert_eq!(next_counter("00FF"), "0100");
        assert_eq!(next_counter("0009"), "000A");
        assert_eq!(next_counter("FFFF"), "FFFF");
        assert_eq!(next_counter("9abc"), "9ABD");
    }

    const REAL_INIT: &str = "<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?>\n<ui_init>\n\t<Init>\n\t\t<Counter>40E5</Counter>\n\t\t<Session_ID>01</Session_ID>\n\t</Init>\n\t<Button_Page>\n\t\t<Page_12540>On</Page_12540>\n\t\t<Page_Button>On</Page_Button>\n\t\t<Page_LED>On</Page_LED>\n\t\t<Button nr=\"1\"><Text>Garage</Text></Button>\n\t\t<Button nr=\"2\"><Text></Text></Button>\n\t\t<LED nr=\"1\"><Text>Feuer</Text></LED>\n\t</Button_Page>\n\t<LED_Page>\n\t\t<LED nr=\"1\"><Text>Feueralarm</Text></LED>\n\t</LED_Page>\n</ui_init>";

    #[test]
    fn real_init_names_come_from_text_children() {
        let i = parse_init(REAL_INIT).unwrap();
        assert_eq!(i.session_id, "01");
        assert_eq!(i.led_names, vec!["Feueralarm"]);
        assert_eq!(
            i.button_names,
            vec![
                ("Button".into(), "Garage".into()),
                ("Button".into(), "".into()),
                ("LED".into(), "Feuer".into())
            ]
        );
        assert!(i.page_leds);
    }

    #[test]
    fn lcd_cells_keep_glyphs_and_map_a00() {
        let c = decode_cells("42,C6,0A,7E,E1,20");
        assert_eq!(
            c,
            vec![
                Cell::Ch("B".into()),
                Cell::Ch("ﾆ".into()),
                Cell::Glyph(10),
                Cell::Ch("→".into()),
                Cell::Ch("ä".into()),
                Cell::Ch(" ".into())
            ]
        );
        assert_eq!(decode_lcd("42,0A,E1"), "B ä");
    }

    #[test]
    fn signs_parse() {
        let x = "<ui_12540_sz><Counter>0005</Counter><SZeichen><Code>1F,11,11,11,11,11,1F,00</Code></SZeichen><SZeichen><Code>04</Code></SZeichen></ui_12540_sz>";
        let (c, g) = parse_signs(x).unwrap();
        assert_eq!(c, "0005");
        assert_eq!(g[0], vec![0x1F, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1F, 0]);
        assert_eq!(g[1], vec![4, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn init_parses() {
        let i = parse_init(INIT).unwrap();
        assert_eq!(i.session_id, "AB12");
        assert_eq!(i.counter, "0001");
        assert_eq!(i.led_names, vec!["Alarm", "Strom"]);
        assert_eq!(
            i.button_names,
            vec![
                ("B1".into(), "Garage".into()),
                ("B2".into(), "Licht".into())
            ]
        );
        assert!(i.page_control && i.page_buttons && !i.page_leds);
    }

    #[test]
    fn io_parses() {
        let x = "<ui_io><Counter>0002</Counter><Button_Page><A>GREEN_ON</A><B>RED_BLINK_0</B><C>GREY</C></Button_Page><LED_Page><L>YELLOW_OFF</L></LED_Page></ui_io>";
        let i = parse_io(x).unwrap();
        assert_eq!(i.counter, "0002");
        assert_eq!(
            i.buttons[0],
            LedVal {
                color: "green".into(),
                state: "on".into()
            }
        );
        assert_eq!(
            i.buttons[1],
            LedVal {
                color: "red".into(),
                state: "blink".into()
            }
        );
        assert_eq!(i.buttons[2].state, "off");
        assert_eq!(
            i.leds[0],
            LedVal {
                color: "yellow".into(),
                state: "off".into()
            }
        );
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
