# Architecture

## Overview

```
Browser (HA sidebar, Ingress)  --HTTP-->  ema-panel add-on (Rust)  --HTTP GET-->  panel (LAN)
```

The add-on runs next to Home Assistant. Home Assistant authenticates the user and proxies the
page through Ingress. The add-on is the only component that talks to the panel.

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
