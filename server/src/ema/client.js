// Minimal client for the alarm panel's XML protocol.
//
// Supports two payload modes, selected by whether an AES key is configured:
//   - plaintext (no key on the panel): the B-GR payload is sent raw
//   - AES (key set): AES/ECB/PKCS7 of the payload, hex-encoded
//
// NOTE: the exact B-GR payload layout (counter / session / key-code order) still needs
// to be confirmed against a real traffic capture before command-building is wired up.
// Response parsing uses the known field names and already works.

import crypto from 'node:crypto';
import { XMLParser } from 'fast-xml-parser';

const parser = new XMLParser({ ignoreAttributes: false, parseTagValue: false });

export class EmaClient {
  /**
   * @param {object} cfg
   * @param {string} cfg.host   panel host reachable from this backend
   * @param {number} cfg.port   panel port (e.g. 8016)
   * @param {string} [cfg.aesKey] 32 hex chars to enable AES mode; empty/undefined = plaintext
   */
  constructor({ host, port, aesKey }) {
    this.host = host;
    this.port = port;
    this.aesKey = aesKey || '';
    this.encrypted = this.aesKey.length > 0;
    this.sessionId = null;
    this.counter = 0xffff; // starts at FFFF, increments per request
  }

  get baseUrl() {
    return `http://${this.host}:${this.port}`;
  }

  /** Build the B-GR value for a command payload, honoring the current mode. */
  buildPayload(raw) {
    if (!this.encrypted) return raw; // plaintext mode: send as-is
    const key = Buffer.from(this.aesKey, 'hex');
    const cipher = crypto.createCipheriv('aes-128-ecb', key, null);
    cipher.setAutoPadding(true); // PKCS7
    const input = Buffer.from(raw, 'latin1'); // Cp1250 ~ latin1 for the ASCII range used
    const enc = Buffer.concat([cipher.update(input), cipher.final()]);
    return enc.toString('hex').toUpperCase();
  }

  nextCounter() {
    this.counter = (this.counter + 1) & 0xffff;
    return this.counter.toString(16).toUpperCase().padStart(4, '0');
  }

  /**
   * Fetch one XML page and return it parsed to an object.
   * @param {string} page  e.g. 'ui_io.xml'
   * @param {string} [payload] raw command payload (optional)
   */
  async fetchPage(page, payload = '') {
    const params = new URLSearchParams();
    if (payload) params.set('B-GR', this.buildPayload(payload));
    params.set('CNT', this.nextCounter());
    if (this.sessionId) params.set('S-ID', this.sessionId);

    const url = `${this.baseUrl}/${page}?${params.toString()}`;
    const res = await fetch(url, { headers: { Connection: 'close' } });
    if (!res.ok) throw new Error(`panel responded ${res.status} for ${page}`);
    const xml = await res.text();
    const doc = parser.parse(xml);

    // Remember the session id from ui_init.
    const sid = findField(doc, 'Session_ID');
    if (sid) this.sessionId = String(sid);
    return doc;
  }

  /** Read current display + LED state from ui_io.xml into a normalized shape. */
  async getState() {
    const doc = await this.fetchPage('ui_io.xml');
    return normalizeIo(doc);
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
    // LED tags (RED_ON/OFF/BLINK_0, GREEN_*, YELLOW_*) are read as-is for now;
    // mapping them to named indicators happens once list tag names are confirmed.
    raw: doc,
  };
}
