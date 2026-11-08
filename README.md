# ema-webpanel

A modern, clean **Progressive Web App (PWA)** as a virtual control panel ("Bedienteil")
for an intrusion alarm panel (EMA), with light and dark mode. It is a ground-up
reimplementation of an older, no-longer-maintained Android keypad app — rebuilt as a
responsive web app with a small backend that talks to the panel.

> **Status:** early development. The frontend is a working design prototype with demo
> data; the backend is a skeleton. Not yet wired to a live panel.

---

## ⚠️ Disclaimer

This is an **independent, unofficial** project. It is **not affiliated with, endorsed by,
or connected to Honeywell, Novar, or any manufacturer**. All product and company names are
the property of their respective owners and are used only to describe interoperability.

This software controls / displays the state of a **security alarm system**. It is provided
**"as is", without any warranty**, and is **not a certified safety device**. Do not rely on
it for life-safety or security-critical functions. Use entirely at your own risk.

This repository contains **only original code**. It does **not** include any decompiled
code, binaries, assets, or trademarks from the original application.

---

## Architecture

```
┌──────────────┐   HTTPS    ┌─────────────────────┐   HTTP    ┌──────────────┐
│   PWA (web/) │ ─────────▶ │  Backend (server/)  │ ────────▶ │  Alarm panel │
│  browser/app │ ◀───────── │  Node.js on Plesk   │ ◀──────── │   (EMA/ÜG)   │
└──────────────┘  JSON/WS   └─────────────────────┘   XML     └──────────────┘
```

- **Frontend (`web/`)** — static PWA (HTML/CSS/JS). No secrets. Renders the keypad,
  buttons and status LEDs; all labels come from the panel, not hardcoded.
- **Backend (`server/`)** — Node.js. Holds the panel host (and, when enabled, the AES key)
  as **secrets / environment variables**, does the polling and any crypto, parses the
  panel's XML and exposes clean JSON (later: WebSocket for push updates). Login via
  email + password so the panel is never exposed directly to the browser.
- **Panel** — speaks plain HTTP with XML pages. Two modes are supported, mirroring the
  original app: **plaintext** (no key configured on the panel) and **AES** (a key is set).

See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for details.

---

## Security model

- Panel credentials / AES key live **only on the backend** as environment variables — never
  in the frontend or the repo.
- The browser authenticates to the backend (email + password); the backend talks to the panel.
- **Hardening roadmap:** restrict the panel's port to the backend's IP (firewall), or put it
  behind a VPN (WireGuard/Tailscale) and close the public port entirely.

---

## Development

```bash
cd server
cp .env.example .env     # fill in your panel host etc.
npm install
npm run dev              # serves the API and the PWA from web/
```

Then open the printed URL. Without a configured panel the PWA still runs in demo mode.

---

## Deployment (Plesk, via Git)

This repo is laid out to deploy with **Plesk → Git**:

1. In Plesk, add this repository under **Git** and enable automatic deployment on push.
2. Set the **deployment actions** to run in `server/`:
   ```
   cd server && npm ci && npm run build --if-present
   ```
   and (re)start the Node.js app.
3. Configure the Node.js app's **document root** / startup file to `server/src/index.js`.
4. Put the panel host, login and (optional) AES key into the app's **environment variables**
   in Plesk — not into the repo.

Every `git push` then updates the deployment automatically.

---

## License

[MIT](LICENSE) © PXLJAKE
