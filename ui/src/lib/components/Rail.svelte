<script lang="ts">
  // Barra lateral estrecha (48 px) con las 4 secciones (UX decisión 2). Pulsar la activa pliega el panel.
  import { store, type Pane } from '../stores/app.svelte';
  import { t } from '../i18n.svelte';

  const items: { pane: Pane; icon: string; label: string }[] = [
    { pane: 'collections', icon: '📁', label: 'rail.collections' },
    { pane: 'environments', icon: '🌐', label: 'rail.environments' },
    { pane: 'history', icon: '🕘', label: 'rail.history' },
    { pane: 'agent', icon: '🤖', label: 'rail.agent' },
  ];

  function select(pane: Pane): void {
    if (store.ui.pane === pane && store.ui.sideOpen) {
      store.ui.sideOpen = false;
      return;
    }
    store.ui.pane = pane;
    store.ui.sideOpen = true;
  }
</script>

<nav class="rail" aria-label="RunPost">
  {#each items as item (item.pane)}
    <button
      class:on={store.ui.pane === item.pane && store.ui.sideOpen}
      title={t(item.label)}
      aria-label={t(item.label)}
      onclick={() => select(item.pane)}
    >
      {item.icon}
      {#if item.pane === 'agent' && (store.agent.unseen || store.agent.pending.length)}
        <span class="badge" aria-label={t('agent.newCalls')}>{store.agent.pending.length ? '!' : store.agent.unseen > 99 ? '99+' : store.agent.unseen}</span>
      {/if}
    </button>
  {/each}
</nav>

<style>
  .rail { width: 48px; flex: none; background: var(--bg-2); border-right: 1px solid var(--border); display: flex; flex-direction: column; align-items: center; padding-top: 8px; gap: 4px; }
  .rail button { width: 36px; height: 36px; border: none; border-radius: 8px; font-size: 17px; padding: 0; position: relative; }
  .badge { position: absolute; top: 2px; right: 1px; background: var(--accent); color: var(--on-accent); font-size: 9px; font-weight: 700; line-height: 14px; min-width: 14px; border-radius: 999px; padding: 0 3px; }
  .rail button.on { background: var(--bg-3); box-shadow: inset 3px 0 0 var(--accent); }
</style>
