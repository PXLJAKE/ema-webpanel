//! Home Assistant entities over MQTT discovery, fed by a background session of its own.
//!
//! The panel lets only one user at a time onto the keypad page (`ui_12540`), but the
//! button/LED page (`ui_io`) works in several sessions at once. So this task keeps a permanent
//! session that only ever reads `ui_io` (with the system's own HA user group, which the panel
//! can restrict to buttons and LEDs) and leaves the keypad free until someone opens it in the UI.
//! Verified on a real panel: an io-only session does not take the keypad.
//!
//! Taster become HA buttons only while "control" is on AND the `mqtt_buttons` option is on.
//! Presses from HA go through the same whitelist and are logged; they are never retried.

use crate::panel::{self, InitInfo, IoInfo, LedVal, Panel};
use crate::systems::System;
use crate::{Shared, VERSION};
use rumqttc::{AsyncClient, Event, LastWill, MqttOptions, Packet, QoS};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

const BASE: &str = "ema_panel";
const DISCOVERY: &str = "homeassistant";

#[derive(Debug, Clone)]
pub struct Broker {
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub pass: Option<String>,
}

/// `MQTT_HOST` (+ `MQTT_PORT`, `MQTT_USER`, `MQTT_PASSWORD`) wins; inside Home Assistant the
/// Supervisor hands out the Mosquitto add-on's credentials (`services: mqtt:want`).
async fn find_broker() -> Result<Broker, String> {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    if let Some(host) = env("MQTT_HOST") {
        return Ok(Broker {
            host,
            port: env("MQTT_PORT")
                .and_then(|p| p.parse().ok())
                .unwrap_or(1883),
            user: env("MQTT_USER"),
            pass: env("MQTT_PASSWORD"),
        });
    }
    let token = env("SUPERVISOR_TOKEN").ok_or("kein MQTT-Broker: MQTT_HOST setzen oder in Home Assistant den Mosquitto-Broker installieren")?;
    let text = reqwest::Client::new()
        .get("http://supervisor/services/mqtt")
        .header("Authorization", format!("Bearer {token}"))
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("Supervisor nicht erreichbar: {e}"))?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    let v: Value =
        serde_json::from_str(&text).map_err(|e| format!("Supervisor-Antwort unlesbar: {e}"))?;
    let d = v
        .get("data")
        .filter(|d| d.get("host").is_some())
        .ok_or("kein MQTT-Dienst in Home Assistant (Mosquitto-Broker installiert?)")?;
    let s = |k: &str| {
        d.get(k)
            .and_then(|x| x.as_str())
            .map(String::from)
            .filter(|x| !x.is_empty())
    };
    Ok(Broker {
        host: s("host").unwrap_or_default(),
        port: d.get("port").and_then(|p| p.as_u64()).unwrap_or(1883) as u16,
        user: s("username"),
        pass: s("password"),
    })
}

/// Topic-safe id of a system.
pub fn topic_id(id: &str) -> String {
    let s: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() {
        "panel".into()
    } else {
        s
    }
}

pub fn on_off(v: Option<&LedVal>) -> &'static str {
    match v.map(|v| v.state.as_str()) {
        Some("on") | Some("blink") => "ON",
        _ => "OFF",
    }
}

fn attrs(v: Option<&LedVal>) -> String {
    match v {
        Some(v) => json!({ "color": v.color, "blink": v.state == "blink" }).to_string(),
        None => "{}".into(),
    }
}

/// One discovery entry: (config topic, retained config payload or "" to remove it).
#[derive(Debug, PartialEq)]
pub struct Disc {
    pub topic: String,
    pub payload: String,
}

