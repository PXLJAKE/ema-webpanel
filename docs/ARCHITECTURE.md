# Architecture & protocol notes

## Components

### Frontend — `server/public/`
A static PWA (under `server/` so the Plesk Document Root can sit inside the app root). Views:
- **Bedienteil** — 2-line LCD, status LEDs, numeric keypad, EIN/AUS, function keys, Revision.
- **Buttons** — momentary push-buttons (Taster) with a status LED; edit mode allows
  reorder (drag & drop) and remove. Three display-only alarm indicators on top.
- **LEDs** — status indicators; same edit mode (reorder / remove).
- **Anlagen** — list of configured panels (profiles).

All labels, button counts and LED counts come from the panel, not from hardcoded lists.

### Backend — `server/`
Node.js service that:
1. Authenticates the browser (email + password → session).
2. Talks to the panel over plain HTTP, polls its XML pages, and maintains current state.
3. Exposes a clean JSON API (and later a WebSocket channel for push updates) to the PWA.
4. Keeps the panel host and the optional AES key as environment variables.

## Panel protocol (summary)

> Reverse-engineered for interoperability with the owner's **own** device. The raw
> reverse-engineering material is **not** part of this repository.

- **Transport:** plain HTTP GET only (no HTTPS, no POST). Everything via URL parameters.
- **XML endpoints:**
  - `ui_init.xml` — session id, revision, start state
  - `ui_io.xml` — LED / display state
  - `ui_12540.xml` — keypad page (model 12540)
  - `ui_12540_sz.xml` — special characters
- **Request parameters:** `?B-GR=<group>&S-ID=<session id>&CNT=<counter>`
  - `B-GR` is the **Benutzergruppe** (user group), sent as-is — *not* an encrypted command.
  - First call is `ui_init.xml` with `S-ID` empty and `CNT=FFFF`; the response carries
    `Session_ID` and `Counter`. Each later request uses `S-ID=<session>` and
    `CNT=<last response Counter>`.
  - In plaintext mode (no key) the URL is sent raw — reading state needs no crypto.
- **Response XML fields:** `Text_80_01` / `Text_80_02` (two 80-char LCD lines),
  LED tags `RED_ON|OFF|BLINK_0`, `GREEN_*`, `YELLOW_*`, `Cursor_Pos`, `Cursor_Blink`,
  `Counter`, `Session_ID`, `Revision`.

### Two payload modes

The `B-GR` payload is built differently depending on whether the panel has a key configured:

| Mode       | When                         | `B-GR` value                                   |
|------------|------------------------------|------------------------------------------------|
| Plaintext  | no key set on the panel      | the raw command payload (no encryption)        |
| AES        | a key is set on the panel    | `AES/ECB/PKCS7` of the payload, hex-encoded     |

- AES: `AES/ECB/PKCS7Padding`, charset `Cp1250`, key = 32 hex chars (AES-128),
  ciphertext hex-encoded. In AES mode the response's root node is `crypt`.
- The backend selects the mode from the presence of an `EMA_AES_KEY` env var, so the
  same code path works now (plaintext) and later (AES) without changes.

### Still to confirm
- Exact XML list tag names for the button / LED lists (need a real capture).
- Key-press / command payload layout — **not implemented** (the service is read-only).

A read-only connection test (`GET /api/connect-test`, login required) hits the panel,
returns the parsed LCD snapshot plus the raw XML of each page, and is the intended way to
discover those real tag names against the live panel.

Best confirmed via a real traffic capture or by reading the panel's own web UI. These are
needed before the backend's command-building is wired up end-to-end; display/polling can be
built against the known response fields first.

## Roadmap

1. **Frontend prototype** — done (design, demo data). ← we are here
2. **Backend skeleton** — service, config, auth stub, protocol module stubs. ← in progress
3. **Wire it up** — confirm payload format, implement polling + command build, connect PWA.
4. **Harden** — firewall to backend IP / VPN, close public port.
