// Cloud projects client: a thin fetch wrapper over the buildsim API (JSON, Bearer token).
// API root: window.BUILDSIM_API (e.g. "https://api.example.com/api") or same-origin "/api".
// No UI here; app.js hides the Cloud tab when health() fails, so static deployments are unaffected.

export const TOKEN_KEY = 'buildsim.cloud.token';

export class CloudError extends Error {
  constructor(status, body, retryAfter = null) {
    super((body && (body.error || body.message)) || `HTTP ${status}`);
    this.status = status; this.body = body; this.retryAfter = retryAfter; // seconds, on 429
  }
  get conflict() { return this.status === 409; }
  get rateLimited() { return this.status === 429; }
}

/** An emailed link in the URL hash: `#reset=<token>` or `#verify=<token>` -> {kind, token}, else null. */
export function linkFromHash(hash) {
  const m = /^#(reset|verify)=([0-9a-f]{16,128})$/i.exec(String(hash || '').trim());
  return m ? { kind: m[1].toLowerCase(), token: m[2] } : null;
}
/** The token from an emailed reset link (`#reset=<token>`), or null. */
export function resetTokenFromHash(hash) {
  const l = linkFromHash(hash);
  return l && l.kind === 'reset' ? l.token : null;
}

// localStorage can be missing or throw (private mode, blocked storage): never let that break the app
function defaultStorage() {
  try { return globalThis.localStorage || null; } catch (_) { return null; }
}

export function createCloud({ base, fetch: f, storage } = {}) {
  const root = String(base ?? (globalThis.window && window.BUILDSIM_API) ?? '/api').replace(/\/+$/, '');
  const doFetch = f || ((...a) => globalThis.fetch(...a));
  const st = storage === undefined ? defaultStorage() : storage;
  const load = () => { try { return st ? st.getItem(TOKEN_KEY) : null; } catch (_) { return null; } };
  let token = load();
  const setToken = t => { token = t || null; try { if (st) { if (token) st.setItem(TOKEN_KEY, token); else st.removeItem(TOKEN_KEY); } } catch (_) { /* memory only */ } };
  const enc = encodeURIComponent;

  async function req(method, path, body) {
    const headers = { Accept: 'application/json' };
    if (body !== undefined) headers['Content-Type'] = 'application/json';
    if (token) headers.Authorization = `Bearer ${token}`;
    const r = await doFetch(root + path, { method, headers, body: body === undefined ? undefined : JSON.stringify(body) });
    const text = await r.text();
    let data = null; if (text) { try { data = JSON.parse(text); } catch (_) { data = { error: text }; } }
    if (!r.ok) {
      if (r.status === 401) setToken(null);
      const ra = r.headers && typeof r.headers.get === 'function' ? parseInt(r.headers.get('Retry-After'), 10) : NaN;
      throw new CloudError(r.status, data, Number.isFinite(ra) ? ra : null);
    }
    return data;
  }
  async function auth(path, email, password) {
    const r = await req('POST', path, { email, password });
    setToken(r && r.token); return r.user;
  }

  return {
    get signedIn() { return !!token; },
    get base() { return root; },
    /** true when the API answers; any failure (404 on a static host, network, CORS) is false */
    async health() { try { await req('GET', '/health'); return true; } catch (_) { return false; } },
    register: (email, password) => auth('/auth/register', email, password),
    login: (email, password) => auth('/auth/login', email, password),
    /** Emails a reset link if the account exists (same answer either way); 503 when the server has no email set up */
    requestReset: email => req('POST', '/auth/password-reset', { email }),
    /** Sets a new password from a reset-link token; signs the account out everywhere */
    async confirmReset(token, password) { await req('POST', '/auth/password-reset/confirm', { token, password }); setToken(null); },
    /** Confirms the address from an emailed `#verify=` link; works signed in or not */
    verifyEmail: token => req('POST', '/auth/verify', { token }),
    /** Mails a fresh verification link (202), or 204 when already verified; 503 without email */
    resendVerification: () => req('POST', '/auth/verify/resend'),
    /** Everything the server stores about this account, as an object (see GET /api/me/export) */
    exportData: () => req('GET', '/me/export'),
    /** Deletes the account and every project it owns; the password is checked again (400 if wrong) */
    async deleteAccount(password) { await req('DELETE', '/me', { password }); setToken(null); },
    async logout() { try { await req('POST', '/auth/logout'); } catch (_) { /* token dropped regardless */ } setToken(null); },
    me: () => req('GET', '/me'),
    list: () => req('GET', '/projects'),
    create: (name, data) => req('POST', '/projects', { name, data }),
    get: id => req('GET', `/projects/${enc(id)}`),
    /** PUT name/data; pass updated_at to fail with 409 (err.conflict) if the project changed remotely */
    save: (id, { name, data, updated_at } = {}) => req('PUT', `/projects/${enc(id)}`, { name, data, updated_at }),
    remove: id => req('DELETE', `/projects/${enc(id)}`),
    shares: id => req('GET', `/projects/${enc(id)}/shares`),
    share: (id, email, role) => req('PUT', `/projects/${enc(id)}/shares`, { email, role }),
    unshare: (id, email) => req('DELETE', `/projects/${enc(id)}/shares/${enc(email)}`),
  };
}
