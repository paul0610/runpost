<script lang="ts">
  // Documentación markdown de una colección o carpeta (S4), renderizada; "Editar" muestra el texto.
  import { store, type DocsTab } from '../stores/app.svelte';
  import { t } from '../i18n.svelte';
  import MarkdownBlock from './MarkdownBlock.svelte';

  let { tab }: { tab: DocsTab } = $props();

  const collection = $derived(store.collection(tab.collectionId));
  const title = $derived(store.tabTitle(tab).replace(/^[^·]*·\s*/, ''));
</script>

<div class="docs-view">
  <header>
    <h2>📄 {title}</h2>
    {#if tab.itemId && collection}<span class="pill">{collection.name}</span>{/if}
    <span class="spacer"></span>
    <button onclick={() => (tab.editing = !tab.editing)}>{tab.editing ? t('docs.preview') : t('docs.edit')}</button>
    <button class="primary" disabled={!store.isDirty(tab)} onclick={() => store.saveTab(tab)}>{t('req.save')}</button>
  </header>
  {#if tab.editing}
    <textarea class="mono source" bind:value={tab.draft} placeholder={t('docs.placeholder')} aria-label={t('docs.tab')} spellcheck="true"></textarea>
    <p class="muted hint">{t('docs.hint')}</p>
  {:else}
    <MarkdownBlock source={tab.draft} empty={t('docs.empty')} />
  {/if}
</div>

<style>
  .docs-view { flex: 1; min-height: 0; display: flex; flex-direction: column; padding: 12px 16px; gap: 10px; overflow: auto; }
  header { display: flex; gap: 8px; align-items: center; }
  h2 { margin: 0; font-size: 15px; }
  .source { flex: 1; min-height: 240px; font-size: 12.5px; line-height: 1.55; }
  .hint { font-size: 12px; margin: 0; }
</style>
