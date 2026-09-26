// The acidtrip website: a static site built as the editor itself.
import { defineConfig } from 'astro/config';
import { unified } from '@astrojs/markdown-remark';
import rehypeAcid from './src/lib/rehype-acid.mjs';

// Set SITE_BASE=/acidtrip/ to host it under a path (GitHub Pages, say).
const base = process.env.SITE_BASE || '/';

export default defineConfig({
  output: 'static',
  base,
  trailingSlash: 'always',
  build: { format: 'directory' },
  markdown: {
    syntaxHighlight: false,
    processor: unified({ rehypePlugins: [[rehypeAcid, { base }]] }),
  },
  devToolbar: { enabled: false },
});