/// Everything Home Assistant needs to know about one system. Unused (empty) slots and
/// disabled buttons get an empty payload so an entity from an earlier start disappears.
pub fn discovery(sys: &System, info: &InitInfo, buttons_enabled: bool) -> Vec<Disc> {
    let tid = topic_id(&sys.id);
    let node = format!("{BASE}_{tid}");
    let device = json!({
        "identifiers": [node],
        "name": sys.name,
        "manufacturer": "ema-webpanel (inoffiziell)",
        "model": "Alarmanlage über IP",
        "sw_version": VERSION,
    });
    let avail =
        json!([{ "topic": format!("{BASE}/status") }, { "topic": format!("{BASE}/{tid}/panel") }]);
    let mut out = Vec::new();
    let mut push = |component: &str, object: String, cfg: Option<Value>| {
        out.push(Disc {
            topic: format!("{DISCOVERY}/{component}/{node}/{object}/config"),
            payload: cfg.map(|c| c.to_string()).unwrap_or_default(),
        });
    };

    push(
        "binary_sensor",
        "connection".into(),
        Some(json!({
            "name": "Verbindung zur Anlage",
            "unique_id": format!("{node}_connection"),
            "state_topic": format!("{BASE}/{tid}/panel"),
            "payload_on": "online",
            "payload_off": "offline",
            "device_class": "connectivity",
            "entity_category": "diagnostic",
            "availability_topic": format!("{BASE}/status"),
            "has_entity_name": true,
            "device": device,
        })),
    );

    let sensor = |name: &str, uid: String, state: String| {
        json!({
            "name": name,
            "unique_id": uid,
            "state_topic": state,
            "json_attributes_topic": format!("{}/attr", state),
            "payload_on": "ON",
            "payload_off": "OFF",
            "availability": avail,
            "availability_mode": "all",
            "has_entity_name": true,
            "device": device,
        })
    };

    // LED page (8): the panel's own status LEDs, e.g. "Einbruch Haus"
    for i in 1..=8usize {
        let name = info
            .led_names
            .get(i - 1)
            .map(|n| n.trim().to_string())
            .unwrap_or_default();
        let cfg = (!name.is_empty()).then(|| {
            sensor(
                &name,
                format!("{node}_led{i}"),
                format!("{BASE}/{tid}/led/{i}"),
            )
        });
        push("binary_sensor", format!("led{i}"), cfg);
    }
    // button page: 8 Taster (status LED + optional button) and 3 display-only indicators
    for i in 1..=11usize {
        let name = info
            .button_names
            .get(i - 1)
            .map(|(_, n)| n.trim().to_string())
            .unwrap_or_default();
        if i <= 8 {
            let cfg = (!name.is_empty()).then(|| {
                sensor(
                    &format!("{name} Status"),
                    format!("{node}_taster{i}_state"),
                    format!("{BASE}/{tid}/taster/{i}"),
                )
            });
            push("binary_sensor", format!("taster{i}_state"), cfg);
            let btn = (!name.is_empty() && buttons_enabled).then(|| {
                json!({
                    "name": name,
                    "unique_id": format!("{node}_taster{i}"),
                    "command_topic": format!("{BASE}/{tid}/taster/{i}/press"),
                    "payload_press": "PRESS",
                    "availability": avail,
                    "availability_mode": "all",
                    "has_entity_name": true,
                    "device": device,
                })
            });
            push("button", format!("taster{i}"), btn);
        } else {
            let k = i - 8;
            let cfg = (!name.is_empty()).then(|| {
                sensor(
                    &name,
                    format!("{node}_indicator{k}"),
                    format!("{BASE}/{tid}/indicator/{k}"),
                )
            });
            push("binary_sensor", format!("indicator{k}"), cfg);
        }
    }
    // status lamps of the keypad page: only known while the keypad is open in the app
    let keypad_avail =
        json!([{ "topic": format!("{BASE}/status") }, { "topic": format!("{BASE}/{tid}/keypad") }]);
    for tag in STATUS_TAGS {
        let lower = tag.to_ascii_lowercase();
        let cfg = json!({
            "name": format!("Status {}", panel::led_label(tag)),
            "unique_id": format!("{node}_status_{lower}"),
            "state_topic": format!("{BASE}/{tid}/status/{lower}"),
            "json_attributes_topic": format!("{BASE}/{tid}/status/{lower}/attr"),
            "payload_on": "ON",
            "payload_off": "OFF",
            "availability": keypad_avail,
            "availability_mode": "all",
            "has_entity_name": true,
            "device": device,
        });
        push("binary_sensor", format!("status_{lower}"), Some(cfg));
    }
    out
}

/// The status lamps of the keypad page (`ui_12540`).
pub const STATUS_TAGS: [&str; 8] = [
    "Betrieb",
    "S_Alarm",
    "S_Stoerung",
    "Sys_Stoerung",
    "Einbruch",
    "Sabotage",
    "Ueberfall",
    "Pol_Not",
];

