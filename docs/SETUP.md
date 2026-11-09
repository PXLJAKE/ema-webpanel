# Step-by-step setup

*Deutsche Version: [SETUP.de.md](SETUP.de.md)*

This guide takes you from zero to a working keypad, either as a **Home Assistant add-on** or as
a **Docker container**, and optionally on to sensors and buttons in Home Assistant.

> Unofficial project, no warranty. The app operates a real alarm system. Leave **Allow control
> off** for your first test, then nothing can happen.

**Contents**

1. [What you need](#1-what-you-need)
2. [Find your panel's details](#2-find-your-panels-details)
3. [Option A: Home Assistant add-on](#3-option-a-home-assistant-add-on)
4. [Option B: Docker (without Home Assistant)](#4-option-b-docker-without-home-assistant)
5. [Add systems and connect](#5-add-systems-and-connect)
6. [Switch on control](#6-switch-on-control)
7. [Sensors and buttons in Home Assistant (MQTT)](#7-sensors-and-buttons-in-home-assistant-mqtt)
8. [Install as an app](#8-install-as-an-app)
9. [Updating](#9-updating)
10. [Language](#10-language)
11. [Troubleshooting](#11-troubleshooting)

---

## 1. What you need

- An intrusion alarm panel with an **IP transmitter** that works with the *MB-Remote Control V2*
  app (tested: DS7700, which reports as "DS7500 IP"; see
  [compatibility](../README.md#compatible-systems)).
- The transmitter must be reachable on your home network, by default on **port 8016**.
- **Either** Home Assistant OS or Supervised (for the add-on) **or** any Docker host on the same
  network (Proxmox LXC, Synology, Raspberry Pi, NAS …).
- For sensors and buttons in Home Assistant, also the **Mosquitto broker** add-on.

## 2. Find your panel's details

The easiest place to look is the original app under **Anlagenauswahl** (system selection). You
need:

| Detail | Example | Note |
|---|---|---|
| Address | `192.168.178.99` | IP of the transmitter on your network. Best give it a fixed IP in your router. |
| Port | `8016` | The default. |
| User group | `1` | The same number as in the original app. |
| AES key | *(usually empty)* | Only if the original app has one entered: 32, 48 or 64 hex characters. |

Quick test in a browser (read-only, presses nothing):
`http://192.168.178.99:8016/ui_init.xml?B-GR=1&S-ID=00&CNT=FFFF`
If you get an XML answer with `<Session_ID>`, the panel is reachable. With an AES key you get
`<crypt>…</crypt>` instead, which is fine too.

## 3. Option A: Home Assistant add-on

### 3.1 Add the repository

1. In Home Assistant open **Settings → Add-ons → Add-on store**.
2. Top right **⋮ → Repositories**.
3. Enter `https://github.com/PXLJAKE/ema-webpanel` and click **Add**.
4. Close the dialog. **Alarm Keypad (EMA)** appears at the bottom of the store (if not, use
   **⋮ → Check for updates** once).

### 3.2 Install and configure

1. Open **Alarm Keypad (EMA)** and click **Install**. The add-on is built on your Home
   Assistant, which takes a few minutes.
2. **Configuration** tab:

   | Option | What to enter |
   |---|---|
   | Panel address | IP of the transmitter, e.g. `192.168.178.99` |
   | Panel port | `8016` |
   | User group | as in the original app, usually `1` |
   | Allow control | leave **off** |
   | Refresh interval (ms) | `1000` |
   | Home Assistant entities (MQTT) | off for now, see [section 7](#7-sensors-and-buttons-in-home-assistant-mqtt) |
   | Function buttons in Home Assistant | off |
   | Language | `auto` (follows Home Assistant), or a fixed `en` or `de` |

   Address, port and group are only used **on the very first start** to create the first
   system. After that you manage systems in the app (section 5).
3. **Info** tab: **Start**, and switch on **Show in sidebar**.
4. **Alarm Keypad** appears in the sidebar. Click it to open the app.

Inside Home Assistant, Home Assistant signs you in. No separate password is needed, and the
add-on has no port open to the outside.

## 4. Option B: Docker (without Home Assistant)

### 4.1 Start

```bash
docker run -d --name alarm-keypad --restart unless-stopped \
  -p 8099:8099 -v ema-data:/data ghcr.io/pxljake/ema-webpanel:latest
```

or with [`docker-compose.yml`](../docker-compose.yml) from the repository: `docker compose up -d`.

- **Synology** (Container Manager): *Project → Create*, paste the compose file, map port 8099
  and a folder to `/data`.
- **Proxmox**: in an LXC or a VM with Docker, same command.
- The container must reach the panel on your network. Normal bridge networking is enough.

### 4.2 First start in the browser

1. Open `http://<docker-host>:8099`.
2. The setup asks for a **password**. Recommended as soon as more than one device on your
   network can reach the page. You can also set it later under ⚙.
3. Then add your first system (section 5).

Everything is stored in the `/data` volume (`systems.json`, `settings.json`). Forgot the
password? Delete `settings.json` and restart the container: the password is gone, control is
off again, your systems are kept.

### 4.3 Access on the go

Do **not** forward port 8099 on your router. Use a **Cloudflare Tunnel** (prepared in the compose
file) and put **Cloudflare Access** in front of it (sign-in with an email code). The password in
the app is then a second layer. A VPN (WireGuard, Tailscale, router VPN) works too.

## 5. Add systems and connect

**Systems** tab (bottom left on a phone, in the side rail on a computer) → **＋ Add system**.

<p align="center"><img src="screenshots/phone-system-dialog-light.png" width="260" alt="Add system dialog"></p>

| Field | Meaning |
|---|---|
| Name | anything you like, e.g. "Home" |
| Address | IP or host name, from outside e.g. `name.myfritz.net` |
| Port | usually `8016` |
| Group | user group for the keypad, as in the original app |
| AES key | only if the panel has one, otherwise leave empty. When editing: empty = keep the saved key, `-` = remove it. The key is never sent back to the browser. |
| Group for HA | user group for the Home Assistant entities (section 7). "same as above" = the same group. |

Then tap the system and **Connect**. The header shows **live** with a countdown.

Good to know: the panel only ever lets **one** user use the keypad at a time. That is why the
app only connects when you ask it to, and disconnects after **5 minutes without use** or when
you tap **Disconnect**. If someone else is on the keypad right now (original app, second
device), the LCD shows that it is locked. Buttons and LEDs keep working.

Under **Edit** you can reorder and delete systems. The **first system in the list** is the one
reported to Home Assistant.

## 6. Switch on control

Only once the LCD, lamps and buttons look right:

- Add-on: in the app **⚙ → Allow control** (or the add-on option, applied on the next start).
- Docker: **⚙ → Allow control**. If a password is set, it is asked for again.

From then on digits and arrows are sent immediately; **Arm**, **Disarm** and every function
button ask first. There is deliberately **no** panic, hold-up or emergency key. Fire, water and
heating are display-only. Every command sent is listed in the log of the add-on or container.

## 7. Sensors and buttons in Home Assistant (MQTT)

This gives you the panel's lamps and buttons as entities, for dashboards and automations such
as an intrusion alert.

### 7.1 How it works

The add-on keeps a **separate, permanent connection** for this that **only reads the button and
LED page**. This was checked on a real panel: the connection does **not** take the keypad, so it
stays free for you, the original app and the real keypad.

### 7.2 A user group of its own (recommended)

Ideally the HA connection gets **its own user group** in the panel that is only allowed
**buttons and LEDs**, without the keypad. Then Home Assistant cannot operate the keypad, enforced
by the panel itself, and you decide there which buttons HA may press. How to set up groups
depends on your panel (installer programming of the transmitter).

Example: main group `3` for the keypad, group `2` with only buttons for HA. Then enter
**Group = 3** and **Group for HA = 2** for the system. Without a group of its own it works the
same with "same as above", just without this second safeguard.

### 7.3 Switch it on

1. Install and start the **Mosquitto broker** add-on if you do not have it yet (the **MQTT**
   integration should show up under *Settings → Devices & services*). The add-on fetches user
   and password automatically.
2. In the **Alarm Keypad (EMA)** add-on → **Configuration**: switch on **Home Assistant
   entities (MQTT)**, save and **restart** the add-on.
3. A device named after your system appears under *Settings → Devices & services → MQTT*.

Docker without Home Assistant but with your own broker: `-e MQTT_ENTITIES=1 -e MQTT_HOST=…
-e MQTT_PORT=1883 -e MQTT_USER=… -e MQTT_PASSWORD=…` (optionally `-e MQTT_BUTTONS=1`).

### 7.4 What you get

| Entity | Type | When available |
|---|---|---|
| The 8 LEDs of the LED page (e.g. "Einbruch Haus") | binary sensor | always |
| State of the 8 function buttons (e.g. "Weihnachtsbel. Status") | binary sensor | always |
| The three indicators (fire, water alarm, heating fault) | binary sensor | always |
| Panel connection | binary sensor (diagnostic) | always |
| Status Operation, Intrusion, Tamper, Hold-up, faults … | binary sensor | only while the keypad is open in the app |
| The function buttons themselves | button | only while **Allow control** **and** **Function buttons in Home Assistant** are on |

The fixed names (connection, status …) follow the add-on's language; the names of lamps and
buttons always come from the panel, and empty slots are left out. Blinking counts as on. Every
lamp has the attributes `color` and `blink`.

The panel only reports the keypad's status lamps (operation, intrusion and so on) on the keypad
page. Reading it permanently would lock the keypad, so they are only current while you have the
keypad open in the app. For alarm automations use the LEDs of the LED page (e.g. "Einbruch
Haus"), which are always there.

### 7.5 Press buttons from Home Assistant

1. Switch on **Allow control** in the app.
2. Switch on the add-on option **Function buttons in Home Assistant** and restart.

The buttons then appear on the device. If you switch control off again, they disappear from Home
Assistant and presses from HA are ignored. Careful: buttons in Home Assistant do **not** ask for
confirmation. Think about which automations may press them.

### 7.6 Example: intrusion alert

You find the entity ID on the device (e.g. `binary_sensor.home_einbruch_haus`).

```yaml
alias: Intrusion alert
triggers:
  - trigger: state
    entity_id: binary_sensor.home_einbruch_haus
    to: "on"
actions:
  - action: notify.mobile_app_your_phone
    data:
      title: "Alarm system"
      message: "Intrusion detected (house)!"
      data:
        push:
          interruption-level: critical
mode: single
```

And a simple dashboard card:

```yaml
type: entities
title: Alarm system
entities:
  - binary_sensor.home_panel_connection
  - binary_sensor.home_einbruch_haus
  - binary_sensor.home_feuer
  - binary_sensor.home_wasseralarm
```

## 8. Install as an app

The page can be installed like an app once it is served over **HTTPS** (e.g. through the
Cloudflare Tunnel): on a phone **Share → Add to Home Screen** (iPhone) or **⋮ → Install app**
(Android, Chrome), on a computer with the install icon in the address bar. In Home Assistant
just use the HA app, the keypad is in its sidebar.

## 9. Updating

- **Add-on**: Home Assistant shows updates under *Settings → Add-ons*. If not: in the store
  **⋮ → Check for updates**. The add-on is built locally (a few minutes).
- **Docker**: `docker pull ghcr.io/pxljake/ema-webpanel:latest` and recreate the container
  (`docker compose pull && docker compose up -d`).

Your systems and settings are kept either way.

## 10. Language

The app speaks English and German.

- **Add-on**: option **Language**. `auto` (default) follows Home Assistant: your HA profile
  language in the app, the HA system language for the entity names. Anything other than German
  becomes English. `en` or `de` fixes it. Applied on the next add-on start.
- **Docker**: the browser's language decides.
- **Per device**: in the app under **⚙ → Language** (Automatic, English, Deutsch).

Names that come from the panel (buttons, LEDs, LCD text) are shown as the panel sends them.

## 11. Troubleshooting

| Problem | Fix |
|---|---|
| "not connected" | Check address, port and group. **⚙ → Test connection to the panel** shows the raw answer of the panel. Does the browser test from section 2 work? |
| LCD says the keypad is locked | Someone else is on the keypad (original app, second device). Disconnect there or wait. Buttons and LEDs keep working. |
| "The panel answers encrypted" | The panel uses an AES key: enter it for the system. |
| "Decryption failed" | Wrong AES key. Enter it again. |
| HA entities missing | Is Mosquitto running? Is the option **Home Assistant entities** on and was the add-on restarted? The add-on log shows "MQTT connected" and "Background session started". |
| HA entities "unavailable" | The background connection cannot reach the panel, or the HA group has no rights. Check the log. |
| Status lamps in HA "unavailable" | Normal while the keypad is not open in the app (section 7.4). |
| No function buttons in HA | **Allow control** and **Function buttons in Home Assistant** both have to be on. |
| Panel does not accept a connection | It only allows a few sessions at a time. Close other apps and wait a moment. |
| "exit code 137" in the log during an update | Harmless: the old container was stopped hard during the update. |
| Umlauts on the LCD look wrong | Open **⚙ → Test connection to the panel** and report the raw data in an issue. |
