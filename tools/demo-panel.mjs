// Demo panel: answers like a real transmitter unit with made-up data, so the app can be tried
// (and screenshots taken) without a real alarm system. Commands are only logged.
//   node tools/demo-panel.mjs [port]      then add a system with address 127.0.0.1 and that port
import http from 'node:http';

const port = Number(process.argv[2] || 8016);
let counter = 0x1000;
const next = () => (counter = (counter + 1) % 0xffff).toString(16).toUpperCase().padStart(4, '0');

// LCD codes: ASCII, umlauts at 0xE1 (ä) 0xEF (ö) 0xF5 (ü)
const enc = s => [...s.padEnd(40).slice(0, 40)].map(c => ({ 'ä': 0xe1, 'ö': 0xef, 'ü': 0xf5 }[c] ?? c.charCodeAt(0)).toString(16).toUpperCase().padStart(2, '0')).join(',');
const now = () => { const d = new Date(); const p = n => String(n).padStart(2, '0'); return `${p(d.getDate())}.${p(d.getMonth() + 1)}.${String(d.getFullYear()).slice(2)} ${p(d.getHours())}:${p(d.getMinutes())}`; };

const buttons = ['Garagentor', 'Hoftor öffnen', 'Außenlicht', 'Alarme löschen', 'Heizung EG', 'Heizung OG', 'Gartenpumpe', ''];
const indicators = ['Feuer', 'Wasseralarm', 'Heizung Störung'];
const leds = ['Haustür', 'Terrassentür', 'Garage', 'Fenster EG', 'Fenster OG', 'Keller', 'Netz', 'Akku'];
const btnState = ['GREEN_OFF', 'GREEN_OFF', 'GREEN_ON', 'GREEN_OFF', 'GREEN_ON', 'GREEN_OFF', 'GREEN_OFF', 'GREEN_OFF'];
const indState = ['RED_OFF', 'RED_OFF', 'YELLOW_OFF'];
const ledState = ['GREEN_ON', 'GREEN_ON', 'GREEN_ON', 'YELLOW_ON', 'GREEN_ON', 'GREEN_ON', 'GREEN_ON', 'GREEN_ON'];

const xml = body => `<?xml version="1.0" encoding="ISO-8859-1"?>\n${body}`;
const pages = {
  'ui_init.xml': () => xml(`<ui_init><Init><Counter>${next()}</Counter><Session_ID>01</Session_ID></Init>
<Button_Page><Page_12540>On</Page_12540><Page_Button>On</Page_Button><Page_LED>On</Page_LED>
${buttons.map((b, i) => `<Button nr="${i + 1}"><Text>${b}</Text></Button>`).join('')}
${indicators.map((b, i) => `<LED nr="${i + 1}"><Text>${b}</Text></LED>`).join('')}</Button_Page>
<LED_Page>${leds.map((b, i) => `<LED nr="${i + 1}"><Text>${b}</Text></LED>`).join('')}</LED_Page></ui_init>`),
  'ui_12540_sz.xml': () => xml(`<ui_12540_sz><Counter>${next()}</Counter>${'<SZeichen><Code>00,00,00,00,00,00,00,00</Code></SZeichen>'.repeat(16)}</ui_12540_sz>`),
  'ui_io.xml': () => xml(`<ui_io><Counter>${next()}</Counter><Button_Page>${btnState.map((v, i) => `<LED_Button nr="${i + 1}">${v}</LED_Button>`).join('')}${indState.map((v, i) => `<LED nr="${i + 1}">${v}</LED>`).join('')}</Button_Page>
<LED_Page>${ledState.map((v, i) => `<LED nr="${i + 1}">${v}</LED>`).join('')}</LED_Page></ui_io>`),
  'ui_12540.xml': () => xml(`<ui_12540><Locked>NO</Locked><Counter>${next()}</Counter><LED>
<Betrieb><LED>GREEN_ON</LED></Betrieb><S_Alarm><LED>RED_OFF</LED></S_Alarm><S_Stoerung><LED>YELLOW_OFF</LED></S_Stoerung><Sys_Stoerung><LED>YELLOW_OFF</LED></Sys_Stoerung>
<Einbruch><LED>RED_OFF</LED></Einbruch><Sabotage><LED>RED_OFF</LED></Sabotage><Ueberfall><LED>RED_OFF</LED></Ueberfall><Pol_Not><LED>RED_OFF</LED></Pol_Not></LED>
<Display><Text_80_01>${enc(' Demo-Anlage       Bereich 1 unscharf')}</Text_80_01><Text_80_02>${enc(' Fenster EG offen       ' + now())}</Text_80_02><Cursor_Pos>0</Cursor_Pos><Cursor_Blink>0</Cursor_Blink></Display></ui_12540>`),
};

http.createServer((req, res) => {
  const u = new URL(req.url, 'http://x');
  const page = u.pathname.slice(1);
  const cmd = [...u.searchParams.keys()].find(k => /^T[0-9A-F]+$/.test(k));
  if (cmd) console.log(`demo: command ${cmd} on ${page} (ignored)`);
  const f = pages[page];
  res.writeHead(f ? 200 : 404, { 'content-type': 'text/xml; charset=ISO-8859-1', connection: 'close' });
  res.end(f ? Buffer.from(f(), 'latin1') : 'not found');
}).listen(port, () => console.log(`demo panel on :${port}`));
