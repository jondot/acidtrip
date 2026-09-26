// The editor chrome's behaviour: status-bar tips, COLORS, CHARACTERS, the
// MAP, replay, copy buttons, the command palette, the lightbox and the keys.
// Plain DOM, no framework.

interface PageInfo { id: string; file: string; desc: string; href: string; doc: boolean }
interface Vga { name: string; hex: string; light: boolean }
interface SiteData { github: string; install: string; pages: PageInfo[]; current: string; vga: Vga[] }
interface Entry { label: string; hint: string; href?: string; run?: () => void }

const $ = <T extends Element = HTMLElement>(s: string) => document.querySelector(s) as T;
const $$ = <T extends Element = HTMLElement>(s: string) => [...document.querySelectorAll(s)] as T[];
const SITE: SiteData = JSON.parse($('#site-data').textContent || '{}');
const VGA = SITE.vga;
const HEX = VGA.map((c) => c.hex);
const store = {
  get(k: string) { try { return JSON.parse(localStorage.getItem('acidtrip.site.' + k) || 'null'); } catch { return null; } },
  set(k: string, v: unknown) { try { localStorage.setItem('acidtrip.site.' + k, JSON.stringify(v)); } catch { /* private mode */ } },
};
const esc = (s: unknown) => String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]!);
const reduced = () => matchMedia('(prefers-reduced-motion: reduce)').matches;

const canvas = $('#canvas'), sheet = $('#sheet'), tipEl = $('#tip'), side = $('#side'), page = $('.page');
const current = SITE.pages.find((p) => p.id === SITE.current)!;

/* ── status bar tips, like the app ── */
const DEFAULT_TIP = 'Welcome to acidtrip — hover anything · click a tool or a layer · Ctrl-K commands';
function tip(text: string, key?: string, warn = false) {
  tipEl.className = warn ? 'warn' : '';
  tipEl.innerHTML = '<span class="ic">▸</span> ' + esc(text) + (key ? '  <span class="key">[' + esc(key) + ']</span>' : '');
}
let flashT: ReturnType<typeof setTimeout> | undefined;
function flash(text: string, warn = false) {
  clearTimeout(flashT);
  tip(text, '', warn);
  flashT = setTimeout(() => tip(DEFAULT_TIP), 2600);
}
document.addEventListener('mouseover', (e) => {
  const t = (e.target as Element).closest<HTMLElement>('[data-tip]');
  if (t) { clearTimeout(flashT); tip(t.dataset.tip!, t.dataset.key); return; }
  const a = (e.target as Element).closest('a[href]');
  if (a) {
    clearTimeout(flashT);
    const href = a.getAttribute('href')!;
    tip(/^https?:/.test(href) ? 'Open ' + href + ' in a new tab' : href.startsWith('#') ? 'Jump to ' + a.textContent!.trim() : 'Go to ' + a.textContent!.trim());
  }
});
document.addEventListener('mouseout', (e) => {
  const rel = e.relatedTarget as Element | null;
  if ((e.target as Element).closest('[data-tip],a[href]') && !rel?.closest?.('[data-tip],a[href]')) tip(DEFAULT_TIP);
});

/* ── the docs tab: the last doc you read ── */
if (current.doc) store.set('lastdoc', current.id);
else {
  const last = SITE.pages.find((p) => p.doc && p.id === store.get('lastdoc'));
  const tab = $<HTMLAnchorElement>('[data-doc-tab]');
  if (last && tab) { tab.href = last.href; tab.textContent = last.file; tab.dataset.tip = `${last.file}: ${last.desc}`; }
}
// On a narrow screen the tab strip scrolls: bring the open file into view.
{
  const strip = $<HTMLElement>('#tabs');
  const on = $<HTMLElement>('#tabs [aria-current="page"]');
  if (strip && on) {
    const over = on.getBoundingClientRect().right - strip.getBoundingClientRect().right;
    if (over > 0) strip.scrollLeft += over + 8;
  }
}

