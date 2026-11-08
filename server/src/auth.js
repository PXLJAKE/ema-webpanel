// Tiny stateless session auth: a signed cookie, no DB, no new deps.
//
// The token is `<payload>.<sig>` where payload is base64url JSON {sub, exp}
// and sig is an HMAC-SHA256 over the payload using SESSION_SECRET. Nothing
// secret lives in the browser — only the signed token, which the server
// re-verifies on every request.

import crypto from 'node:crypto';

const COOKIE = 'mb_session';

function b64url(buf) {
  return Buffer.from(buf).toString('base64url');
}

function sign(payloadB64, secret) {
  return crypto.createHmac('sha256', secret).update(payloadB64).digest('base64url');
}

/**
 * Create a signed session token.
 * @param {string} sub      subject (the logged-in email)
 * @param {string} secret   SESSION_SECRET
 * @param {number} maxAgeSec lifetime in seconds
 */
export function issueToken(sub, secret, maxAgeSec) {
  const payload = { sub, exp: Math.floor(Date.now() / 1000) + maxAgeSec };
  const payloadB64 = b64url(JSON.stringify(payload));
  return `${payloadB64}.${sign(payloadB64, secret)}`;
}

/** Verify a token; returns the payload object or null. */
export function verifyToken(token, secret) {
  if (!token || typeof token !== 'string') return null;
  const dot = token.indexOf('.');
  if (dot < 1) return null;
  const payloadB64 = token.slice(0, dot);
  const sig = token.slice(dot + 1);
  const expected = sign(payloadB64, secret);
  // constant-time compare
  const a = Buffer.from(sig);
  const b = Buffer.from(expected);
  if (a.length !== b.length || !crypto.timingSafeEqual(a, b)) return null;
  let payload;
  try {
    payload = JSON.parse(Buffer.from(payloadB64, 'base64url').toString('utf8'));
  } catch {
    return null;
  }
  if (!payload || typeof payload.exp !== 'number') return null;
  if (payload.exp < Math.floor(Date.now() / 1000)) return null;
  return payload;
}

/** Parse the session token out of a Cookie header. */
export function readCookie(req) {
  const header = req.headers.cookie || '';
  for (const part of header.split(';')) {
    const i = part.indexOf('=');
    if (i < 0) continue;
    if (part.slice(0, i).trim() === COOKIE) {
      return decodeURIComponent(part.slice(i + 1).trim());
    }
  }
  return null;
}

/** Build a Set-Cookie value for the session token (or a clearing one). */
export function cookieHeader(token, maxAgeSec, secure) {
  const attrs = [
    `${COOKIE}=${token ? encodeURIComponent(token) : ''}`,
    'Path=/',
    'HttpOnly',
    'SameSite=Lax',
  ];
  if (secure) attrs.push('Secure');
  if (token) attrs.push(`Max-Age=${maxAgeSec}`);
  else attrs.push('Max-Age=0');
  return attrs.join('; ');
}

/** Express middleware factory: require a valid session, else 401. */
export function requireAuth(secret) {
  return (req, res, next) => {
    const payload = verifyToken(readCookie(req), secret);
    if (!payload) return res.status(401).json({ error: 'not authenticated' });
    req.user = payload.sub;
    next();
  };
}

export { COOKIE };