/// State messages for the status lamps, taken from the app's own keypad session.
pub fn status_states(tid: &str, leds: &[crate::panel::Led]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for tag in STATUS_TAGS {
        let lower = tag.to_ascii_lowercase();
        let v = leds.iter().find(|l| l.tag == tag).map(|l| LedVal {
            color: l.color.clone(),
            state: l.state.clone(),
        });
        out.push((
            format!("{BASE}/{tid}/status/{lower}"),
            on_off(v.as_ref()).to_string(),
        ));
        out.push((
            format!("{BASE}/{tid}/status/{lower}/attr"),
            attrs(v.as_ref()),
        ));
    }
    out
}

/// State messages (topic, payload) of one `ui_io` answer.
pub fn states(tid: &str, io: &IoInfo) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut add = |path: String, v: Option<&LedVal>| {
        out.push((format!("{BASE}/{tid}/{path}"), on_off(v).to_string()));
        out.push((format!("{BASE}/{tid}/{path}/attr"), attrs(v)));
    };
    for i in 0..8 {
        add(format!("led/{}", i + 1), io.leds.get(i));
    }
    for i in 0..8 {
        add(format!("taster/{}", i + 1), io.buttons.get(i));
    }
    for i in 8..11 {
        add(format!("indicator/{}", i - 7), io.buttons.get(i));
    }
    out
}

/// `ema_panel/<tid>/taster/<n>/press` -> n (1..=8)
pub fn press_index(topic: &str, tid: &str) -> Option<u8> {
    let rest = topic.strip_prefix(&format!("{BASE}/{tid}/taster/"))?;
    let n: u8 = rest.strip_suffix("/press")?.parse().ok()?;
    (1..=8).contains(&n).then_some(n)
}

async fn publish(c: &AsyncClient, topic: &str, payload: &str, retain: bool) {
    if let Err(e) = c
        .publish(topic, QoS::AtLeastOnce, retain, payload.as_bytes().to_vec())
        .await
    {
        eprintln!("mqtt publish {topic}: {e}");
    }
}

