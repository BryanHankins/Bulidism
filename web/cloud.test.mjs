// node --test web/cloud.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createCloud, CloudError, TOKEN_KEY, resetTokenFromHash } from './cloud.js';

// stub fetch: routes "METHOD /path" -> [status, body]; records every call
function stub(routes) {
  const calls = [];
  const fetch = async (url, init) => {
    calls.push({ url, method: init.method, headers: init.headers, body: init.body === undefined ? undefined : JSON.parse(init.body) });
    const key = `${init.method} ${url}`, r = routes[key];
    if (!r) return { ok: false, status: 404, text: async () => '' };
    const [status, body, hdrs = {}] = typeof r === 'function' ? r(init) : r;
    return { ok: status >= 200 && status < 300, status, headers: { get: k => hdrs[k] ?? null }, text: async () => body === undefined ? '' : JSON.stringify(body) };
  };
  return { fetch, calls };
}
const memStore = () => { const m = new Map(); return { getItem: k => m.has(k) ? m.get(k) : null, setItem: (k, v) => m.set(k, String(v)), removeItem: k => m.delete(k), m }; };

test('login stores the token and sends it as a Bearer header', async () => {
  const { fetch, calls } = stub({
    'POST /api/auth/login': [200, { user: { id: 'u1', email: 'a@b.c' }, token: 'T1' }],
    'GET /api/projects': [200, [{ id: 'p1', name: 'A', role: 'owner', updated_at: '2026-01-01T00:00:00Z' }]],
    'POST /api/auth/logout': [204],
  });
  const storage = memStore(), c = createCloud({ base: '/api', fetch, storage });
  assert.equal(c.signedIn, false);
  assert.deepEqual(await c.login('a@b.c', 'pw'), { id: 'u1', email: 'a@b.c' });
  assert.deepEqual(calls[0].body, { email: 'a@b.c', password: 'pw' });
  assert.equal(calls[0].headers.Authorization, undefined);
  assert.equal(storage.m.get(TOKEN_KEY), 'T1');
  assert.equal((await c.list())[0].id, 'p1');
  assert.equal(calls[1].headers.Authorization, 'Bearer T1');
  // a new client picks the token up from storage
  assert.equal(createCloud({ base: '/api', fetch, storage }).signedIn, true);
  await c.logout();
  assert.equal(c.signedIn, false); assert.equal(storage.m.has(TOKEN_KEY), false);
});

test('project CRUD and share paths, ids and emails encoded', async () => {
  const { fetch, calls } = stub({
    'POST https://x.test/api/auth/register': [200, { user: { id: 'u', email: 'e' }, token: 'T' }],
    'POST https://x.test/api/projects': [200, { id: 'p 1', name: 'N', role: 'owner', updated_at: 't0' }],
    'GET https://x.test/api/projects/p%201': [200, { id: 'p 1', name: 'N', role: 'owner', updated_at: 't0', data: { a: 1 } }],
    'PUT https://x.test/api/projects/p%201': [200, { id: 'p 1', name: 'N', role: 'owner', updated_at: 't1' }],
    'DELETE https://x.test/api/projects/p%201': [204],
    'GET https://x.test/api/projects/p%201/shares': [200, [{ email: 'b@c.d', role: 'viewer' }]],
    'PUT https://x.test/api/projects/p%201/shares': [200, { email: 'b@c.d', role: 'editor' }],
    'DELETE https://x.test/api/projects/p%201/shares/b%40c.d': [204],
  });
  const c = createCloud({ base: 'https://x.test/api/', fetch, storage: null });
  await c.register('e', 'pw');
  assert.equal((await c.create('N', { a: 1 })).id, 'p 1');
  assert.deepEqual(calls[1].body, { name: 'N', data: { a: 1 } });
  assert.deepEqual((await c.get('p 1')).data, { a: 1 });
  assert.equal((await c.save('p 1', { data: { a: 2 }, updated_at: 't0' })).updated_at, 't1');
  assert.deepEqual(calls[3].body, { data: { a: 2 }, updated_at: 't0' });
  assert.equal(calls[3].headers['Content-Type'], 'application/json');
  assert.equal((await c.shares('p 1'))[0].role, 'viewer');
  await c.share('p 1', 'b@c.d', 'editor'); assert.deepEqual(calls[5].body, { email: 'b@c.d', role: 'editor' });
  assert.equal(await c.unshare('p 1', 'b@c.d'), null);
  assert.equal(await c.remove('p 1'), null);
  assert.equal(c.signedIn, true, 'storage: null keeps the token in memory');
});

