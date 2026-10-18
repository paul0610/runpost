<script lang="ts">
  // Pestañas abiertas (P1): peticiones con su método coloreado, gestores de entorno 🌐 y docs 📄;
  // • si hay cambios sin guardar, ＋ nueva petición.
  import { store } from '../stores/app.svelte';
  import { t } from '../i18n.svelte';
  import { methodClass, methodLabel } from '../format';

  function middleClick(event: MouseEvent, id: string): void {
    if (event.button === 1) {
      event.preventDefault();
      void store.closeTab(id);
    }
  }
</script>

<div class="tabs" role="tablist">
  {#each store.tabs as tab (tab.id)}
    {@const dirty = store.isDirty(tab)}
    {@const title = store.tabTitle(tab)}
    <div
      class="tab"
      class:on={tab.id === store.activeTabId}
      role="tab"
      tabindex="0"
      aria-selected={tab.id === store.activeTabId}
      {title}
      onclick={() => (store.activeTabId = tab.id)}
      onkeydown={(e) => e.key === 'Enter' && (store.activeTabId = tab.id)}
      onauxclick={(e) => middleClick(e, tab.id)}
    >
      {#if tab.kind === 'request'}
        <span class="meth {methodClass(tab.draft.method)}">{methodLabel(tab.draft.method)}</span>
      {:else if tab.kind === 'env'}
        <span class="icon" aria-hidden="true">🌐</span>
      {:else if tab.kind === 'policy'}
        <span class="icon" aria-hidden="true">🤖</span>
      {:else}
        <span class="icon" aria-hidden="true">📄</span>
      {/if}
      <span class="name">{title}</span>
      {#if dirty}<span class="dirty" title={t('tabs.unsaved')}>•</span>{/if}
      <button
        class="close ghost"
        title={t('tabs.close')}
        aria-label={t('tabs.close')}
        onclick={(e) => {
          e.stopPropagation();
          void store.closeTab(tab.id);
        }}>×</button
      >
    </div>
  {/each}
  <button class="add ghost" title={t('tabs.new')} aria-label={t('tabs.new')} onclick={() => store.newTab()}>＋</button>
</div>

<style>
  .tabs { display: flex; border-bottom: 1px solid var(--border); background: var(--bg-2); overflow-x: auto; flex: none; }
  .tab { padding: 0 6px 0 14px; height: 36px; border-right: 1px solid var(--border); display: flex; gap: 8px; align-items: center; cursor: pointer; white-space: nowrap; color: var(--muted); max-width: 240px; }
  .tab.on { background: var(--bg); color: var(--fg); box-shadow: inset 0 2px 0 var(--accent); }
  .tab:focus-visible { outline: 2px solid var(--focus); outline-offset: -2px; }
  .meth { font-weight: 700; font-size: 11px; }
  .icon { font-size: 12px; }
  .name { overflow: hidden; text-overflow: ellipsis; }
  .dirty { color: var(--accent-text); }
  .close { width: 20px; height: 20px; padding: 0; line-height: 1; border: none; visibility: hidden; color: var(--muted); }
  .tab:hover .close, .tab.on .close, .tab:focus-within .close { visibility: visible; }
  .add { border: none; border-radius: 0; padding: 0 14px; color: var(--muted); }
</style>
