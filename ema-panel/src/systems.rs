//! The list of alarm systems the user manages in the UI (name, address, port, group).
//! Stored as JSON in the add-on's persistent `/data` folder. The add-on options only seed
//! the first entry on the very first start.

use crate::config::split_host_port;
use serde::{Deserialize, Serialize};
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct System {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub group: String,
}

/// What the UI sends when adding or editing a system.
#[derive(Debug, Deserialize)]
pub struct SystemInput {
    pub id: Option<String>,
    pub name: String,
    pub host: String,
    pub port: Option<u16>,
    pub group: Option<String>,
}

pub struct Store {
    path: String,
    list: RwLock<Vec<System>>,
}

fn now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

/// Validate and normalise user input. Only plain host names / IPv4 addresses are accepted,
/// so the add-on can never be pointed at anything but `http://host:port`.
pub fn validate(input: &SystemInput) -> Result<(String, String, u16, String), String> {
    let name: String = input.name.trim().chars().filter(|c| !c.is_control()).collect();
    if name.is_empty() || name.chars().count() > 40 {
        return Err("Name: 1 bis 40 Zeichen.".into());
    }
    let (host, port) = split_host_port(&input.host, input.port.unwrap_or(8016));
    let host = host.to_ascii_lowercase();
    let ok_chars = host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_');
    if host.is_empty() || host.len() > 253 || !ok_chars || host.starts_with('.') || host.starts_with('-') {
        return Err("Adresse: nur Hostname oder IP, z. B. 192.168.178.99 oder name.myfritz.net.".into());
    }
    if port == 0 {
        return Err("Port: 1 bis 65535.".into());
    }
    let group = input.group.clone().unwrap_or_else(|| "1".into());
    if !matches!(group.as_str(), "1" | "2" | "3" | "4") {
        return Err("Benutzergruppe: 1 bis 4.".into());
    }
    Ok((name, host, port, group))
}

impl Store {
    pub fn load(path: &str, seed: Option<System>) -> Store {
        let list = match std::fs::read_to_string(path) {
            Ok(s) => serde_json::from_str::<Vec<System>>(&s).unwrap_or_else(|e| {
                eprintln!("{path} unreadable ({e}); starting with an empty list");
                Vec::new()
            }),
            Err(_) => seed.into_iter().collect(), // first start: use the add-on options
        };
        let store = Store { path: path.to_string(), list: RwLock::new(list) };
        if !std::path::Path::new(path).exists() {
            let _ = store.save();
        }
        store
    }

    fn save(&self) -> Result<(), String> {
        let json = serde_json::to_string_pretty(&*self.list.read().unwrap()).map_err(|e| e.to_string())?;
        let tmp = format!("{}.tmp", self.path);
        std::fs::write(&tmp, json).map_err(|e| format!("Speichern fehlgeschlagen: {e}"))?;
        std::fs::rename(&tmp, &self.path).map_err(|e| format!("Speichern fehlgeschlagen: {e}"))
    }

    pub fn list(&self) -> Vec<System> {
        self.list.read().unwrap().clone()
    }

    pub fn get(&self, id: &str) -> Option<System> {
        self.list.read().unwrap().iter().find(|s| s.id == id).cloned()
    }

    pub fn first(&self) -> Option<System> {
        self.list.read().unwrap().first().cloned()
    }

    pub fn upsert(&self, input: SystemInput) -> Result<System, String> {
        let (name, host, port, group) = validate(&input)?;
        let sys = {
            let mut l = self.list.write().unwrap();
            match input.id.as_deref().filter(|i| !i.is_empty()) {
                Some(id) => {
                    let s = l.iter_mut().find(|s| s.id == id).ok_or("Anlage nicht gefunden.")?;
                    s.name = name;
                    s.host = host;
                    s.port = port;
                    s.group = group;
                    s.clone()
                }
                None => {
                    if l.len() >= 20 {
                        return Err("Maximal 20 Anlagen.".into());
                    }
                    let mut n = now_ms();
                    while l.iter().any(|s| s.id == format!("s{n:x}")) {
                        n += 1;
                    }
                    let s = System { id: format!("s{n:x}"), name, host, port, group };
                    l.push(s.clone());
                    s
                }
            }
        };
        self.save()?;
        Ok(sys)
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        {
            let mut l = self.list.write().unwrap();
            let before = l.len();
            l.retain(|s| s.id != id);
            if l.len() == before {
                return Err("Anlage nicht gefunden.".into());
            }
        }
        self.save()
    }

    /// Reorder by id list; unknown ids are ignored, missing ones keep their relative order at the end.
    pub fn reorder(&self, ids: &[String]) -> Result<(), String> {
        {
            let mut l = self.list.write().unwrap();
            let mut out: Vec<System> = Vec::new();
            for id in ids {
                if let Some(pos) = l.iter().position(|s| &s.id == id) {
                    out.push(l.remove(pos));
                }
            }
            out.append(&mut l);
            *l = out;
        }
        self.save()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inp(name: &str, host: &str, port: Option<u16>, group: Option<&str>) -> SystemInput {
        SystemInput { id: None, name: name.into(), host: host.into(), port, group: group.map(|g| g.into()) }
    }

    #[test]
    fn validates_input() {
        assert_eq!(validate(&inp(" Haus ", "192.168.1.5", None, None)).unwrap(), ("Haus".into(), "192.168.1.5".into(), 8016, "1".into()));
        assert_eq!(validate(&inp("X", "http://Abc.myfritz.net:9000/", None, Some("2"))).unwrap().1, "abc.myfritz.net");
        assert!(validate(&inp("", "a.b", None, None)).is_err());
        assert!(validate(&inp("X", "", None, None)).is_err());
        assert!(validate(&inp("X", "a b", None, None)).is_err());
        assert!(validate(&inp("X", "a.b/../x", None, None)).is_err());
        assert!(validate(&inp("X", "a.b", Some(0), None)).is_err());
        assert!(validate(&inp("X", "a.b", None, Some("7"))).is_err());
    }

    #[test]
    fn store_roundtrip() {
        let dir = std::env::temp_dir().join(format!("ema-sys-{}", now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s.json").to_string_lossy().to_string();
        let seed = System { id: "default".into(), name: "Meine Anlage".into(), host: "10.0.0.1".into(), port: 8016, group: "1".into() };
        let st = Store::load(&path, Some(seed));
        assert_eq!(st.list().len(), 1);
        let a = st.upsert(inp("Ferienhaus", "x.myfritz.net", Some(8016), Some("1"))).unwrap();
        let st2 = Store::load(&path, None); // file wins over seed
        assert_eq!(st2.list().len(), 2);
        st2.reorder(&[a.id.clone()]).unwrap();
        assert_eq!(st2.list()[0].id, a.id);
        st2.delete("default").unwrap();
        assert_eq!(Store::load(&path, None).list().len(), 1);
        assert!(st2.delete("nope").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