/* ── sections: jump, and light the chip of the one you're reading ── */
function jump(id: string) {
  const el = document.getElementById(id);
  if (!el) return;
  canvas.scrollTo({ top: el.getBoundingClientRect().top - sheet.getBoundingClientRect().top - 12, behavior: reduced() ? 'auto' : 'smooth' });
  history.replaceState(null, '', '#' + id);
  closeSheet();
}
document.addEventListener('click', (e) => {
  const a = (e.target as Element).closest<HTMLAnchorElement>('a[href^="#"]');
  if (a && a.getAttribute('href')!.length > 1) { e.preventDefault(); jump(decodeURIComponent(a.getAttribute('href')!.slice(1))); return; }
  if ((e.target as Element).closest('[data-cmd]')) openPalette();
});
const chips = $$<HTMLAnchorElement>('#opts a, .toc a');
function markSection() {
  let at = '';
  const top = canvas.getBoundingClientRect().top + 40;
  for (const c of chips) {
    const el = document.getElementById(c.getAttribute('href')!.slice(1));
    if (el && el.getBoundingClientRect().top <= top) at = el.id;
  }
  chips.forEach((c) => c.getAttribute('href') === '#' + at ? c.setAttribute('aria-current', 'true') : c.removeAttribute('aria-current'));
}

/* ── CHARACTERS: the glyph paints the page's rules ── */
const KEYS = ['1', '2', '3', '4', '5', '6', '7', '8', '9', '0'];
const SETS: [string, string[]][] = [
  ['blocks & shades', ['░', '▒', '▓', '█', '▀', '▄', '▌', '▐', '■', '·']],
  ['single lines', ['─', '│', '┌', '┐', '└', '┘', '├', '┤', '┼', '·']],
  ['double lines', ['═', '║', '╔', '╗', '╚', '╝', '╠', '╣', '╬', '·']],
  ['mixed ╓', ['─', '║', '╓', '╖', '╙', '╜', '╟', '╢', '╫', '·']],
  ['mixed ╒', ['═', '│', '╒', '╕', '╘', '╛', '╞', '╡', '╪', '·']],
];
let glyph = 3, setIx = 0;
const chars0 = store.get('chars');
if (chars0 && SETS[chars0.set] && chars0.glyph >= 0 && chars0.glyph < 10) { setIx = chars0.set; glyph = chars0.glyph; }
const glyphs = () => SETS[setIx][1];
function buildGlyphs() {
  const g = glyphs();
  $('#glyphs').innerHTML = KEYS.map((k, i) =>
    `<button data-glyph="${i}" aria-pressed="${i === glyph}" data-tip="Draw the page's rules with ${g[i]}" data-key="${k}"><b>${g[i]}</b><span>${k}</span></button>`).join('');
  $('#set').innerHTML = '<span class="lbl">set ' + (setIx + 6) + '</span>' + KEYS.map((k, i) =>
    `<button data-glyph="${i}" aria-pressed="${i === glyph}" data-tip="Draw the page's rules with ${g[i]}" data-key="${k}">${k}<b>${g[i]}</b></button>`).join('');
  $('.chars .name').textContent = `${SETS[setIx][0]} ${setIx + 6}/15`;
  store.set('chars', { set: setIx, glyph });
}
function fillRules() {
  const g = glyphs()[glyph];
  const run = g === '·' ? '· ' : g;
  $$('.rule').forEach((r) => (r.textContent = run.repeat(Math.ceil(240 / run.length))));
}
function pickGlyph(i: number) { glyph = i; buildGlyphs(); fillRules(); flash('rules now drawn with ' + glyphs()[i]); }
function stepSet(d: number) { setIx = (setIx + d + SETS.length) % SETS.length; buildGlyphs(); fillRules(); flash('character set: ' + SETS[setIx][0]); }
document.addEventListener('click', (e) => { const b = (e.target as Element).closest<HTMLElement>('[data-glyph]'); if (b) pickGlyph(+b.dataset.glyph!); });
$('#prevset').onclick = () => stepSet(-1);
$('#nextset').onclick = () => stepSet(1);

