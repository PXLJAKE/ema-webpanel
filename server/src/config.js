// Central config, read from environment variables.
// On Plesk these come from the Node.js app's environment; locally from .env
// (Node 20.6+ supports `node --env-file=.env`; or export them in your shell).

export const config = {
  port: Number(process.env.PORT || 3000),
  ema: {
    host: process.env.EMA_HOST || '',
    port: Number(process.env.EMA_PORT || 8016),
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
