# Changelog

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