/* ── COLORS: FG recolors the accent, BG the canvas ── */
let fg = 13, bg = 0, ice = false, active: 'fg' | 'bg' = 'fg';
const col0 = store.get('colors');
if (col0 && HEX[col0.fg] && HEX[col0.bg] && col0.fg !== col0.bg) { fg = col0.fg; bg = col0.bg; ice = !!col0.ice; }
const ink = (i: number) => (VGA[i].light ? '#000' : '#fff');
function applyColors() {
  const r = document.documentElement.style;
  r.setProperty('--accent', HEX[fg]);
  r.setProperty('--on-accent', ink(fg));
  r.setProperty('--canvas', HEX[bg]);
  const fb = $('#fgbox'), bb = $('#bgbox');
  fb.style.background = HEX[fg]; fb.style.color = ink(fg);
  bb.style.background = HEX[bg]; bb.style.color = ink(bg);
  $('#fgname').textContent = VGA[fg].name; $('#bgname').textContent = VGA[bg].name;
  $('#fglbl').textContent = active === 'fg' ? '▸FG◂' : 'FG';
  $('#bglbl').textContent = active === 'bg' ? '▸BG◂' : 'BG';
  $('#palhint').textContent = 'click a color: ' + (active === 'fg' ? 'foreground' : 'background');
  $('#fgsw').style.background = HEX[fg];
  $('#ice').setAttribute('aria-pressed', String(ice));
  $('#pal').innerHTML = HEX.map((h, i) => {
    const mark = i === fg ? 'F' : i === bg ? 'B' : '';
    return `<button style="background:${h};color:${ink(i)}" data-col="${i}" aria-label="${VGA[i].name}" data-tip="${VGA[i].name}: set the ${active === 'fg' ? 'accent (FG)' : 'canvas (BG)'}">${mark}</button>`;
  }).join('');
  store.set('colors', { fg, bg, ice });
  requestAnimationFrame(drawMap);
}
function setColor(i: number) {
  if (active === 'fg') {
    if (i === bg) return flash('FG and BG would be the same color. Pick another', true);
    fg = i; flash('accent: ' + VGA[i].name);
  } else {
    if (i > 7 && !ice) return flash('without iCE, backgrounds stay in the dark 8. Click iCE in the status bar', true);
    if (i === fg) return flash('FG and BG would be the same color. Pick another', true);
    bg = i; flash('canvas: ' + VGA[i].name);
  }
  applyColors();
}
$('#pal').addEventListener('click', (e) => { const b = (e.target as Element).closest<HTMLElement>('[data-col]'); if (b) setColor(+b.dataset.col!); });
$('#fgbox').onclick = () => { active = 'fg'; applyColors(); };
$('#bgbox').onclick = () => { active = 'bg'; applyColors(); };
function swap() {
  if (fg > 7 && !ice) return flash(`can't swap: ${VGA[fg].name} needs iCE to be a background`, true);
  [fg, bg] = [bg, fg]; applyColors(); flash('swapped FG and BG');
}
$('#swap').onclick = swap;
$('#reset').onclick = () => { fg = 13; bg = 0; active = 'fg'; applyColors(); flash('colors reset'); };
function toggleIce() {
  ice = !ice;
  if (!ice && bg > 7) bg -= 8;
  applyColors(); flash(ice ? 'iCE on: all 16 backgrounds' : 'iCE off: backgrounds in the dark 8');
}
$('#ice').onclick = toggleIce;
$('#classic').onclick = () => flash('this site is a Classic document: CP437 and the 16 VGA colors');

/* ── status bar: cursor cell and page size ── */
function cellWidth() {
  const s = document.createElement('span');
  s.textContent = 'M'.repeat(40);
  s.style.cssText = 'position:absolute;visibility:hidden;font:14px var(--mono)';
  document.body.append(s);
  const w = s.getBoundingClientRect().width / 40;
  s.remove();
  return w || 8.4;
}
let cw = cellWidth();
sheet.addEventListener('mousemove', (e) => {
  const r = sheet.getBoundingClientRect();
  $('#pos').textContent = Math.max(0, Math.floor((e.clientX - r.left) / cw)) + ',' + Math.max(0, Math.floor((e.clientY - r.top) / 18));
});
function sizeStatus() { $('#size').textContent = Math.round(sheet.clientWidth / cw) + 'x' + Math.round(sheet.scrollHeight / 18); }

