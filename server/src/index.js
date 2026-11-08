// ema-webpanel backend
//
// Serves the PWA (../public) and a small JSON API that talks to the alarm panel.
// Secrets (panel host, optional AES key, login) come from environment variables
// (see server/.env.example). Nothing secret is ever sent to the browser.
//
// SAFETY: every panel call in this service is READ-ONLY. There is no route that
// arms/disarms the panel or sends a key-press; commands are not implemented.

import express from 'express';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { config, panelConfigured } from './config.js';
import { EmaClient } from './ema/client.js';
import { issueToken, cookieHeader, requireAuth, readCookie, verifyToken } from './auth.js';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
// server/public also serves as the webserver Document Root on Plesk; the webserver
// serves static files from here directly and passes other routes (/api/*) to Node.
const webDir = path.resolve(__dirname, '../public');

// Session lifetimes: normal vs. "Angemeldet bleiben" (remember me).
const SESSION_MAXAGE = 12 * 60 * 60; // 12 hours
const REMEMBER_MAXAGE = 90 * 24 * 60 * 60; // 90 days

const app = express();
app.set('trust proxy', 1); // behind Plesk/Passenger reverse proxy
app.use(express.json());

// --- static PWA ---
app.use(express.static(webDir));

// --- health / status (public) ---
app.get('/api/health', (_req, res) => {
  res.json({
    ok: true,
    panelConfigured: panelConfigured(),
    authConfigured: Boolean(config.auth.email && config.auth.sessionSecret),
    mode: config.ema.aesKey ? 'aes' : 'plaintext',
  });
});

// --- auth ---
function requestIsSecure(req) {
  return req.secure || req.headers['x-forwarded-proto'] === 'https';
}

app.post('/api/login', (req, res) => {
  const { email, password, remember } = req.body || {};
  if (!config.auth.email || !config.auth.sessionSecret) {
    return res.status(501).json({ error: 'login not configured' });
  }
  const ok = email === config.auth.email && password === config.auth.password;
  if (!ok) return res.status(401).json({ error: 'invalid credentials' });

  const maxAge = remember ? REMEMBER_MAXAGE : SESSION_MAXAGE;
  const token = issueToken(email, config.auth.sessionSecret, maxAge);
  res.setHeader('Set-Cookie', cookieHeader(token, maxAge, requestIsSecure(req)));
  res.json({ ok: true, email, remember: Boolean(remember) });
});

app.post('/api/logout', (req, res) => {
  res.setHeader('Set-Cookie', cookieHeader('', 0, requestIsSecure(req)));
  res.json({ ok: true });
});

app.get('/api/session', (req, res) => {
  const payload = verifyToken(readCookie(req), config.auth.sessionSecret || '');
  if (!payload) return res.json({ authenticated: false });
  res.json({ authenticated: true, email: payload.sub, exp: payload.exp });
});

// --- panel state (polling target, requires login) ---
app.get('/api/state', auth(), async (_req, res) => {
  if (!panelConfigured()) {
    return res.status(503).json({ error: 'panel not configured', demo: true });
  }
  try {
    const state = await getClient().getState();
    res.json(state);
  } catch (err) {
    res.status(502).json({ error: String(err.message || err) });
  }
});

// --- read-only connection test (requires login) ---
// Hits the panel, returns status + parsed LCD snapshot + raw XML of each page.
app.get('/api/connect-test', auth(), async (_req, res) => {
  if (!panelConfigured()) {
    return res.status(503).json({ error: 'panel not configured' });
  }
  try {
    // fresh client each time so the test always starts a new session
    const client = new EmaClient({
      host: config.ema.host,
      port: config.ema.port,
      group: config.ema.group,
      aesKey: config.ema.aesKey,
    });
    const result = await client.connectTest();
    res.json(result);
  } catch (err) {
    res.status(502).json({ ok: false, error: String(err.message || err) });
  }
});

// --- key press: intentionally not implemented (read-only service) ---
app.post('/api/key', (_req, res) => {
  res.status(501).json({ error: 'command sending not implemented (read-only)' });
});

// auth middleware bound to the configured secret
function auth() {
  return requireAuth(config.auth.sessionSecret || '');
}

// one shared client for polling; the connect-test makes its own.
let _client = null;
function getClient() {
  if (!_client) {
    _client = new EmaClient({
      host: config.ema.host,
      port: config.ema.port,
      group: config.ema.group,
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
  console.log(`  auth: ${config.auth.email && config.auth.sessionSecret ? 'configured' : 'NOT configured'}`);
});
