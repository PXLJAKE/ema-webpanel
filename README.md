# EMA Webpanel – Home Assistant add-on

A modern, fast web keypad for a compatible intrusion alarm panel, running **locally** as a
Home Assistant add-on. It appears in the HA sidebar (Ingress), works on phone, tablet and
desktop, and has a light and a dark theme. Nothing leaves your network.

> **Disclaimer.** This is an **unofficial** community project. It is **not affiliated with,
> endorsed by or supported by** the manufacturer of the alarm panel or of its original app.
> It controls a safety-relevant system. **Use it at your own risk, with no warranty** (see
> [LICENSE](LICENSE)). Do not rely on it as the only way to monitor or operate your alarm system.

## Features

- Live display of the panel: LCD text, status LEDs, function buttons and indicator LEDs
- Virtual keypad (digits, ▲▼, EIN/AUS, function keys, Revision) and the panel's function buttons
- **View-only by default.** Sending commands is an explicit add-on option (`control_enabled`)
- Arm/disarm and function buttons always ask for confirmation first
- **Connects on demand.** The panel allows only one keypad user at a time, so the add-on connects when you tap *Verbinden*, disconnects after 5 minutes without activity or when you tap *Trennen*, leaving the panel free for other apps
- Responsive layout: phone (bottom tabs) and tablet/desktop (side rail, wide keypad)
- Small static Rust binary, no cloud, no account, no telemetry

## Install

1. In Home Assistant open **Settings → Add-ons → Add-on store → ⋮ → Repositories** and add
   `https://github.com/PXLJAKE/ema-webpanel`.
2. Install **Alarm Keypad (EMA)**, open the **Configuration** tab and set the panel address
   (and user group). Leave **Allow control** off for a first test.
3. Start the add-on and open **Alarmanlage** in the sidebar.

(Alternatively copy the `ema-panel/` folder into your HA `/addons` share to install it as a
local add-on.)

## Options

| Option | Default | Meaning |
|---|---|---|
| `panel_host` | `192.168.178.99` | IP/hostname of the transmitter unit in your LAN |
| `panel_port` | `8016` | Port of the unit |
| `user_group` | `1` | Same group number as in the original app's panel entry |
| `control_enabled` | `false` | `false` = view only. `true` = keys/buttons send real commands |
| `poll_interval_ms` | `1000` | How often state is read |

## Safety model

- Reading state never sends a command. Commands exist only behind `control_enabled`.
- The backend accepts only a fixed whitelist of keys (no panic/duress keys exist in the UI or API).
- Every command needs an explicit tap; arm/disarm and buttons need a second confirmation.
- Commands are never retried automatically, so a press can be lost but never doubled.
- Every sent command is logged by the add-on.
- Access is through Home Assistant's own authentication (Ingress); the add-on exposes no port.

## Development

```bash
cd ema-panel
cargo test
PANEL_HOST=192.168.x.x CONTROL_ENABLED=false PORT=8099 OPTIONS_PATH=/nonexistent cargo run
```

The UI is a single self-contained file, `ema-panel/web/index.html`, embedded into the binary.
All API calls are relative so it works behind the Ingress path.

## Notes

This project was written by observing the traffic of the author's own panel. No vendor code,
logos or app files are included in this repository.

## License

MIT – see [LICENSE](LICENSE).

---

### Kurz auf Deutsch

Lokales Home-Assistant-Add-on mit modernem Bedienteil für eine kompatible Einbruchmeldeanlage.
**Standardmäßig nur Ansicht**; Steuern schaltest du in den Add-on-Optionen frei. EIN/AUS und
Taster fragen immer nach. Inoffiziell, ohne Gewähr, Nutzung auf eigene Gefahr.