/* ── MAP: the whole page, drawn small, with your view on it ── */
const map = $<HTMLCanvasElement>('#map'), mctx = map.getContext('2d')!;
let mapK = 1;
function drawMap() {
  const dpr = devicePixelRatio || 1, W = map.clientWidth, H = map.clientHeight;
  if (!W) return;
  map.width = W * dpr; map.height = H * dpr;
  mctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  mctx.imageSmoothingEnabled = false;
  mctx.fillStyle = '#000'; mctx.fillRect(0, 0, W, H);
  const sw = sheet.offsetWidth, sh = sheet.scrollHeight;
  const k = Math.min(W / sw, H / sh), ox = (W - sw * k) / 2;
  const base = sheet.getBoundingClientRect();
  mctx.fillStyle = HEX[bg]; mctx.fillRect(ox, 0, sw * k, sh * k);
  const at = (el: Element) => { const r = el.getBoundingClientRect(); return [ox + (r.left - base.left) * k, (r.top - base.top) * k, r.width * k, r.height * k]; };
  page.querySelectorAll<HTMLImageElement>('img').forEach((im) => {
    if (im.complete && im.naturalWidth) { const [x, y, w, h] = at(im); if (h > 0) mctx.drawImage(im, x, y, w, h); }
  });
  page.querySelectorAll('p,li,td,th,h2,h3,.rule,figcaption,.path,.toc').forEach((el) => {
    const [x, y, w, h] = at(el);
    const hi = el.matches('h2,h3,.rule,th,.toc');
    mctx.fillStyle = hi ? HEX[fg] : el.matches('.lead') ? HEX[15] : HEX[7];
    mctx.globalAlpha = hi ? 0.9 : 0.45;
    const lines = Math.max(1, Math.round(h / (18 * k)));
    for (let i = 0; i < lines; i++) mctx.fillRect(x, y + i * 18 * k + 5 * k, w * (i === lines - 1 && lines > 1 ? 0.6 : 1), Math.max(1, 8 * k));
    mctx.globalAlpha = 1;
  });
  page.querySelectorAll('.term').forEach((el) => { const [x, y, w, h] = at(el); mctx.fillStyle = HEX[1]; mctx.globalAlpha = 0.8; mctx.fillRect(x, y, w, h); mctx.globalAlpha = 1; });
  // your view
  const vy = canvas.scrollTop * k, vh = canvas.clientHeight * k;
  mctx.strokeStyle = '#55ffff'; mctx.lineWidth = 1;
  mctx.strokeRect(ox + 0.5, Math.max(0.5, vy + 0.5), sw * k - 1, Math.min(vh, H) - 1);
  mapK = k;
}
function mapJump(e: PointerEvent) {
  const r = map.getBoundingClientRect();
  canvas.scrollTop = (e.clientY - r.top) / mapK - canvas.clientHeight / 2;
}
let dragging = false;
map.addEventListener('pointerdown', (e) => { dragging = true; map.setPointerCapture(e.pointerId); mapJump(e); });
map.addEventListener('pointermove', (e) => { if (dragging) mapJump(e); });
map.addEventListener('pointerup', () => (dragging = false));
let raf = 0;
canvas.addEventListener('scroll', () => { cancelAnimationFrame(raf); raf = requestAnimationFrame(() => { drawMap(); markSection(); }); }, { passive: true });
addEventListener('resize', () => { sizeStatus(); drawMap(); });
$$<HTMLImageElement>('.page img').forEach((im) => im.addEventListener('load', () => { drawMap(); sizeStatus(); }));

/* ── replay: the page's lettering, modem reveal ── */
function replay() {
  const l = $('.page [data-lettering]');
  if (!l) return flash('nothing to replay on this page', true);
  canvas.scrollTo({ top: 0 });
  l.classList.remove('reveal'); void l.offsetWidth; l.classList.add('reveal');
  flash('replaying at 14400 baud');
}
$('#replay-btn').onclick = replay;

