//! App settings that the user changes in the web UI (persisted in `<data>/settings.json`):
//! "allow control", poll interval and the optional login password.
//!
//! Two run modes share this code:
//! * **Home Assistant add-on**: the add-on options seed these settings. When an option is
//!   changed in Home Assistant it wins on the next start; otherwise the UI value stays.
//!   HA authenticates every request (Ingress), so the password is not used there.
//! * **Standalone** (plain Docker): no config files needed, everything is set in the UI.

use crate::config::Options;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::sync::RwLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    HomeAssistant,
    Standalone,
}

impl Mode {
    /// `EMA_MODE=standalone|homeassistant` wins; otherwise HA is detected by the token the
    /// Supervisor gives every add-on, or by its options file.
    pub fn detect(options_path: &str) -> Mode {
        match std::env::var("EMA_MODE")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "standalone" | "docker" => return Mode::Standalone,
            "homeassistant" | "ha" => return Mode::HomeAssistant,
            _ => {}
        }
        if std::env::var("SUPERVISOR_TOKEN")
            .map(|t| !t.is_empty())
            .unwrap_or(false)
            || std::path::Path::new(options_path).exists()
        {
            Mode::HomeAssistant
        } else {
            Mode::Standalone
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct HaSeen {
    control_enabled: bool,
    poll_interval_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Data {
    #[serde(default)]
    control_enabled: bool,
    #[serde(default = "default_poll")]
    poll_interval_ms: u64,
    /// Argon2 PHC string; `None` = no login required.
    #[serde(default)]
    password_hash: Option<String>,
    /// Key that signs login cookies; replaced when the password changes (logs everyone out).
    #[serde(default)]
    secret: String,
    /// Add-on option values applied last time (HA mode only).
    #[serde(default)]
    ha_seen: Option<HaSeen>,
}

fn default_poll() -> u64 {
    1000
}

impl Default for Data {
    fn default() -> Self {
        Data {
            control_enabled: false,
            poll_interval_ms: 1000,
            password_hash: None,
            secret: String::new(),
            ha_seen: None,
        }
    }
}

pub struct Settings {
    path: String,
    pub mode: Mode,
    data: RwLock<Data>,
}

pub const SESSION_DAYS: u64 = 30;

fn random_hex(n: usize) -> String {
    let mut b = vec![0u8; n];
    getrandom::getrandom(&mut b).expect("random");
    hex(&b)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn clamp_poll(ms: u64) -> u64 {
    ms.clamp(250, 10_000)
}

impl Settings {
    pub fn load(path: &str, mode: Mode, opts: &Options, options_file_present: bool) -> Settings {
        let existing = std::fs::read_to_string(path).ok().and_then(
            |s| match serde_json::from_str::<Data>(&s) {
                Ok(d) => Some(d),
                Err(e) => {
                    eprintln!(
                        "{path} unreadable ({e}); starting with defaults (view only, no password)"
                    );
                    None
                }
            },
        );
        let first = existing.is_none();
        let mut d = existing.unwrap_or_default();
        if first {
            // first start: env vars / add-on options are the starting values
            d.control_enabled = opts.control_enabled;
            d.poll_interval_ms = opts.poll_interval_ms;
        }
        if mode == Mode::HomeAssistant && options_file_present {
            let now = HaSeen {
                control_enabled: opts.control_enabled,
                poll_interval_ms: opts.poll_interval_ms,
            };
            if d.ha_seen.as_ref() != Some(&now) {
                if !first {
                    println!("add-on options changed in Home Assistant: applying them");
                }
                d.control_enabled = now.control_enabled;
                d.poll_interval_ms = now.poll_interval_ms;
                d.ha_seen = Some(now);
            }
        }
        d.poll_interval_ms = clamp_poll(d.poll_interval_ms);
        if d.secret.len() < 32 {
            d.secret = random_hex(32);
        }
        let s = Settings {
            path: path.to_string(),
            mode,
            data: RwLock::new(d),
        };
        if let Err(e) = s.save() {
            eprintln!("{e}");
        }
        s
    }

    fn save(&self) -> Result<(), String> {
        let json =
            serde_json::to_string_pretty(&*self.data.read().unwrap()).map_err(|e| e.to_string())?;
        let tmp = format!("{}.tmp", self.path);
        std::fs::write(&tmp, json).map_err(|e| format!("Speichern fehlgeschlagen: {e}"))?;
        std::fs::rename(&tmp, &self.path).map_err(|e| format!("Speichern fehlgeschlagen: {e}"))
    }

    pub fn control(&self) -> bool {
        self.data.read().unwrap().control_enabled
    }

    pub fn poll_ms(&self) -> u64 {
        self.data.read().unwrap().poll_interval_ms
    }

    pub fn set_control(&self, on: bool) -> Result<(), String> {
        self.data.write().unwrap().control_enabled = on;
        self.save()
    }

    pub fn set_poll(&self, ms: u64) -> Result<(), String> {
        self.data.write().unwrap().poll_interval_ms = clamp_poll(ms);
        self.save()
    }

    // ------------------------------------------------------------ login (standalone only)

    /// Login is only ever enforced in standalone mode and only when a password is set.
    pub fn auth_required(&self) -> bool {
        self.mode == Mode::Standalone && self.data.read().unwrap().password_hash.is_some()
    }

    pub fn has_password(&self) -> bool {
        self.data.read().unwrap().password_hash.is_some()
    }

    pub fn check_password(&self, pw: &str) -> bool {
        let Some(h) = self.data.read().unwrap().password_hash.clone() else {
            return false;
        };
        match PasswordHash::new(&h) {
            Ok(parsed) => Argon2::default()
                .verify_password(pw.as_bytes(), &parsed)
                .is_ok(),
            Err(_) => false,
        }
    }

    /// Set, change (`Some`) or remove (`None`) the password. Always rotates the cookie key.
    pub fn set_password(&self, pw: Option<&str>) -> Result<(), String> {
        let hash = match pw {
            Some(p) => {
                if p.chars().count() < 6 || p.len() > 200 {
                    return Err("Passwort: mindestens 6 Zeichen.".into());
                }
                let mut salt = [0u8; 16];
                getrandom::getrandom(&mut salt).map_err(|e| e.to_string())?;
                let salt = SaltString::encode_b64(&salt).map_err(|e| e.to_string())?;
                Some(
                    Argon2::default()
                        .hash_password(p.as_bytes(), &salt)
                        .map_err(|e| e.to_string())?
                        .to_string(),
                )
            }
            None => None,
        };
        {
            let mut d = self.data.write().unwrap();
            d.password_hash = hash;
            d.secret = random_hex(32);
        }
        self.save()
    }

    fn sign(&self, msg: &str) -> String {
        let key = self.data.read().unwrap().secret.clone();
        let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes()).expect("hmac key");
        mac.update(msg.as_bytes());
        hex(&mac.finalize().into_bytes())
    }

    /// Cookie value `<expiry secs>.<nonce>.<hmac>`.
    pub fn issue_token(&self, now_s: u64) -> String {
        let msg = format!("{}.{}", now_s + SESSION_DAYS * 86_400, random_hex(8));
        format!("{msg}.{}", self.sign(&msg))
    }

    pub fn verify_token(&self, token: &str, now_s: u64) -> bool {
        let Some((msg, sig)) = token.rsplit_once('.') else {
            return false;
        };
        let Some(exp) = msg.split('.').next().and_then(|e| e.parse::<u64>().ok()) else {
            return false;
        };
        if exp <= now_s {
            return false;
        }
        let want = self.sign(msg);
        // constant-time compare
        want.len() == sig.len()
            && want
                .bytes()
                .zip(sig.bytes())
                .fold(0u8, |a, (x, y)| a | (x ^ y))
                == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(control: bool, poll: u64) -> Options {
        Options {
            panel_host: String::new(),
            panel_port: 8016,
            user_group: "1".into(),
            control_enabled: control,
            poll_interval_ms: poll,
        }
    }

    fn tmp(name: &str) -> String {
        let dir = std::env::temp_dir().join(format!("ema-set-{name}-{}", random_hex(4)));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("settings.json").to_string_lossy().to_string()
    }

    #[test]
    fn standalone_ui_values_persist() {
        let p = tmp("sa");
        let s = Settings::load(&p, Mode::Standalone, &opts(false, 1000), false);
        assert!(!s.control());
        s.set_control(true).unwrap();
        s.set_poll(50).unwrap();
        let s2 = Settings::load(&p, Mode::Standalone, &opts(false, 1000), false);
        assert!(s2.control());
        assert_eq!(s2.poll_ms(), 250);
    }

    #[test]
    fn ha_option_change_wins_once() {
        let p = tmp("ha");
        let s = Settings::load(&p, Mode::HomeAssistant, &opts(false, 1000), true);
        s.set_control(true).unwrap(); // changed in the UI
                                      // same options on restart: UI value stays
        assert!(Settings::load(&p, Mode::HomeAssistant, &opts(false, 1000), true).control());
        // option changed in HA: it wins
        let s3 = Settings::load(&p, Mode::HomeAssistant, &opts(true, 2000), true);
        assert_eq!(s3.poll_ms(), 2000);
        s3.set_control(false).unwrap();
        assert!(!Settings::load(&p, Mode::HomeAssistant, &opts(true, 2000), true).control());
    }

    #[test]
    fn password_and_tokens() {
        let p = tmp("pw");
        let s = Settings::load(&p, Mode::Standalone, &opts(false, 1000), false);
        assert!(!s.auth_required());
        assert!(s.set_password(Some("123")).is_err());
        s.set_password(Some("geheim123")).unwrap();
        assert!(s.auth_required());
        assert!(s.check_password("geheim123"));
        assert!(!s.check_password("geheim124"));
        let t = s.issue_token(1000);
        assert!(s.verify_token(&t, 1000));
        assert!(!s.verify_token(&t, 1000 + SESSION_DAYS * 86_400));
        assert!(!s.verify_token(&format!("{t}0"), 1000));
        assert!(!s.verify_token("9999999999.aa.bb", 1000));
        // password survives a restart, changing it logs everyone out
        let s2 = Settings::load(&p, Mode::Standalone, &opts(false, 1000), false);
        assert!(s2.check_password("geheim123") && s2.verify_token(&t, 1000));
        s2.set_password(Some("anderes99")).unwrap();
        assert!(!s2.verify_token(&t, 1000));
        s2.set_password(None).unwrap();
        assert!(!s2.auth_required());
        // HA mode never asks for a login
        let s3 = Settings::load(&tmp("ha2"), Mode::HomeAssistant, &opts(false, 1000), true);
        s3.set_password(Some("geheim123")).unwrap();
        assert!(!s3.auth_required());
    }
}
