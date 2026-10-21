// node --test web/cites.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import './cites.js';
const { citeUrl, citeHtml, ibcChapter } = globalThis.BuildsimCites;
const BOOK = 'https://codes.iccsafe.org/content/IBC2024P1';

test('IBC 2024 sections link to their chapter when the slug is known', () => {
  assert.equal(citeUrl('IBC 2024 §1010.1.1'), `${BOOK}/chapter-10-means-of-egress`);
  assert.equal(citeUrl('IBC 2024 §1011.2 / §1011.5.2'), `${BOOK}/chapter-10-means-of-egress`);
  assert.equal(citeUrl('IBC 2024 Table 1004.5 / Table 1006.2.1'), `${BOOK}/chapter-10-means-of-egress`);
  assert.equal(citeUrl('IBC §1012.2'), `${BOOK}/chapter-10-means-of-egress`);
  assert.equal(citeUrl('IBC 2024 §303.1'), BOOK, 'unmapped chapter -> book root');
  assert.equal(citeUrl('IBC 2024'), BOOK);
  assert.equal(ibcChapter('§1010.1.1'), 10); assert.equal(ibcChapter('§303.1'), 3); assert.equal(ibcChapter('Table 1106.1'), 11);
});

test('Indiana and unknown citations', () => {
  assert.equal(citeUrl('675 IAC 13-2.7-10'), 'http://iac.iga.in.gov/iac/T06750/A00130.PDF');
  assert.equal(citeUrl('675 IAC 22-2.5'), 'http://iac.iga.in.gov/iac/');
  assert.equal(citeUrl('IBC 2018 §1010.1.1'), null, 'other editions are not mapped');
  assert.equal(citeUrl('NFPA 101 §7.2'), null);
  assert.equal(citeUrl(''), null); assert.equal(citeUrl(undefined), null);
});

test('links open in a new tab and escape the citation', () => {
  const h = citeHtml('IBC 2024 §1010.1.1 <img onerror=x>');
  assert.ok(h.startsWith(`<a href="${BOOK}/chapter-10-means-of-egress" target="_blank" rel="noopener noreferrer">`));
  assert.ok(h.includes('&#60;img onerror=x&#62;') && !h.includes('<img'));
  assert.equal(citeHtml('Local rule "a" <b>'), 'Local rule &#34;a&#34; &#60;b&#62;');
});
