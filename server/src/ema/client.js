// Read-only client for the alarm panel's XML protocol.
//
// SAFETY: this client only ever issues HTTP GETs that READ state
// (ui_init / ui_io / ui_12540). It never sends a key-press or command
// payload. Writing commands is deliberately not implemented here yet; it
// will be added behind the server-side read-only guard, not in this file.
//
// Protocol (resolved from the original app):
//   GET http://<host>:<port>/<page>?B-GR=<group>&S-ID=<session>&CNT=<counter>
//   - B-GR is the Benutzergruppe (user group), NOT an encrypted command.
//   - First call ui_init.xml with S-ID empty and CNT=FFFF; the response carries
//     Session_ID and Counter. Each following request uses S-ID=<session> and
//     CNT=<last response Counter>.
//   - Plaintext mode (no key on the panel): the URL is sent as-is, no crypto.
//   - AES mode (key set): only the "S-ID=…&CNT=…" substring is AES/ECB/PKCS7
//     (Cp1250, hex). Deferred — this panel has no key.
//
// Responses are ISO-8859-1 (Latin-1), so bytes are decoded as latin1 before XML parsing.

import { XMLParser } from 'fast-xml-parser';

const parser = new XMLParser({ ignoreAttributes: false, parseTagValue: false });

export class EmaClient {
  /**
   * @param {object} cfg
   * @param {string} cfg.host    panel host reachable from this backend
   * @param {number} cfg.port    panel port (e.g. 8016)
   * @param {string} [cfg.group] Benutzergruppe sent as B-GR (default '1')
   * @param {string} [cfg.aesKey] 32 hex chars to enable AES mode; empty = plaintext
   * @param {number} [cfg.timeoutMs] per-request timeout (default 8000)
   */
  constructor({ host, port, group, aesKey, timeoutMs }) {
    this.host = host;
    this.port = port;
    this.group = (group ?? '1').toString();
    this.aesKey = aesKey || '';
    this.encrypted = this.aesKey.length > 0;
    this.timeoutMs = timeoutMs || 8000;
    this.sessionId = '';
    this.counter = 'FFFF'; // first request uses FFFF
  }

  get baseUrl() {
    return `http://${this.host}:${this.port}`;
  }

  /** Fetch one XML page (read-only) and return { url, status, xml, doc }. */
  async fetchPage(page) {
    const params = new URLSearchParams();
    params.set('B-GR', this.group);
    params.set('S-ID', this.sessionId);
    params.set('CNT', this.counter);

    // AES mode would encrypt only the S-ID/CNT substring here; plaintext sends raw.
    // (Not reached on a keyless panel.)
    const url = `${this.baseUrl}/${page}?${params.toString()}`;

    const ctrl = new AbortController();
    const t = setTimeout(() => ctrl.abort(), this.timeoutMs);
    let res;
    try {
      res = await fetch(url, { headers: { Connection: 'close' }, signal: ctrl.signal });
    } finally {
      clearTimeout(t);
    }
    // Decode as Latin-1 (panel serves ISO-8859-1).
    const buf = Buffer.from(await res.arrayBuffer());
    const xml = buf.toString('latin1');
    let doc = null;
    try {
      doc = parser.parse(xml);
    } catch {
      /* leave doc null; caller still gets raw xml */
    }

    // Carry the session id and counter forward for the next request.
    const sid = doc && findField(doc, 'Session_ID');
    if (sid) this.sessionId = String(sid);
    const cnt = doc && findField(doc, 'Counter');
    if (cnt) this.counter = String(cnt);

    return { url, status: res.status, xml, doc };
  }

  /** Initialize a session (ui_init.xml). Returns the raw fetch result. */
  async init() {
    this.sessionId = '';
    this.counter = 'FFFF';
    return this.fetchPage('ui_init.xml');
  }

  /** Read current display + LED state from ui_io.xml. */
  async getState() {
    if (!this.sessionId) await this.init();
    const io = await this.fetchPage('ui_io.xml');
    return normalizeIo(io.doc || {});
  }

  /**
   * Read-only connection test: init, then read ui_io and ui_12540, returning
   * status, the parsed LCD/LED snapshot, and the raw XML of each page so the
   * real field names can be inspected.
   */
  async connectTest() {
    const pages = [];
    const initRes = await this.init();
    pages.push({ page: 'ui_init.xml', status: initRes.status, xml: initRes.xml });

    const sessionOk = Boolean(this.sessionId);
    let state = null;
    if (sessionOk) {
      const io = await this.fetchPage('ui_io.xml');
      pages.push({ page: 'ui_io.xml', status: io.status, xml: io.xml });
      state = normalizeIo(io.doc || {});

      const kp = await this.fetchPage('ui_12540.xml');
      pages.push({ page: 'ui_12540.xml', status: kp.status, xml: kp.xml });
    }

    return {
      ok: sessionOk,
      host: `${this.host}:${this.port}`,
      mode: this.encrypted ? 'aes' : 'plaintext',
      sessionId: this.sessionId || null,
      counter: this.counter,
      state,
      pages,
    };
  }
}

// --- helpers ---------------------------------------------------------------

function findField(obj, name) {
  if (obj == null || typeof obj !== 'object') return undefined;
  if (name in obj) return obj[name];
  for (const v of Object.values(obj)) {
    const hit = findField(v, name);
    if (hit !== undefined) return hit;
  }
  return undefined;
}

function normalizeIo(doc) {
  return {
    line1: findField(doc, 'Text_80_01') ?? '',
    line2: findField(doc, 'Text_80_02') ?? '',
    revision: findField(doc, 'Revision') ?? null,
    cursorPos: findField(doc, 'Cursor_Pos') ?? null,
    cursorBlink: findField(doc, 'Cursor_Blink') ?? null,
  };
}
