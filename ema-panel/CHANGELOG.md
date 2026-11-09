# Changelog

## 0.1.0
- First release: live panel state, virtual keypad, function buttons, responsive UI.
- View-only by default; control behind an add-on option with confirmations.

## 0.1.3
- Fix: the panel expects the answered counter plus one in the next request. Sending it unchanged made the panel close every request after the first.

## 0.1.4
- Names of function buttons, indicators and LEDs are read from the panel (they sit in a nested Text element).
- LCD is drawn as 2 x 40 cells with the panel's own custom glyphs (ui_12540_sz) and the cursor position.
- UI is called "Alarm Keypad"; the intro no longer claims read-only when control is enabled.
