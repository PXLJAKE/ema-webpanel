# Alarm Keypad (EMA)

Modern web keypad for a compatible intrusion alarm panel, shown in the Home Assistant sidebar.

**Unofficial. No affiliation, no warranty, use at your own risk.**

## Setup

1. Set **Panel address** (IP of the transmitter unit in your network) and, if needed, the
   **User group** (the same number as in the original app).
2. Start the add-on and open **Alarmanlage** in the sidebar.
3. Check that the LCD shows the panel text and the header says **live**.
4. Only when everything looks right: turn on **Steuern erlauben** in the app under ⚙
   (or the **Allow control** option, applied on the next start).

More systems, also remote ones, are added in the app under **Anlagen**. App settings win until
you change an add-on option; a changed option is applied on the next start.

## View only vs. control

With control off, the keypad and buttons only show a hint. Nothing is ever sent.
With it on, digits and navigation send immediately; **EIN**, **AUS** and every function button
ask for confirmation first. The add-on log lists every command that was sent.

## Home Assistant entities (MQTT)

Turn on **mqtt_entities** (needs the Mosquitto broker add-on). A device per system appears under
*Settings → Devices → MQTT* with binary sensors for the LED page (e.g. "Einbruch Haus"), the
Taster LEDs, the three indicators and the connection. Blinking counts as on; the attributes
carry `color` and `blink`.

The add-on keeps its own background connection for this that only reads the button/LED page,
so the keypad stays free (checked on a real panel). Best give it a user group of its own that
the panel only allows buttons and LEDs: set **Gruppe für HA** when you add or edit a system.
The HA entities belong to the first system in the list. The status lamps of the keypad page
(Betrieb, Einbruch, Sabotage, ...) are only known while the keypad is open in the app; the rest
of the time they show as unavailable.

## AES key

If the panel is set up with an AES key, enter it (hex, as in the original app) when you add or
edit the system. Requests and answers are then encrypted like the original app does it; without
a key everything stays plain text. The key is stored in `/data/systems.json` and never sent
back to the browser.

Taster show up as HA buttons only while **Steuern erlauben** and **mqtt_buttons** are both on.
HA buttons have no confirmation, so think about which automations may press them.

## Troubleshooting

- **"nicht verbunden"**: wrong address/port/user group, or the panel is not reachable from
  Home Assistant. Open *Settings → ⚙ → Verbindung testen* for the raw panel response.
- **Umlauts look wrong on the LCD**: open *Verbindung testen* and report the raw text.
