# Changelog

## 0.7.0
- English and German. New option `language` (`auto` by default): `auto` follows Home Assistant (the HA profile language in the app, the HA system language for entity names), anything other than German becomes English; `en` or `de` fixes it. Each device can also choose in the app under ⚙ → Language. Standalone Docker follows the browser (or `UI_LANGUAGE`).
- Names that come from the panel (buttons, LEDs, LCD, system names) are never translated.
- Home Assistant entity names follow the language (e.g. "Panel connection", "Status Intrusion"); entity IDs that already exist stay the same.
- Reworded add-on option texts in English and German; add-on log messages are now English.
- The add-on reads the HA language through the Home Assistant API (`homeassistant_api: true`, read only). Sidebar entry renamed to "Alarm Keypad".
- Setup guide now in English (`docs/SETUP.md`) and German (`docs/SETUP.de.md`); new English screenshots.

## 0.6.0
- AES key per system (optional, hex): with a key, requests are sent and answers read encrypted like the original app (AES-ECB, PKCS7, hex); without one everything stays plain text. The key is stored in `/data/systems.json` and never sent to the browser; edit it in the system dialog (empty = keep, `-` = remove).
- "Gruppe für HA" per system: the Home Assistant background session can use a different user group than the keypad (e.g. main group 3, HA group 2 with buttons and LEDs only). Replaces the `mqtt_group` option.
- New HA entities "Status Betrieb/Einbruch/Sabotage/…": the keypad page's status lamps, available while the keypad is open in the app.
- Checked on the real panel: a session that only reads buttons/LEDs does not lock the keypad; a second keypad reader gets `Locked=YES` but still the status lamps.

## 0.5.0
- Home Assistant entities over MQTT discovery (option `mqtt_entities`, off by default; uses the Mosquitto broker, credentials come from the Supervisor). One device per system with binary sensors for the 8 LEDs of the LED page, the 8 Taster LEDs, the 3 indicators and the connection.
- A permanent background session feeds them. It only reads the button/LED page (`ui_io`) and never the keypad page, so the keypad stays free for the app and the original keypad. Option `mqtt_group` gives it a user group of its own (e.g. 2, restricted in the panel to buttons and LEDs).
- Taster as HA buttons only while *Steuern erlauben* and the option `mqtt_buttons` are both on (both off by default). Presses are whitelisted, logged and never retried; switching control off removes the buttons from HA.
- Outside HA: `MQTT_ENTITIES=1`, `MQTT_HOST`, `MQTT_PORT`, `MQTT_USER`, `MQTT_PASSWORD`, `MQTT_GROUP`, `MQTT_BUTTONS`.

## 0.4.1
- Diagnostics: `GET /api/probe?mode=lock&group2=2` checks, with plain read-only GETs, whether a session that only reads the button/LED page blocks the keypad, whether a locked keypad answer still carries the status lamps, and what a second user group gets. Groundwork for Home Assistant entities.

## 0.4.0
- Runs as a plain Docker container too (same image): `docker run -p 8099:8099 -v ema-data:/data ghcr.io/pxljake/ema-webpanel`. The app detects Home Assistant by the Supervisor token and otherwise runs standalone. No config files or variables needed; the first start walks you through the setup in the browser.
- *Steuern erlauben* and the poll interval are now settings in the app (⚙), stored in `/data/settings.json`. In Home Assistant the add-on options seed them and a changed option wins on the next start.
- Optional password for standalone use (Argon2 hash, 30-day login cookie, slowed down guessing, needed again to switch control on). A red bar warns when the page is reached from outside without a password.
- Installable as an app (PWA): manifest, icons, a small service worker that never caches live data.
- Image for amd64 and arm64 published to GHCR by GitHub Actions; `docker-compose.yml` with an optional Cloudflare Tunnel.
- New API: `GET/POST /api/settings`, `GET /api/auth/status`, `POST /api/auth/login`, `POST /api/auth/logout`, `POST /api/auth/password`.

## 0.3.0
- Manage several systems in the app: add, edit, delete and sort them on the Anlagen page (name, address, port, group), also remote ones via a MyFRITZ/DynDNS name. Tap a system to connect; one connection at a time. The list is stored in `/data/systems.json`; the add-on options only create the first entry on first start.
- New API: `GET/POST /api/systems`, `POST /api/systems/delete`, `POST /api/systems/order`; `POST /api/connect` takes an optional `id`.

## 0.2.0
- Connect on demand: the panel allows only one keypad user at a time, so the add-on no longer holds a session permanently. The UI asks to connect on start, the connection closes after 5 minutes without activity (key presses and a tap on the status pill extend it), and a "Trennen" button on every page lets go immediately so the original app can be used.
- New API: `POST /api/connect`, `POST /api/disconnect` (JSON only); `/api/state` reports `active`, `expiresAt`, `idleMs`.

## 0.1.4
- Names of function buttons, indicators and LEDs are read from the panel (they sit in a nested Text element).
- LCD is drawn as 2 x 40 cells with the panel's own custom glyphs (ui_12540_sz) and the cursor position.
- UI is called "Alarm Keypad"; the intro no longer claims read-only when control is enabled.

## 0.1.3
- Fix: the panel expects the answered counter plus one in the next request. Sending it unchanged made the panel close every request after the first.

## 0.1.0
- First release: live panel state, virtual keypad, function buttons, responsive UI.
- View-only by default; control behind an add-on option with confirmations.
