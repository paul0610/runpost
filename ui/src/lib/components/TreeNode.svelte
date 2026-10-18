<script lang="ts">
  // Una carpeta o petición del árbol, recursivo. Método coloreado, plegado, menú contextual y arrastre.
  import { store } from '../stores/app.svelte';
  import { itemMenu, menuPoint } from '../menus';
  import { DND_TYPE, drag } from '../dnd';
  import { methodClass, methodLabel } from '../format';
  import { t } from '../i18n.svelte';
  import type { Collection, Item } from '../types';
  import TreeNode from './TreeNode.svelte';

  interface Props {
    collection: Collection;
    item: Item;
    depth: number;
    parentId: string | null;
  }

  let { collection, item, depth, parentId }: Props = $props();

  const folder = $derived(item.type === 'folder');
  const open = $derived(!store.isCollapsed(item.id));
  const selected = $derived(store.activeRequestTab?.requestId === item.id);
  let over = $state<'into' | 'before' | null>(null);

  function click(): void {
    if (item.type === 'folder') store.toggleCollapsed(item.id);
    else store.openRequest(collection.id, item.id);
  }

  function menu(event: MouseEvent): void {
    event.preventDefault();
    const { x, y } = menuPoint(event);
    store.openMenu(x, y, itemMenu(collection.id, item));
  }

  function dragStart(event: DragEvent): void {
    drag.current = { collectionId: collection.id, itemId: item.id };
    event.dataTransfer?.setData(DND_TYPE, item.id);
    if (event.dataTransfer) event.dataTransfer.effectAllowed = 'move';
  }

  function dragOver(event: DragEvent): void {
    if (!drag.current || drag.current.itemId === item.id) return;
    event.preventDefault();
    over = folder ? 'into' : 'before';
  }

  function drop(event: DragEvent): void {
    event.preventDefault();
    const source = drag.current;
    over = null;
    drag.current = null;
    if (!source || source.itemId === item.id) return;
    void store.moveItem(source.collectionId, source.itemId, {
      collectionId: collection.id,
      parentId: folder ? item.id : parentId,
      before: folder ? null : item.id,
    });
  }
</script>

<div
  class="row"
  class:sel={selected}
  class:fold={folder}
  class:over-into={over === 'into'}
  class:over-before={over === 'before'}
  style:--d={depth}
  role="treeitem"
  aria-level={depth + 1}
  aria-selected={selected}
  aria-expanded={folder ? open : undefined}
  tabindex={selected ? 0 : -1}
  draggable="true"
  title={item.name}
  onclick={click}
  onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), click())}
  oncontextmenu={menu}
  ondragstart={dragStart}
  ondragover={dragOver}
  ondragleave={() => (over = null)}
  ondrop={drop}
  ondragend={() => ((over = null), (drag.current = null))}
>
  {#if item.type === 'folder'}
    <span class="caret">{open ? '▾' : '▸'}</span>
    <span class="name">{item.name}</span>
  {:else}
    <span class="meth {methodClass(item.method)}">{methodLabel(item.method)}</span>
    <span class="name">{item.name}</span>
    {#if store.requestTabs().some((tab) => tab.requestId === item.id && store.isDirty(tab))}
      <span class="dirty" title={t('tabs.unsaved')}>•</span>
    {/if}
  {/if}
</div>

{#if item.type === 'folder' && open}
  {#each item.items as child (child.id)}
    <TreeNode {collection} item={child} depth={depth + 1} parentId={item.id} />
  {/each}
{/if}

<style>
  .row { padding: 4px 10px 4px calc(10px + var(--d, 0) * 14px); display: flex; gap: 8px; align-items: center; cursor: pointer; white-space: nowrap; user-select: none; }
  .row:hover { background: var(--bg-3); }
  .row:focus-visible { outline: 2px solid var(--focus); outline-offset: -2px; }
  .row.sel { background: var(--bg-3); box-shadow: inset 2px 0 0 var(--accent); }
  .row.fold { color: var(--muted); }
  .row.over-into { background: var(--selection); }
  .row.over-before { box-shadow: inset 0 2px 0 var(--accent); }
  .caret { width: 10px; }
  .meth { width: 44px; flex: none; font-weight: 700; font-size: 10px; text-align: right; }
  .name { overflow: hidden; text-overflow: ellipsis; }
  .dirty { color: var(--accent-text); }
</style>
