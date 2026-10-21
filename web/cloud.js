// Cloud projects client: a thin fetch wrapper over the buildsim API (JSON, Bearer token).
// API root: window.BUILDSIM_API (e.g. "https://api.example.com/api") or same-origin "/api".
// No UI here; app.js hides the Cloud tab when health() fails, so static deployments are unaffected.

export const TOKEN_KEY = 'buildsim.cloud.token';

export class CloudError extends Error {
  constructor(status, body) {
    super((body && (body.error || body.message)) || `HTTP ${status}`);
    this.status = status; this.body = body;
  }
  get conflict() { return this.status === 409; }
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
    if (!r.ok) { if (r.status === 401) setToken(null); throw new CloudError(r.status, data); }
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
