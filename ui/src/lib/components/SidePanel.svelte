<script lang="ts">
  // Panel desplegable de 290 px (UX decisión 2) con el contenido de la sección elegida en la barra lateral.
  import { GLOBALS, store } from '../stores/app.svelte';
  import { t } from '../i18n.svelte';
  import { collectionMenu, environmentMenu, historyMenu, menuPoint } from '../menus';
  import { DND_TYPE, drag } from '../dnd';
  import { openImport } from '../importer';
  import { methodClass, methodLabel, statusClass } from '../format';
  import TreeNode from './TreeNode.svelte';
  import AgentPanel from './AgentPanel.svelte';
  import type { Collection } from '../types';

  let overCollection = $state<string | null>(null);
  let filterTimer: ReturnType<typeof setTimeout> | undefined;
  let tree = $state<HTMLDivElement>();

  function menuAt(event: MouseEvent, items: Parameters<typeof store.openMenu>[2]): void {
    event.preventDefault();
    const { x, y } = menuPoint(event);
    store.openMenu(x, y, items);
  }

  function dropOnCollection(event: DragEvent, collection: Collection): void {
    event.preventDefault();
    overCollection = null;
    const source = drag.current;
    drag.current = null;
    if (source) void store.moveItem(source.collectionId, source.itemId, { collectionId: collection.id, parentId: null, before: null });
  }

  function filterHistory(): void {
    clearTimeout(filterTimer);
    filterTimer = setTimeout(() => void store.reloadHistory(), 250);
  }

  /** `https://host/api/v2/ping?x` → `…/api/v2/ping`; las rutas largas, por el final (`…/sign`), como el mockup. */
  function shortUrl(url: string): string {
    const noQuery = url.split('?')[0];
    const match = noQuery.match(/^[a-z]+:\/\/[^/]+(\/.*)?$/i);
    const parts = (match ? (match[1] ?? '') : noQuery).split('/').filter(Boolean);
    let tail = '';
    for (let i = parts.length - 1; i >= 0; i -= 1) {
      const next = `/${parts[i]}${tail}`;
      if (tail && next.length > 22) break;
      tail = next;
    }
    return `…${tail || '/'}`;
  }

  /** Teclado del árbol (patrón ARIA tree): ↑ ↓ mueven, → abre/entra, ← cierra/sube, Inicio/Fin. */
  function treeKey(event: KeyboardEvent): void {
    if (!tree) return;
    const rows = [...tree.querySelectorAll<HTMLElement>('[role="treeitem"]')];
    const index = rows.indexOf(document.activeElement as HTMLElement);
    if (index < 0) return;
    const row = rows[index];
    const expanded = row.getAttribute('aria-expanded');
    const level = Number(row.getAttribute('aria-level') ?? '1');
    switch (event.key) {
      case 'ArrowDown':
        rows[index + 1]?.focus();
        break;
      case 'ArrowUp':
        rows[index - 1]?.focus();
        break;
      case 'Home':
        rows[0]?.focus();
        break;
      case 'End':
        rows.at(-1)?.focus();
        break;
      case 'ArrowRight':
        if (expanded === 'false') row.click();
        else if (expanded === 'true') rows[index + 1]?.focus();
        break;
      case 'ArrowLeft':
        if (expanded === 'true') row.click();
        else rows.slice(0, index).reverse().find((r) => Number(r.getAttribute('aria-level') ?? '1') < level)?.focus();
        break;
      default:
        return;
    }
    event.preventDefault();
  }

  const activeEnvId = $derived(store.activeEnvironment?.id ?? null);
</script>