test('409 on a stale save is a conflict error; 401 drops the token', async () => {
  const { fetch } = stub({
    'PUT /api/projects/p': [409, { error: 'project changed', updated_at: 't9' }],
    'GET /api/me': [401, { error: 'unauthorized' }],
  });
  const storage = memStore(); storage.setItem(TOKEN_KEY, 'OLD');
  const c = createCloud({ fetch, storage, base: '/api' });
  await assert.rejects(c.save('p', { data: {}, updated_at: 't0' }), e => e instanceof CloudError && e.conflict && e.status === 409 && e.message === 'project changed' && e.body.updated_at === 't9');
  await assert.rejects(c.me(), e => e.status === 401 && !e.conflict);
  assert.equal(c.signedIn, false); assert.equal(storage.getItem(TOKEN_KEY), null);
});

test('health is false when the API is absent or unreachable; storage errors are tolerated', async () => {
  assert.equal(await createCloud({ base: '/api', fetch: stub({}).fetch, storage: null }).health(), false);
  assert.equal(await createCloud({ base: '/api', fetch: async () => { throw new TypeError('network'); }, storage: null }).health(), false);
  assert.equal(await createCloud({ base: '/api', fetch: stub({ 'GET /api/health': [200, { ok: true }] }).fetch, storage: null }).health(), true);
  const bad = { getItem() { throw new Error('denied'); }, setItem() { throw new Error('denied'); }, removeItem() { throw new Error('denied'); } };
  const c = createCloud({ base: '/api', fetch: stub({ 'POST /api/auth/login': [200, { user: {}, token: 'T' }] }).fetch, storage: bad });
  await c.login('a', 'b'); assert.equal(c.signedIn, true);
  // non-JSON error bodies surface as the message
  const t = createCloud({ base: '/api', storage: null, fetch: async () => ({ ok: false, status: 500, text: async () => 'boom' }) });
  await assert.rejects(t.list(), e => e.status === 500 && e.message === 'boom');
});

test('password reset: request and confirm paths; confirm drops any local token', async () => {
  const { fetch, calls } = stub({
    'POST /api/auth/login': [200, { user: { id: 'u1', email: 'a@b.c' }, token: 'T1' }],
    'POST /api/auth/password-reset': [202],
    'POST /api/auth/password-reset/confirm': [204],
  });
  const storage = memStore(), c = createCloud({ base: '/api', fetch, storage });
  await c.login('a@b.c', 'pw');
  await c.requestReset('a@b.c');
  assert.deepEqual(calls[1].body, { email: 'a@b.c' });
  await c.confirmReset('ab'.repeat(32), 'new password');
  assert.deepEqual(calls[2].body, { token: 'ab'.repeat(32), password: 'new password' });
  assert.equal(c.signedIn, false);
  assert.equal(storage.getItem(TOKEN_KEY), null);
});

test('429 carries Retry-After seconds; 503 and 400 surface the server message', async () => {
  const { fetch } = stub({
    'POST /api/auth/login': [429, { error: 'too many attempts; try again later' }, { 'Retry-After': '120' }],
    'POST /api/auth/password-reset': [503, { error: 'password reset is not configured on this server' }],
    'POST /api/auth/password-reset/confirm': [400, { error: 'this reset link is invalid or has expired' }],
  });
  const c = createCloud({ base: '/api', fetch, storage: null });
  await assert.rejects(c.login('a@b.c', 'pw'), e => e instanceof CloudError && e.rateLimited && e.retryAfter === 120);
  await assert.rejects(c.requestReset('a@b.c'), e => e.status === 503 && /not configured/.test(e.message));
  await assert.rejects(c.confirmReset('ab'.repeat(32), 'new password'), e => e.status === 400 && !e.rateLimited && e.retryAfter === null);
});

test('reset token is read only from a well-formed #reset= hash', () => {
  const t = '0123456789abcdef'.repeat(4);
  assert.equal(resetTokenFromHash('#reset=' + t), t);
  for (const bad of ['', '#', '#reset=', '#reset=xyz', '#reset=' + t + '&x=1', '#other=' + t, '#reset=<script>']) assert.equal(resetTokenFromHash(bad), null, bad);
});
