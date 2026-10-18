<script lang="ts">
  // Bloque de markdown seguro (lib/markdown.ts escapa todo el HTML antes de añadir etiquetas).
  import { renderMarkdown } from '../markdown';

  let { source, empty }: { source: string; empty: string } = $props();
  const html = $derived(renderMarkdown(source));
</script>

{#if source.trim()}
  <!-- HTML de renderMarkdown: la entrada ya va escapada y los enlaces filtrados. -->
  <div class="md">{@html html}</div>
{:else}
  <p class="muted">{empty}</p>
{/if}

<style>
  .md { line-height: 1.6; max-width: 820px; }
  .md :global(h1), .md :global(h2), .md :global(h3) { margin: 18px 0 8px; line-height: 1.3; }
  .md :global(h1) { font-size: 20px; }
  .md :global(h2) { font-size: 17px; }
  .md :global(h3) { font-size: 15px; }
  .md :global(p) { margin: 8px 0; }
  .md :global(code) { background: var(--code-bg); border: 1px solid var(--border); border-radius: 4px; padding: 0 4px; font-family: Consolas, monospace; font-size: 12px; }
  .md :global(pre) { background: var(--code-bg); border: 1px solid var(--border); border-radius: 6px; padding: 10px 12px; overflow: auto; }
  .md :global(pre code) { border: none; padding: 0; }
  .md :global(a) { color: var(--info); }
  .md :global(blockquote) { margin: 8px 0; padding: 4px 12px; border-left: 3px solid var(--border); color: var(--muted); }
  .md :global(ul), .md :global(ol) { padding-left: 22px; }
</style>
