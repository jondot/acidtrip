import { defineCollection } from 'astro:content';
import { glob } from 'astro/loaders';
import { z } from 'astro/zod';

// Each doc is a "file" in the editor: a layer in LAYERS, a tab, a tool.
const docs = defineCollection({
  loader: glob({ base: './src/content/docs', pattern: '**/*.{md,mdx}' }),
  schema: z.object({
    title: z.string(),
    nav: z.string(),
    description: z.string(),
    order: z.number(),
    glyph: z.string(),
  }),
});

export const collections = { docs };
