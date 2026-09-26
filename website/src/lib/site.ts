import { getCollection, render, type CollectionEntry } from 'astro:content';

/** The project's home. Change it here and every link follows. */
export const GITHUB = 'https://github.com/jondot/acidtrip';
export const INSTALL = 'curl -fsSL https://raw.githubusercontent.com/jondot/acidtrip/main/install.sh | sh';
export const RELEASES = GITHUB + '/releases/latest';
export const FROM_SOURCE = 'cargo install --git https://github.com/jondot/acidtrip acidtrip';

/** A root-relative path with the site's base in front. */
export function url(p: string): string {
  const base = import.meta.env.BASE_URL.replace(/\/$/, '');
  return base + (p.startsWith('/') ? p : '/' + p);
}

export interface Section {
  id: string;
  label: string;
}

/** A "file" open in the editor: a page of the site. */
export interface Page {
  id: string;
  file: string;
  name: string;
  tool: string;
  desc: string;
  href: string;
  sections: Section[];
  doc?: boolean;
}

// The top pages' sections: the ids their h2s carry, for the options chips
// and the command palette.
const TOP: Omit<Page, 'href'>[] = [
  {
    id: 'index', file: 'index.ans', name: 'Home', tool: '⌂', desc: 'the front page: what acidtrip is',
    sections: [
      { id: 'draw', label: 'draw' }, { id: 'together', label: 'together' }, { id: 'replay', label: 'replay' },
      { id: 'frames', label: 'frames' }, { id: 'gallery', label: 'gallery' }, { id: 'studio', label: 'studio' },
      { id: 'formats', label: 'formats' }, { id: 'export', label: 'export' },
    ],
  },
  {
    id: 'features', file: 'features.ans', name: 'Features', tool: '▚', desc: 'every tool and its key',
    sections: [
      { id: 'tools', label: 'Tools' }, { id: 'sidebar', label: 'The sidebar' }, { id: 'art', label: 'The Art tool' },
      { id: 'pen', label: 'The smart pen and brushes' }, { id: 'gradient', label: 'Gradient fill' },
      { id: 'filters', label: 'Filters' }, { id: 'pattern', label: 'Pattern brush' },
      { id: 'safe', label: 'Never lose work' },
    ],
  },
  {
    id: 'docs', file: 'docs/index.ans', name: 'Docs', tool: '?', desc: 'the manual, one file per topic',
    sections: [],
  },
  {
    id: 'gallery', file: 'gallery.ans', name: 'Gallery', tool: '♦', desc: 'screens and lettering from the real editor',
    sections: [{ id: 'lettering', label: 'Lettering' }, { id: 'screens', label: 'Screens' }],
  },
  {
    id: 'download', file: 'download.ans', name: 'Download', tool: '↓', desc: 'install, terminals and the command line',
    sections: [
      { id: 'install', label: 'Install' }, { id: 'terminals', label: 'Terminals' },
      { id: 'files', label: 'Files and config' }, { id: 'cli', label: 'The command line' },
      { id: 'credits', label: 'Credits' },
    ],
  },
];

const HREF: Record<string, string> = {
  index: '/', features: '/features/', docs: '/docs/', gallery: '/gallery/', download: '/download/',
};

export async function docEntries(): Promise<CollectionEntry<'docs'>[]> {
  return (await getCollection('docs')).sort((a, b) => a.data.order - b.data.order);
}

let cache: Promise<Page[]> | undefined;

/** Every page, in LAYERS order: the top pages, then the docs. */
export function pages(): Promise<Page[]> {
  cache ??= (async () => {
    const top = TOP.map((p) => ({ ...p, href: url(HREF[p.id]) }));
    const docs = await Promise.all(
      (await docEntries()).map(async (d) => {
        const { headings } = await render(d);
        return {
          id: d.id,
          file: `docs/${d.id}.ans`,
          name: d.data.nav,
          tool: d.data.glyph,
          desc: d.data.description,
          href: url(`/docs/${d.id}/`),
          sections: headings.filter((h) => h.depth === 2).map((h) => ({ id: h.slug, label: h.text })),
          doc: true,
        } satisfies Page;
      }),
    );
    return [...top, ...docs];
  })();
  return cache;
}

/** Heading art for a page: public/art/NAME.png. */
export function art(name: string): string {
  return url(`/art/${name}.png`);
}
