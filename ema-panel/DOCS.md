# Alarm Keypad (EMA)

Modern web keypad for a compatible intrusion alarm panel, shown in the Home Assistant sidebar.

**Unofficial. No affiliation, no warranty, use at your own risk.**

## Setup

1. Set **Panel address** (IP of the transmitter unit in your network) and, if needed, the
   **User group** (the same number as in the original app).
2. Start the add-on and open **Alarmanlage** in the sidebar.
3. Check that the LCD shows the panel text and the header says **live**.
4. Only when everything looks right: turn on **Allow control** and restart the add-on.

## View only vs. control

With **Allow control** off, the keypad and buttons only show a hint. Nothing is ever sent.
With it on, digits and navigation send immediately; **EIN**, **AUS** and every function button
ask for confirmation first. The add-on log lists every command that was sent.

## Troubleshooting

- **"nicht verbunden"**: wrong address/port/user group, or the panel is not reachable from
  Home Assistant. Open *Settings → ⚙ → Verbindung testen* for the raw panel response.
- **Umlauts look wrong on the LCD**: open *Verbindung testen* and report the raw text.
