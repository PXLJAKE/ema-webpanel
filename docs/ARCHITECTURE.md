# Architecture

## Overview

```
Browser (HA sidebar, Ingress)  --HTTP-->  ema-panel (Rust)  --HTTP GET-->  panel (LAN)
Browser / PWA (LAN or tunnel)  --HTTP-->  ema-panel (Rust)
```

One binary and one image, two run modes:

- **Home Assistant add-on** (detected by `SUPERVISOR_TOKEN` or `/data/options.json`): HA
  authenticates the user and proxies the page through Ingress. The add-on options seed the
  settings; a changed option wins on the next start.
- **Standalone** (plain Docker): everything is set up in the UI. An optional password protects
  the API (signed HttpOnly cookie, Argon2 hash). `EMA_MODE` overrides the detection.

Persistent files in `DATA_DIR` (default `/data`): `systems.json` (the systems) and
`settings.json` (control switch, poll interval, password hash, cookie key). ema-panel is the
only component that talks to the panel.

## Components

- **Poller** (one background task) owns the panel session (session id + counter), polls
  `ui_io.xml` and `ui_12540.xml`, and keeps the latest state in memory.
- **API** (`/api/state`, `/api/health`, `/api/connect-test`, `/api/key`) serves that state and
  accepts key presses. `POST /api/key` is the only route that can make the panel do anything.
- **UI** is one HTML file embedded into the binary; all URLs are relative (Ingress base path).

## Panel protocol (summary)

Plain HTTP GET, XML responses (ISO-8859-1):

`GET /<page>?B-GR=<user group>&S-ID=<session>&CNT=<counter>[&T<code>]`

- First request is `ui_init.xml` with an empty session and counter `FFFF`; the response carries
  `Session_ID` and `Counter`. Every later request uses the counter of the previous response.
- `ui_io.xml`: colours of the 8 function buttons, 3 indicators and 8 LEDs.
  `ui_12540.xml`: the 8 status LEDs and the two LCD lines (comma separated hex character codes).
- A key press is appended as `&T<code>`: keypad keys on `ui_12540.xml`, function buttons
  (`T01`..`T08`) on `ui_io.xml`.

## Safety

- Default is view only. `control_enabled` gates every command.
- Fixed key whitelist, JSON-only POST, single owner of the session, no automatic retries,
  every command logged.
- The UI asks for confirmation before arm/disarm and function buttons.
