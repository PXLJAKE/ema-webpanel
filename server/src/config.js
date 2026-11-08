// Central config, read from environment variables.
// On Plesk these come from the Node.js app's environment; locally from .env
// (Node 20.6+ supports `node --env-file=.env`; or export them in your shell).

// Accept EMA_HOST as either "host" or "host:port" (and tolerate a stray
// "http://" prefix or trailing slash). A port in EMA_HOST wins over EMA_PORT,
// so setting both never produces a doubled ":8016:8016".
function parseHost(raw, fallbackPort) {
  let h = (raw || '').trim().replace(/^https?:\/\//i, '').replace(/\/+$/, '');
  let port = fallbackPort;
  const i = h.lastIndexOf(':');
  if (i > 0) {
    const maybePort = h.slice(i + 1);
    if (/^\d+$/.test(maybePort)) {
      port = Number(maybePort);
      h = h.slice(0, i);
    }
  }
  return { host: h, port };
}

const emaPortEnv = process.env.EMA_PORT ? Number(process.env.EMA_PORT) : 8016;
const ema = parseHost(process.env.EMA_HOST, emaPortEnv);

export const config = {
  port: Number(process.env.PORT || 3000),
  ema: {
    host: ema.host,
    port: ema.port,
    group: process.env.EMA_GROUP || '1',
    aesKey: process.env.EMA_AES_KEY || '', // empty = plaintext mode
  },
  auth: {
    email: process.env.APP_LOGIN_EMAIL || '',
    password: process.env.APP_LOGIN_PASSWORD || '',
    sessionSecret: process.env.SESSION_SECRET || '',
  },
};

export function panelConfigured() {
  return Boolean(config.ema.host);
}
