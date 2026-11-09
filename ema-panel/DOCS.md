# Alarm Keypad (EMA)

Modern web keypad for a compatible intrusion alarm panel, shown in the Home Assistant sidebar.

**Unofficial. No affiliation, no warranty, use at your own risk.**

Full step-by-step guide: https://github.com/PXLJAKE/ema-webpanel/blob/main/docs/SETUP.md
(Deutsch: https://github.com/PXLJAKE/ema-webpanel/blob/main/docs/SETUP.de.md)

## Setup

1. Set **Panel address** (IP of the transmitter on your network) and, if needed, the
   **User group** (the same number as in the original app).
2. Start the add-on and open **Alarm Keypad** in the sidebar.
3. Check that the LCD shows the panel text and the header says **live**.
4. Only when everything looks right: switch on **Allow control** in the app under ⚙
   (or the **Allow control** option, applied on the next start).

More systems, also remote ones, are added in the app under **Systems**. App settings win until
you change an add-on option; a changed option is applied on the next start.

## Language

The app and the entity names are in English or German. With **Language** set to `auto` (the
default) the app follows your Home Assistant profile language and the entity names follow the
Home Assistant system language; anything other than German becomes English. Set `en` or `de`
to fix it. Each device can also pick its own language in the app under ⚙.

## View only vs. control

With control off, the keypad and buttons only show a hint. Nothing is ever sent.
With it on, digits and navigation send immediately; **Arm**, **Disarm** and every function
button ask for confirmation first. The add-on log lists every command that was sent.

## Home Assistant entities (MQTT)

Switch on **Home Assistant entities (MQTT)** (needs the Mosquitto broker add-on). A device per
system appears under *Settings → Devices & services → MQTT* with binary sensors for the LED page
(e.g. "Einbruch Haus"), the function button LEDs, the three indicators and the connection.
Blinking counts as on; the attributes carry `color` and `blink`.

The add-on keeps its own background connection for this that only reads the button/LED page,
so the keypad stays free (checked on a real panel). Best give it a user group of its own that
the panel only allows buttons and LEDs: set **Group for HA** when you add or edit a system.
The HA entities belong to the first system in the list. The status lamps of the keypad page
(operation, intrusion, tamper, ...) are only known while the keypad is open in the app; the
rest of the time they show as unavailable.

Function buttons show up as HA buttons only while **Allow control** and **Function buttons in
Home Assistant** are both on. HA buttons have no confirmation, so think about which automations
may press them.

## AES key

If the panel is set up with an AES key, enter it (hex, as in the original app) when you add or
edit the system. Requests and answers are then encrypted the same way the original app does it;
without a key everything stays plain text. The key is stored in `/data/systems.json` and never
sent back to the browser.

## Troubleshooting

- **"not connected"**: wrong address, port or user group, or the panel is not reachable from
  Home Assistant. Open *⚙ → Test connection to the panel* for the raw panel response.
- **Umlauts look wrong on the LCD**: open *Test connection to the panel* and report the raw text.
