// ema-webpanel backend
//
// Serves the PWA (../web) and a small JSON API that talks to the alarm panel.
// Secrets (panel host, optional AES key, login) come from environment variables
// (see server/.env.example). Nothing secret is ever sent to the browser.

import express from 'express';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { config, panelConfigured } from './config.js';
import { EmaClient } from './ema/client.js';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const webDir = path.resolve(__dirname, '../../web');

const app = express();
app.use(express.json());

// --- static PWA ---
app.use(express.static(webDir));

// --- health / status ---
app.get('/api/health', (_req, res) => {
  res.json({
    ok: true,
    panelConfigured: panelConfigured(),
    mode: config.ema.aesKey ? 'aes' : 'plaintext',
  });
});

// --- panel state (polling target for the PWA) ---
// TODO: protect behind the app login once auth is implemented.
app.get('/api/state', async (_req, res) => {
  if (!panelConfigured()) {
    return res.status(503).json({ error: 'panel not configured', demo: true });
  }
  try {
    const client = getClient();
    const state = await client.getState();
    res.json(state);
  } catch (err) {
    res.status(502).json({ error: String(err.message || err) });
  }
});

// --- auth (stub) ---
// TODO: implement email+password login -> signed session token; gate /api/* on it.
app.post('/api/login', (req, res) => {
  const { email, password } = req.body || {};
  if (!config.auth.email) {
    return res.status(501).json({ error: 'login not configured yet' });
  }
  const ok = email === config.auth.email && password === config.auth.password;
  if (!ok) return res.status(401).json({ error: 'invalid credentials' });
  // TODO: issue a signed session token here.
  res.json({ ok: true });
});

// --- key press (stub) ---
// TODO: wire up once the B-GR payload layout is confirmed.
app.post('/api/key', (_req, res) => {
  res.status(501).json({ error: 'command sending not implemented yet' });
});

// one shared client for now; later: one per configured panel / session
let _client = null;
function getClient() {
  if (!_client) {
    _client = new EmaClient({
      host: config.ema.host,
      port: config.ema.port,
      aesKey: config.ema.aesKey,
    });
  }
  return _client;
}

app.listen(config.port, () => {
  console.log(`ema-webpanel server on http://localhost:${config.port}`);
  console.log(`  PWA served from ${webDir}`);
  console.log(`  panel: ${panelConfigured() ? config.ema.host + ':' + config.ema.port : '(not configured — demo mode)'}`);
  console.log(`  mode: ${config.ema.aesKey ? 'AES' : 'plaintext'}`);
});