{#snippet trashButton()}
  <footer>
    <button class="ghost foot" title={t('trash.openTitle')} onclick={() => (store.ui.trashOpen = true)}>🗑 {t('trash.open')}</button>
  </footer>
{/snippet}

<aside class="side">
  {#if store.ui.pane === 'collections'}
    <header>
      {t('rail.collections')}
      <span class="spacer"></span>
      <button class="ghost" title={t('side.importTitle')} onclick={openImport}>{t('side.import')}</button>
      <button class="ghost" title={t('side.newCollection')} aria-label={t('side.newCollection')} onclick={() => store.createCollection()}>＋</button>
    </header>
    <div class="body tree" role="tree" aria-label={t('rail.collections')} bind:this={tree} onkeydown={treeKey} tabindex="-1">
      {#each store.collections as collection, index (collection.id)}
        {@const open = !store.isCollapsed(collection.id)}
        <div
          class="row root"
          class:over={overCollection === collection.id}
          role="treeitem"
          aria-level="1"
          aria-expanded={open}
          aria-selected="false"
          tabindex={index === 0 ? 0 : -1}
          title={collection.name}
          onclick={() => store.toggleCollapsed(collection.id)}
          onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), store.toggleCollapsed(collection.id))}
          oncontextmenu={(e) => menuAt(e, collectionMenu(collection))}
          ondragover={(e) => {
            if (drag.current && e.dataTransfer?.types.includes(DND_TYPE)) {
              e.preventDefault();
              overCollection = collection.id;
            }
          }}
          ondragleave={() => (overCollection = null)}
          ondrop={(e) => dropOnCollection(e, collection)}
        >
          <span class="caret">{open ? '▾' : '▸'}</span>
          <b class="name">{collection.name}</b>
          {#if collection.agent_policy}<span class="pill" title={t('side.policyTitle')}>{t('side.policy')}</span>{/if}
        </div>
        {#if open}
          {#each collection.items as item (item.id)}
            <TreeNode {collection} {item} depth={1} parentId={null} />
          {/each}
        {/if}
      {:else}
        <p class="hint">{t('side.noCollections')}</p>
      {/each}
    </div>
    {@render trashButton()}
  {:else if store.ui.pane === 'environments'}
    <header>
      {t('rail.environments')}
      <span class="spacer"></span>
      <button class="ghost" title={t('side.newEnvironment')} aria-label={t('side.newEnvironment')} onclick={() => store.createEnvironment()}>＋</button>
    </header>
    <div class="body tree">
      <button class="row env" title={t('env.openGlobals')} onclick={() => store.openEnvironment(GLOBALS)}>
        <span class="radio">🌐</span> <span class="name">{t('side.globals')}</span>
        <span class="spacer"></span><span class="pill">{store.globals.variables.length}</span>
      </button>
      {#each store.environments as env (env.id)}
        {@const active = activeEnvId === env.id}
        <div class="row env-row" class:sel={active} role="group" oncontextmenu={(e) => menuAt(e, environmentMenu(env))}>
          <button class="radio ghost" title={active ? t('env.deactivate') : t('side.activate')} aria-label={active ? t('env.deactivate') : t('side.activate')} aria-pressed={active} onclick={() => store.setActiveEnvironment(active ? null : env.id)}>
            {active ? '●' : '○'}
          </button>
          <button class="open ghost" title={t('env.open')} onclick={() => store.openEnvironment(env.id)}>
            <span class="name">{env.name}</span>
            {#if active}<span class="pill">{t('side.active')}</span>{/if}
          </button>
          <span class="spacer"></span><span class="pill">{env.variables.length}</span>
        </div>
      {:else}
        <p class="hint">{t('side.noEnvironments')}</p>
      {/each}
      <p class="hint">{t('env.sideHint')}</p>
    </div>
    {@render trashButton()}
  {:else if store.ui.pane === 'history'}
    <header>
      {t('rail.history')}
      <span class="spacer"></span>
      <input placeholder={t('side.filter')} aria-label={t('side.filter')} bind:value={store.historyFilter} oninput={filterHistory} />
      <button class="ghost icon-btn" title={t('history.clear')} aria-label={t('history.clear')} disabled={!store.history.length} onclick={() => store.clearHistory()}>🗑</button>
    </header>
    <div class="body tree">
      {#each store.history as row (row.id)}
        <button class="row" title={`${row.url}\n${t('history.openHint')}`} onclick={() => store.openHistory(row.id)} oncontextmenu={(e) => menuAt(e, historyMenu(row))}>
          <span class="meth {methodClass(row.method)}">{methodLabel(row.method)}</span>
          <span class="name">{shortUrl(row.url)}</span>
          {#if row.status !== null}
            <span class="pill code-{statusClass(row.status)}">{row.status}</span>
          {:else}
            <span class="pill code-err">{(row.error ?? 'error').split(':')[0]}</span>
          {/if}
          <span class="pill">{row.duration_ms} ms</span>
          {#if row.source === 'mcp'}<span class="pill" title={t('side.agentCall')}>🤖</span>{/if}
        </button>
      {:else}
        <p class="hint">{t('side.noHistory')}</p>
      {/each}
    </div>
  {:else}
    <AgentPanel />
  {/if}
</aside>

<style>
  .side { width: 290px; flex: none; border-right: 1px solid var(--border); background: var(--bg-2); display: flex; flex-direction: column; min-height: 0; }
  header { display: flex; align-items: center; gap: 6px; padding: 8px 10px; border-bottom: 1px solid var(--border); font-weight: 600; min-height: 41px; }
  header input { width: 120px; font-weight: normal; padding: 3px 8px; }
  .icon-btn { padding: 2px 6px; }
  .body { overflow: auto; padding: 6px 0; flex: 1; }
  .tree:focus { outline: none; }
  .row { padding: 4px 10px; display: flex; gap: 8px; align-items: center; cursor: pointer; white-space: nowrap; user-select: none; width: 100%; }
  .row:hover { background: var(--bg-3); }
  .row.root { margin-top: 2px; }
  .row.root.over { background: var(--selection); }
  .row:focus-visible { outline: 2px solid var(--focus); outline-offset: -2px; }
  button.row { border: none; border-radius: 0; text-align: left; }
  .row.sel { background: var(--bg-3); box-shadow: inset 2px 0 0 var(--accent); }
  .env-row { gap: 4px; padding: 2px 10px; }
  .env-row button { border: none; padding: 2px 4px; display: flex; gap: 8px; align-items: center; min-width: 0; }
  .radio { width: 22px; flex: none; text-align: center; }
  .open { overflow: hidden; }
  .caret { width: 10px; color: var(--muted); }
  .name { overflow: hidden; text-overflow: ellipsis; }
  .meth { width: 44px; flex: none; font-weight: 700; font-size: 10px; text-align: right; }
  .hint { color: var(--muted); padding: 6px 12px; margin: 0; font-size: 12px; line-height: 1.5; white-space: normal; }
  .code-ok { color: var(--ok); }
  .code-info { color: var(--info); }
  .code-err { color: var(--err); }
  footer { flex: none; border-top: 1px solid var(--border); padding: 4px 6px; }
  .foot { font-size: 12px; color: var(--muted); padding: 3px 8px; }
  .foot:hover { color: var(--fg); }
</style>