pub async fn run(shared: Arc<Shared>) {
    let o = shared.opts.clone();
    if !o.mqtt_entities {
        println!("Home-Assistant-Entitäten (MQTT) sind aus (Option mqtt_entities).");
        return;
    }
    let broker = loop {
        match find_broker().await {
            Ok(b) => break b,
            Err(e) => {
                eprintln!("MQTT: {e}; neuer Versuch in 60 s");
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        }
    };
    println!("MQTT: Broker {}:{}", broker.host, broker.port);

    let mut mo = MqttOptions::new(
        format!("{BASE}_{}", std::process::id()),
        broker.host.clone(),
        broker.port,
    );
    mo.set_keep_alive(Duration::from_secs(30));
    mo.set_last_will(LastWill::new(
        format!("{BASE}/status"),
        "offline",
        QoS::AtLeastOnce,
        true,
    ));
    if let Some(u) = &broker.user {
        mo.set_credentials(u.clone(), broker.pass.clone().unwrap_or_default());
    }
    let (client, mut events) = AsyncClient::new(mo, 256);
    let announce = Arc::new(AtomicBool::new(false));
    let (ptx, mut prx) = mpsc::unbounded_channel::<String>();
    {
        let announce = announce.clone();
        tokio::spawn(async move {
            loop {
                match events.poll().await {
                    Ok(Event::Incoming(Packet::ConnAck(_))) => {
                        println!("MQTT verbunden");
                        announce.store(true, Ordering::Relaxed);
                    }
                    Ok(Event::Incoming(Packet::Publish(p))) => {
                        let _ = ptx.send(p.topic.clone());
                    }
                    Ok(_) => {}
                    Err(e) => {
                        eprintln!("MQTT: {e}; neuer Versuch in 10 s");
                        tokio::time::sleep(Duration::from_secs(10)).await;
                    }
                }
            }
        });
    }

    let mut cur: Option<System> = None;
    let mut panel = Panel::new("", 0, "1");
    let mut info: Option<InitInfo> = None;
    let mut last: HashMap<String, String> = HashMap::new();
    let mut buttons_on = false;
    let mut errors = 0u32;
    let mut backoff = 5u64;

    loop {
        if shared.paused.load(Ordering::Relaxed) {
            info = None; // fresh session after the probe
            tokio::time::sleep(Duration::from_millis(500)).await;
            continue;
        }
        let Some(sys) = shared.systems.first() else {
            tokio::time::sleep(Duration::from_secs(5)).await;
            continue;
        };
        let group = sys.ha_group().to_string();
        if cur.as_ref() != Some(&sys) {
            if let Some(old) = &cur {
                publish(
                    &client,
                    &format!("{BASE}/{}/panel", topic_id(&old.id)),
                    "offline",
                    true,
                )
                .await;
            }
            println!(
                "Hintergrund-Sitzung für HA: {} ({}:{}, Gruppe {group}, nur Taster/LEDs)",
                sys.name, sys.host, sys.port
            );
            panel = Panel::with_key(&sys.host, sys.port, &group, &sys.key);
            info = None;
            cur = Some(sys.clone());
        }
        let tid = topic_id(&sys.id);

        if info.is_none() {
            match panel.init().await {
                Ok((i, _)) => {
                    println!("Hintergrund-Sitzung gestartet (Gruppe {group})");
                    info = Some(i);
                    errors = 0;
                    backoff = 5;
                    announce.store(true, Ordering::Relaxed);
                }
                Err(e) => {
                    eprintln!("Hintergrund-Sitzung: {e}; neuer Versuch in {backoff} s");
                    publish(&client, &format!("{BASE}/{tid}/panel"), "offline", true).await;
                    tokio::time::sleep(Duration::from_secs(backoff)).await;
                    backoff = (backoff * 2).min(120);
                    continue;
                }
            }
        }
        let i = info.clone().unwrap();

        let want_buttons = shared.settings.control() && o.mqtt_buttons;
        if want_buttons != buttons_on {
            buttons_on = want_buttons;
            announce.store(true, Ordering::Relaxed);
        }
        if announce.swap(false, Ordering::Relaxed) {
            for d in discovery(&sys, &i, buttons_on) {
                publish(&client, &d.topic, &d.payload, true).await;
            }
            if let Err(e) = client
                .subscribe(format!("{BASE}/{tid}/taster/+/press"), QoS::AtLeastOnce)
                .await
            {
                eprintln!("mqtt subscribe: {e}");
            }
            publish(&client, &format!("{BASE}/status"), "online", true).await;
            last.clear(); // resend all states
        }

        // presses from HA: only when allowed right now, whitelisted, logged, never retried
        let mut cmd = String::new();
        while let Ok(topic) = prx.try_recv() {
            let Some(n) = press_index(&topic, &tid) else {
                continue;
            };
            if !(shared.settings.control() && o.mqtt_buttons) {
                println!("HA-Taster {n} ignoriert: Steuern oder mqtt_buttons ist aus");
                continue;
            }
            if let Some(code) = panel::taster_code(n) {
                println!("COMMAND taster={n} code={code} (Home Assistant)");
                cmd.push('T');
                cmd.push_str(&code);
            }
        }

        match panel.read_io(&cmd).await {
            Ok((io, _)) => {
                errors = 0;
                let mut msgs = states(&tid, &io);
                msgs.push((format!("{BASE}/{tid}/panel"), "online".into()));
                // status lamps ride along while the app's keypad session for this system runs
                let keypad = {
                    let snap = shared.snap.read().unwrap();
                    let same = shared
                        .target
                        .read()
                        .unwrap()
                        .as_ref()
                        .map(|t| t.id == sys.id)
                        .unwrap_or(false);
                    (same
                        && crate::wanted(&shared)
                        && snap.connected
                        && snap.error.is_none()
                        && snap.leds.iter().any(|l| !l.tag.starts_with("page:")))
                    .then(|| snap.leds.clone())
                };
                match keypad {
                    Some(leds) => {
                        msgs.extend(status_states(&tid, &leds));
                        msgs.push((format!("{BASE}/{tid}/keypad"), "online".into()));
                    }
                    None => msgs.push((format!("{BASE}/{tid}/keypad"), "offline".into())),
                }
                for (t, p) in msgs {
                    if last.get(&t) != Some(&p) {
                        publish(&client, &t, &p, true).await;
                        last.insert(t, p);
                    }
                }
            }
            Err(e) => {
                errors += 1;
                eprintln!("Hintergrund-Sitzung Fehler ({errors}): {e}");
                if errors >= 3 {
                    info = None;
                    errors = 0;
                    last.remove(&format!("{BASE}/{tid}/panel"));
                    publish(&client, &format!("{BASE}/{tid}/panel"), "offline", true).await;
                }
            }
        }
        let wait = if cmd.is_empty() {
            shared.settings.poll_ms().max(1000)
        } else {
            300
        };
        tokio::time::sleep(Duration::from_millis(wait)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sys() -> System {
        System {
            id: "default".into(),
            name: "Meine Anlage".into(),
            host: "h".into(),
            port: 8016,
            group: "1".into(),
            key: String::new(),
            ha_group: String::new(),
        }
    }

    fn info() -> InitInfo {
        InitInfo {
            led_names: vec!["Feueralarm".into(), "".into()],
            button_names: (1..=11)
                .map(|i| {
                    (
                        format!("B{i}"),
                        if i == 4 {
                            String::new()
                        } else {
                            format!("Name {i}")
                        },
                    )
                })
                .collect(),
            ..Default::default()
        }
    }

    fn lv(state: &str) -> LedVal {
        LedVal {
            color: "red".into(),
            state: state.into(),
        }
    }

    #[test]
    fn buttons_only_when_enabled() {
        let off = discovery(&sys(), &info(), false);
        assert!(off
            .iter()
            .filter(|d| d.topic.contains("/button/"))
            .all(|d| d.payload.is_empty()));
        let on = discovery(&sys(), &info(), true);
        let b1 = on
            .iter()
            .find(|d| d.topic == "homeassistant/button/ema_panel_default/taster1/config")
            .unwrap();
        assert!(b1
            .payload
            .contains("\"command_topic\":\"ema_panel/default/taster/1/press\""));
        // empty slot 4 has no button, indicators never get one
        assert!(on
            .iter()
            .find(|d| d
                .topic
                .ends_with("/button/ema_panel_default/taster4/config"))
            .unwrap()
            .payload
            .is_empty());
        assert_eq!(
            on.iter().filter(|d| d.topic.contains("/button/")).count(),
            8
        );
    }

    #[test]
    fn empty_names_remove_entities() {
        let d = discovery(&sys(), &info(), false);
        let led1 = d
            .iter()
            .find(|d| d.topic.ends_with("/led1/config"))
            .unwrap();
        assert!(led1.payload.contains("Feueralarm"));
        assert!(d
            .iter()
            .find(|d| d.topic.ends_with("/led2/config"))
            .unwrap()
            .payload
            .is_empty());
        assert!(d
            .iter()
            .find(|d| d.topic.ends_with("/indicator3/config"))
            .unwrap()
            .payload
            .contains("Name 11"));
    }

    #[test]
    fn state_mapping() {
        let io = IoInfo {
            counter: "1".into(),
            buttons: vec![lv("blink"), lv("off")],
            leds: vec![lv("on")],
        };
        let s: HashMap<_, _> = states("default", &io).into_iter().collect();
        assert_eq!(s["ema_panel/default/led/1"], "ON");
        assert_eq!(s["ema_panel/default/led/2"], "OFF");
        assert_eq!(s["ema_panel/default/taster/1"], "ON");
        assert!(s["ema_panel/default/taster/1/attr"].contains("\"blink\":true"));
        assert_eq!(s["ema_panel/default/taster/2"], "OFF");
    }

    #[test]
    fn press_topics() {
        assert_eq!(
            press_index("ema_panel/default/taster/3/press", "default"),
            Some(3)
        );
        assert_eq!(
            press_index("ema_panel/default/taster/9/press", "default"),
            None
        );
        assert_eq!(
            press_index("ema_panel/other/taster/3/press", "default"),
            None
        );
        assert_eq!(
            press_index("ema_panel/default/taster/3/state", "default"),
            None
        );
        assert_eq!(topic_id("Ab-1"), "ab_1");
    }
}
