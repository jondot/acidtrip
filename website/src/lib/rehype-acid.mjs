// Markdown to the editor's look, at build time:
//  - an image with a title on its own line becomes a framed screenshot with a
//    caption (click to enlarge), sized from the PNG itself, or dropped when
//    public/ has no such PNG;
//  - tables scroll sideways inside .tbl, code blocks become blue .term boxes
//    with the `$` prompt and `# comments` in cyan;
//  - root-relative links and images get the site's base path.
import fs from 'node:fs';
import path from 'node:path';

// Astro runs from the site's root; pages get bundled elsewhere, so don't
// resolve this from import.meta.url.
const PUBLIC = path.resolve(process.cwd(), 'public');

export function pngSize(rel) {
  try {
    const buf = fs.readFileSync(path.join(PUBLIC, rel));
    return [buf.readUInt32BE(16), buf.readUInt32BE(20)];
  } catch {
    return null;
  }
}

const el = (tagName, properties = {}, children = []) => ({ type: 'element', tagName, properties, children });
const text = (value) => ({ type: 'text', value });

function textOf(node) {
  if (node.type === 'text') return node.value;
  return (node.children || []).map(textOf).join('');
}

// One line of a shell block: "$" and "# comment" in cyan.
function shellLine(line, sh) {
  const out = [];
  let rest = line;
  const m = /^(\s*)\$ /.exec(rest);
  if (m) {
    out.push(text(m[1]), el('span', { className: ['c'] }, [text('$')]), text(' '));
    rest = rest.slice(m[0].length);
  }
  const hash = rest.search(/(^|\s)#(\s|$)/);
  if (hash >= 0 && (sh || m || /^\s*#/.test(rest))) {
    out.push(text(rest.slice(0, hash)), el('span', { className: ['c'] }, [text(rest.slice(hash))]));
  } else out.push(text(rest));
  return out;
}

function tomlLine(line) {
  if (/^\s*\[.*\]\s*$/.test(line) || /^\s*#/.test(line)) return [el('span', { className: ['c'] }, [text(line)])];
  return [text(line)];
}

export default function rehypeAcid({ base = '/' } = {}) {
  const b = base.replace(/\/$/, '');
  const fix = (u) => (typeof u === 'string' && u.startsWith('/') && !u.startsWith('//') && b ? b + u : u);

  function figure(img) {
    const src = img.properties.src;
    const size = src.startsWith('/') ? pngSize(src.slice(1)) : null;
    const caption = img.properties.title;
    const props = {
      className: ['shot'],
      src: fix(src),
      alt: img.properties.alt || '',
      loading: 'lazy',
      decoding: 'async',
      'data-zoom': '',
      'data-tip': 'Click to enlarge',
    };
    if (size) [props.width, props.height] = size;
    // "Bold lead. The rest." captions: the first sentence in bold, like the design.
    const m = /^([^.]+\.)\s*(.*)$/.exec(caption);
    const cap = m && m[2] ? [el('b', {}, [text(m[1])]), text(' ' + m[2])] : [text(caption)];
    return el('figure', {}, [el('img', props), el('figcaption', {}, cap)]);
  }

  function walk(node) {
    if (!node.children) return;
    node.children = node.children.map((c) => {
      if (c.type !== 'element') return c;
      // <p><img title></p> -> <figure>
      if (c.tagName === 'p') {
        const kids = c.children.filter((k) => !(k.type === 'text' && !k.value.trim()));
        if (kids.length === 1 && kids[0].tagName === 'img' && kids[0].properties.title) {
          // A screenshot that wasn't made (or was thrown away) drops out.
          const src = kids[0].properties.src;
          if (src.startsWith('/') && !pngSize(src.slice(1))) return null;
          return figure(kids[0]);
        }
      }
      if (c.tagName === 'table') {
        walk(c);
        return el('div', { className: ['tbl'] }, [c]);
      }
      if (c.tagName === 'pre') {
        const code = c.children.find((k) => k.tagName === 'code') || c;
        const cls = (code.properties && code.properties.className) || [];
        const lang = (cls.find((k) => String(k).startsWith('language-')) || '').slice(9);
        const src = textOf(code).replace(/\n$/, '');
        const lines = src.split('\n');
        const kids = [];
        lines.forEach((line, i) => {
          if (i) kids.push(text('\n'));
          kids.push(...(lang === 'toml' ? tomlLine(line) : lang === 'sh' || /^\s*\$ /.test(line) ? shellLine(line, lang === 'sh') : [text(line)]));
        });
        return el('div', { className: ['term'] }, [el('pre', {}, kids)]);
      }
      if (c.tagName === 'a') {
        c.properties.href = fix(c.properties.href);
        if (/^https?:/.test(c.properties.href || '')) {
          c.properties.target = '_blank';
          c.properties.rel = 'noopener';
        }
      }
      if (c.tagName === 'img') c.properties.src = fix(c.properties.src);
      walk(c);
      return c;
    }).filter(Boolean);
  }

  return (tree) => walk(tree);
}
