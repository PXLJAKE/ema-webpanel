# Changelog

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
