<script lang="ts">
  // Estructura de docs/ux/mockup.html (P1): barra superior, barra lateral de iconos + panel,
  // área central con pestañas y panel inferior. Atajos de docs/UX.md sección 4.
  import { onDestroy, onMount } from 'svelte';
  import { store } from './lib/stores/app.svelte';
  import { t } from './lib/i18n.svelte';
  import TopBar from './lib/components/TopBar.svelte';
  import Rail from './lib/components/Rail.svelte';
  import SidePanel from './lib/components/SidePanel.svelte';
  import TabsBar from './lib/components/TabsBar.svelte';
  import Workbench from './lib/components/Workbench.svelte';
  import EnvEditor from './lib/components/EnvEditor.svelte';
  import DocsView from './lib/components/DocsView.svelte';
  import PolicyView from './lib/components/PolicyView.svelte';
  import Drawer from './lib/components/Drawer.svelte';
  import EmptyState from './lib/components/EmptyState.svelte';
  import Palette from './lib/components/Palette.svelte';
  import SettingsDialog from './lib/components/SettingsDialog.svelte';
  import AboutDialog from './lib/components/AboutDialog.svelte';
  import Dialogs from './lib/components/Dialogs.svelte';
  import ContextMenu from './lib/components/ContextMenu.svelte';
  import Toasts from './lib/components/Toasts.svelte';

  let persistTimer: ReturnType<typeof setTimeout> | undefined;

  function onKey(event: KeyboardEvent): void {
    if (store.status !== 'ready') return;
    if (store.dialog) return; // los diálogos gestionan su propio teclado
    // Mayús+F10 o la tecla Menú abren el menú contextual de la fila enfocada del panel lateral.
    if ((event.key === 'F10' && event.shiftKey) || event.key === 'ContextMenu') {
      const focused = document.activeElement;
      if (focused instanceof HTMLElement && focused.closest('aside')) {
        event.preventDefault();
        focused.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true }));
      }
      return;
    }
    const ctrl = event.ctrlKey || event.metaKey;
    if (!ctrl) return;
    const key = event.key.toLowerCase();
    const handled = (() => {
      if (key === 'enter') return void store.send(), true;
      if (key === 's' && !event.shiftKey) return void store.saveTab(), true;
      if (key === 't' && !event.shiftKey) return store.newTab(), true;
      if (key === 'w') return store.activeTabId && void store.closeTab(store.activeTabId), true;
      if (key === 'k') return (store.ui.palette = !store.ui.palette), true;
      if (key === 'b') return (store.ui.sideOpen = !store.ui.sideOpen), true;
      if (key === ',') return (store.ui.settings = true), true;
      if (key === 'l' && event.shiftKey) return void store.cycleTheme(), true;
      if (key === 'e' && !event.shiftKey) return (store.ui.envMenu = !store.ui.envMenu), true;
      if (key === 'c' && event.altKey) return (store.ui.drawer = store.ui.drawer === 'console' ? null : 'console'), true;
      return false;
    })();
    if (handled) {
      event.preventDefault();
      event.stopPropagation();
    }
  }

  onMount(() => {
    void store.init();
    window.addEventListener('keydown', onKey, true);
  });

  onDestroy(() => {
    window.removeEventListener('keydown', onKey, true);
    store.destroy();
  });

  // Mirar las llamadas del agente (panel lateral o pestaña inferior) las da por vistas.
  $effect(() => {
    if (store.agentViewing && store.agent.unseen) store.agent.unseen = 0;
  });

  // Pestañas, panel y árbol plegado persisten en localStorage (sección 9.1).
  $effect(() => {
    $state.snapshot(store.tabs);
    void store.activeTabId;
    void store.ui.pane;
    void store.ui.sideOpen;
    void store.ui.splitPx;
    $state.snapshot(store.collapsed);
    if (store.status !== 'ready') return;
    clearTimeout(persistTimer);
    persistTimer = setTimeout(() => store.persist(), 300);
  });
</script>

{#if store.status === 'loading'}
  <div class="center muted">{t('app.connecting')}</div>
{:else if store.status === 'error'}
  <div class="center">
    <p>{t('app.backendError')}</p>
    <button class="primary" onclick={() => store.init()}>{t('app.retry')}</button>
  </div>
{:else}
  <TopBar />
  <div class="app">
    <Rail />
    {#if store.ui.sideOpen}<SidePanel />{/if}
    <section class="main">
      {#if store.tabs.length === 0}
        <EmptyState />
      {:else}
        <TabsBar />
        {#if store.activeTab}
          {@const tab = store.activeTab}
          {#key tab.id}
            {#if tab.kind === 'request'}
              <Workbench {tab} />
            {:else if tab.kind === 'env'}
              <EnvEditor {tab} />
            {:else if tab.kind === 'policy'}
              <PolicyView {tab} />
            {:else}
              <DocsView {tab} />
            {/if}
          {/key}
        {/if}
      {/if}
      <Drawer />
    </section>
  </div>
  {#if store.ui.palette}<Palette />{/if}
  {#if store.ui.settings}<SettingsDialog />{/if}
  {#if store.ui.about}<AboutDialog />{/if}
  <Dialogs />
  <ContextMenu />
  <Toasts />
{/if}

<style>
  .center { flex: 1; display: grid; place-items: center; align-content: center; gap: 12px; }
  .app { flex: 1; display: flex; min-height: 0; }
  .main { flex: 1; display: flex; flex-direction: column; min-width: 0; min-height: 0; }
</style>
