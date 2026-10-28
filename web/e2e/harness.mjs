// Browser end-to-end harness: serves web/ on a free port and drives headless Chromium.
// Needs the wasm build in web/pkg and rules in web/rules (see web/README.md).
// Playwright comes from web/e2e/node_modules (npm ci) or PLAYWRIGHT_MODULE (a path to it).
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

export const WEB = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
export const REPO = path.resolve(WEB, '..');
const TYPES = { '.html': 'text/html', '.js': 'text/javascript', '.mjs': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.wasm': 'application/wasm' };

function serve() {
  const server = http.createServer((req, res) => {
    const rel = decodeURIComponent(new URL(req.url, 'http://x').pathname).replace(/^\/+/, '') || 'index.html';
    const file = path.resolve(WEB, rel);
    if (!file.startsWith(WEB + path.sep) || !fs.existsSync(file) || !fs.statSync(file).isFile()) { res.writeHead(404).end(); return; }
    res.writeHead(200, { 'content-type': TYPES[path.extname(file)] || 'application/octet-stream' });
    fs.createReadStream(file).pipe(res);
  });
  return new Promise(ok => server.listen(0, '127.0.0.1', () => ok(server)));
}

let shared;
/** One server and browser for the whole file; call `close()` in an `after` hook. */
export async function start() {
  if (!fs.existsSync(path.join(WEB, 'pkg', 'bim_wasm.js'))) throw new Error('web/pkg missing: build the wasm first (web/README.md)');
  const mod = process.env.PLAYWRIGHT_MODULE ? pathToFileURL(process.env.PLAYWRIGHT_MODULE).href : 'playwright';
  const pw = await import(mod);
  const chromium = pw.chromium ?? pw.default.chromium;
  const server = await serve();
  const browser = await chromium.launch();
  const base = `http://127.0.0.1:${server.address().port}/`;
  shared = { server, browser, base };
  return shared;
}
export async function close() {
  if (!shared) return;
  await shared.browser.close();
  await new Promise(ok => shared.server.close(ok));
}

/**
 * A fresh page on the app with empty storage, after its first run. `errors` collects page
 * errors (each test should end with none).
 */
export async function openApp({ viewport = { width: 1280, height: 900 } } = {}) {
  const ctx = await shared.browser.newContext({ acceptDownloads: true, locale: 'en-US', viewport });
  const page = await ctx.newPage();
  const errors = [];
  page.on('pageerror', e => errors.push(String(e)));
  await page.goto(shared.base + 'index.html');
  await ready(page);
  await page.evaluate(() => localStorage.clear());
  await page.reload();
  await ready(page);
  return { page, ctx, errors };
}
export const ready = page => page.waitForFunction(() => /ran in/.test(document.getElementById('status').textContent), null, { timeout: 30000 });
export const text = (page, sel) => page.evaluate(s => document.querySelector(s).innerText.replace(/\s+/g, ' ').trim(), sel);

/** Replace the project through the JSON tab, as a user pasting JSON would. */
export async function applyJson(page, edit) {
  await page.evaluate(src => {
    const box = document.getElementById('jsonBox');
    const p = JSON.parse(box.value);
    const out = new Function('p', src)(p) ?? p;
    box.value = JSON.stringify(out); document.getElementById('apply').click();
  }, `(${edit})(p)`);
  await page.waitForFunction(() => !document.getElementById('jsonMsg').textContent);
}

/** Run an export button and return the downloaded bytes. */
export async function download(page, buttonSel) {
  const [d] = await Promise.all([page.waitForEvent('download'), page.click(buttonSel)]);
  return { name: d.suggestedFilename(), bytes: fs.readFileSync(await d.path()) };
}

/** File names in a stored (or deflated) ZIP, from its central directory. */
export function zipNames(buf) {
  const names = [];
  let eocd = buf.lastIndexOf(Buffer.from([0x50, 0x4b, 0x05, 0x06]));
  if (eocd < 0) throw new Error('not a zip');
  let off = buf.readUInt32LE(eocd + 16);
  const n = buf.readUInt16LE(eocd + 10);
  for (let i = 0; i < n; i++) {
    if (buf.readUInt32LE(off) !== 0x02014b50) throw new Error('bad central directory');
    const len = buf.readUInt16LE(off + 28), extra = buf.readUInt16LE(off + 30), comment = buf.readUInt16LE(off + 32);
    names.push(buf.subarray(off + 46, off + 46 + len).toString());
    off += 46 + len + extra + comment;
  }
  return names;
}