/* ── copy buttons ── */
function copy(text: string, fallback: () => void) {
  try { navigator.clipboard.writeText(text).then(() => flash('copied: ' + text), fallback); } catch { fallback(); }
}
document.addEventListener('click', (e) => {
  const b = (e.target as Element).closest<HTMLElement>('.copy');
  if (!b) return;
  const box = b.closest<HTMLElement>('[data-copy]')!;
  copy(box.dataset.copy!, () => {
    const sel = getSelection()!, r = document.createRange();
    r.selectNodeContents(box.querySelector('pre')!); sel.removeAllRanges(); sel.addRange(r);
    flash('selected: press Ctrl-C to copy');
  });
  b.textContent = 'copied'; setTimeout(() => (b.textContent = 'copy'), 1600);
});

/* ── command palette ── */
const dlg = $('#pal-dlg'), inp = $<HTMLInputElement>('#pal-in'), list = $('#pal-list');
const INDEX: Entry[] = JSON.parse($('#search-index').textContent || '[]');
const commands = (): Entry[] => [
  ...INDEX,
  { label: 'Copy install command', hint: 'curl | sh', run: () => copy(SITE.install, () => { location.href = SITE.pages.find((p) => p.id === 'download')!.href; }) },
  { label: 'Open GitHub', hint: SITE.github.replace(/^https:\/\//, ''), href: SITE.github },
  { label: 'Swap FG and BG', hint: 'X', run: swap },
  { label: 'Toggle iCE colors', hint: 'Alt-Z', run: toggleIce },
  { label: 'Replay the lettering', hint: 'Shift-R', run: replay },
  { label: 'Next character set', hint: ']', run: () => stepSet(1) },
  { label: 'Reset colors', hint: 'magenta on black', run: () => $('#reset').click() },
];
function score(q: string, s: string) {
  if (!q) return { s: 0, hits: [] as number[] };
  const low = s.toLowerCase();
  let i = 0, sc = 0, prev = -2;
  const hits: number[] = [];
  for (const ch of q.toLowerCase()) {
    if (ch === ' ') continue;
    const j = low.indexOf(ch, i);
    if (j < 0) return null;
    sc += j === prev + 1 ? 3 : 1;
    if (j === 0 || ' ›/.'.includes(low[j - 1])) sc += 2;
    hits.push(j); prev = j; i = j + 1;
  }
  return { s: sc - s.length * 0.01, hits };
}
let items: { c: Entry; m: { s: number; hits: number[] } }[] = [], sel = 0;
function render() {
  const q = inp.value.trim();
  items = commands().map((c) => ({ c, m: score(q, c.label)! })).filter((x) => x.m).sort((a, b) => b.m.s - a.m.s).slice(0, 60);
  sel = Math.min(sel, Math.max(0, items.length - 1));
  list.innerHTML = items.length
    ? items.map(({ c, m }, i) => {
        const hl = [...c.label].map((ch, j) => (m.hits.includes(j) ? `<span class="hl">${esc(ch)}</span>` : esc(ch))).join('');
        return `<li role="option" data-i="${i}" aria-selected="${i === sel}"><span>${hl}</span><span class="k">${esc(c.hint)}</span></li>`;
      }).join('')
    : '<li class="k">nothing matches "' + esc(q) + '"</li>';
  list.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest' });
}
function openPalette() { closeSheet(); dlg.hidden = false; inp.value = ''; sel = 0; render(); inp.focus(); }
function closePalette() { dlg.hidden = true; canvas.focus({ preventScroll: true }); }
function runSel() {
  const it = items[sel]?.c;
  if (!it) return;
  closePalette();
  if (it.run) return it.run();
  if (!it.href) return;
  if (/^https?:/.test(it.href)) { window.open(it.href, '_blank', 'noopener'); return; }
  const u = new URL(it.href, location.href);
  if (u.pathname === location.pathname && u.hash) jump(decodeURIComponent(u.hash.slice(1)));
  else location.href = it.href;
}
inp.addEventListener('input', () => { sel = 0; render(); });
inp.addEventListener('keydown', (e) => {
  if (e.key === 'ArrowDown') { sel = Math.min(sel + 1, items.length - 1); render(); e.preventDefault(); }
  else if (e.key === 'ArrowUp') { sel = Math.max(sel - 1, 0); render(); e.preventDefault(); }
  else if (e.key === 'Enter') { runSel(); e.preventDefault(); }
  else if (e.key === 'Escape') { closePalette(); e.preventDefault(); }
});
list.addEventListener('mousemove', (e) => { const li = (e.target as Element).closest<HTMLElement>('[data-i]'); if (li && +li.dataset.i! !== sel) { sel = +li.dataset.i!; render(); } });
list.addEventListener('click', (e) => { const li = (e.target as Element).closest<HTMLElement>('[data-i]'); if (li) { sel = +li.dataset.i!; runSel(); } });
dlg.addEventListener('click', (e) => { if (e.target === dlg) closePalette(); });
$('#cmd').onclick = openPalette;

/* ── phones: the panels are a bottom sheet ── */
const menu = $('#menu');
function closeSheet() { side.classList.remove('open'); menu.setAttribute('aria-expanded', 'false'); }
menu.onclick = () => { const open = side.classList.toggle('open'); menu.setAttribute('aria-expanded', String(open)); };

/* ── lightbox ── */
const zoom = $('#zoom'), zimg = $<HTMLImageElement>('#zoom img'), zcap = $('#zoom .cap');
function openZoom(img: HTMLImageElement) {
  zimg.src = img.currentSrc || img.src;
  zimg.alt = img.alt;
  zimg.width = img.naturalWidth || img.width;
  zimg.height = img.naturalHeight || img.height;
  // Native size when it fits, else scaled to the window.
  const view = { w: innerWidth - 16, h: innerHeight - 80 };
  zimg.classList.toggle('fit', zimg.width > view.w || zimg.height > view.h);
  $('#zoom-name').textContent = img.src.split('/').pop()!;
  zcap.innerHTML = img.closest('figure')?.querySelector('figcaption')?.innerHTML ?? esc(img.alt);
  zoom.hidden = false;
  $('#zoom-close').focus();
}
function closeZoom() { zoom.hidden = true; }
document.addEventListener('click', (e) => {
  const img = (e.target as Element).closest<HTMLImageElement>('img[data-zoom]');
  if (img) { openZoom(img); return; }
  if (!zoom.hidden && (e.target as Element).closest('#zoom')) closeZoom();
});

/* ── keys ── */
document.addEventListener('keydown', (e) => {
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') { e.preventDefault(); dlg.hidden ? openPalette() : closePalette(); return; }
  if (!zoom.hidden) { if (e.key === 'Escape' || e.key === 'Enter') { closeZoom(); e.preventDefault(); } return; }
  if (!dlg.hidden || (e.target instanceof Element && e.target.matches('input,textarea'))) return;
  if (e.key === 'Escape') { closeSheet(); return; }
  if (e.ctrlKey || e.metaKey) return;
  if (e.altKey && /^Digit[1-9]$/.test(e.code)) { e.preventDefault(); const p = SITE.pages[+e.code.slice(5) - 1]; if (p) location.href = p.href; return; }
  if (e.altKey && e.code === 'KeyZ') { e.preventDefault(); toggleIce(); return; }
  if (e.altKey) return;
  const n = KEYS.indexOf(e.key);
  if (n >= 0) return pickGlyph(n);
  if (e.key === '[') return stepSet(-1);
  if (e.key === ']') return stepSet(1);
  if (e.key === '?') return openPalette();
  if (e.key === 'x' || e.key === 'X') return swap();
  if (e.key === 'R' && e.shiftKey) return replay();
});

/* ── boot ── */
buildGlyphs();
fillRules();
applyColors();
tip(DEFAULT_TIP);
requestAnimationFrame(() => { sizeStatus(); drawMap(); markSection(); });
document.fonts?.ready.then(() => { cw = cellWidth(); sizeStatus(); drawMap(); });
