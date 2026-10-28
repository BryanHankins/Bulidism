// Citation link-outs. Loaded both as a classic <script> (viewer.js, standalone report) and as an ES module
// import (print.js, node tests), so it only sets globalThis.BuildsimCites.
(function (g) {
  const IBC = 'https://codes.iccsafe.org/content/IBC2024P1';
  // chapter pages whose URL slug is known; anything else links to the book's table of contents
  const IBC_CHAPTERS = { 10: 'chapter-10-means-of-egress' };
  const IN_BUILDING = 'http://iac.iga.in.gov/iac/T06750/A00130.PDF'; // 675 IAC 13: Indiana building codes
  const IN_ROOT = 'http://iac.iga.in.gov/iac/';
  const esc = v => String(v ?? '').replace(/[&<>"']/g, c => '&#' + c.charCodeAt(0) + ';');

  /** Chapter of the first IBC section/table number in a citation: §1010.1.1 -> 10, Table 1004.5 -> 10, §303.1 -> 3. */
  function ibcChapter(cite) {
    const m = /(?:§|Section|Table)\s*(\d{3,4})(?:\.|\b)/i.exec(cite);
    return m ? +m[1].slice(0, m[1].length - 2) : null;
  }

  /** URL for a rule citation, or null when it names no source we can link. */
  function citeUrl(cite) {
    const c = String(cite ?? '');
    if (/\b675\s*IAC\b/i.test(c)) return /\b675\s*IAC\s*13\b/i.test(c) ? IN_BUILDING : IN_ROOT;
    const m = /\bIBC\b(?:\s*(\d{4}))?/i.exec(c);
    if (!m || (m[1] && m[1] !== '2024')) return null; // only the 2024 edition's online book is mapped
    const ch = IBC_CHAPTERS[ibcChapter(c)];
    return ch ? `${IBC}/${ch}` : IBC;
  }

  /** Escaped citation text, as a new-tab link when it maps to a source. */
  function citeHtml(cite) {
    const u = citeUrl(cite);
    return u ? `<a href="${esc(u)}" target="_blank" rel="noopener noreferrer">${esc(cite)}</a>` : esc(cite);
  }

  g.BuildsimCites = { citeUrl, citeHtml, ibcChapter };
})(typeof window !== 'undefined' ? window : globalThis);
