//! Start-up options. In Home Assistant they live in `/data/options.json`; outside HA they can
//! come from (optional) environment variables. In standalone Docker none are needed: everything
//! is set up in the web UI and stored in the data folder.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Options {
    #[serde(default)]
    pub panel_host: String,
    #[serde(default = "default_port")]
    pub panel_port: u16,
    #[serde(default = "default_group")]
    pub user_group: String,
    /// Master switch for sending commands. Default OFF: view-only.
    #[serde(default)]
    pub control_enabled: bool,
    #[serde(default = "default_poll")]
    pub poll_interval_ms: u64,
    /// Publish the panel's buttons and LEDs as Home Assistant entities (MQTT discovery),
    /// fed by a permanent background session that never touches the keypad page.
    #[serde(default)]
    pub mqtt_entities: bool,
    /// Offer the Taster as HA buttons (only while control is on as well). Default off.
    #[serde(default)]
    pub mqtt_buttons: bool,
}

fn default_port() -> u16 {
    8016
}
fn default_group() -> String {
    "1".into()
}
fn default_poll() -> u64 {
    1000
}

/// Folder for persistent files (`systems.json`, `settings.json`). `DATA_DIR`, default `/data`.
pub fn data_dir() -> String {
    std::env::var("DATA_DIR")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/data".into())
}

pub fn options_path() -> String {
    std::env::var("OPTIONS_PATH").unwrap_or_else(|_| format!("{}/options.json", data_dir()))
}

impl Options {
    pub fn load() -> Options {
        let path = options_path();
        let mut o = match std::fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str::<Options>(&s).unwrap_or_else(|e| {
                eprintln!("options.json unreadable ({e}); falling back to defaults");
                Options::from_env()
            }),
            Err(_) => Options::from_env(),
        };
        let (host, port) = split_host_port(&o.panel_host, o.panel_port);
        o.panel_host = host;
        o.panel_port = port;
        o.poll_interval_ms = o.poll_interval_ms.clamp(250, 10_000);
        o
    }

    fn from_env() -> Options {
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        Options {
            panel_host: env("PANEL_HOST").unwrap_or_default(),
            panel_port: env("PANEL_PORT")
                .and_then(|v| v.parse().ok())
                .unwrap_or(8016),
            user_group: env("USER_GROUP").unwrap_or_else(default_group),
            control_enabled: env("CONTROL_ENABLED")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(false),
            poll_interval_ms: env("POLL_INTERVAL_MS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1000),
            mqtt_entities: env("MQTT_ENTITIES")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(false),
            mqtt_buttons: env("MQTT_BUTTONS")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(false),
        }
    }
}

/// Accept `host`, `host:port`, `http://host:port/` – a port inside the host wins.
pub fn split_host_port(raw: &str, fallback: u16) -> (String, u16) {
    let mut h = raw
        .trim()
        .trim_start_matches("http://")
        .trim_start_matches("https://");
    h = h.trim_end_matches('/');
    if let Some((host, port)) = h.rsplit_once(':') {
        if let Ok(p) = port.parse::<u16>() {
            return (host.to_string(), p);
        }
    }
    (h.to_string(), fallback)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_port_forms() {
        assert_eq!(split_host_port("a.net", 8016), ("a.net".into(), 8016));
        assert_eq!(split_host_port("a.net:9000", 8016), ("a.net".into(), 9000));
        assert_eq!(
            split_host_port("http://a.net:9000/", 8016),
            ("a.net".into(), 9000)
        );
        assert_eq!(split_host_port("", 8016), ("".into(), 8016));
    }
}
