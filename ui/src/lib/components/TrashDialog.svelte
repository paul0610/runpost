<script lang="ts">
  // Papelera (fase 4): colecciones, carpetas, peticiones y entornos borrados en los últimos 30 días,
  // para devolverlos a su sitio o borrarlos para siempre.
  import { store } from '../stores/app.svelte';
  import { locale, t } from '../i18n.svelte';
  import type { TrashItem } from '../types';

  let items = $state<TrashItem[]>([]);
  let loaded = $state(false);

  $effect(() => {
    void refresh();
  });

  async function refresh(): Promise<void> {
    try {
      items = await store.listTrash();
    } catch (e) {
      store.toastError(e);
    } finally {
      loaded = true;
    }
  }

  function close(): void {
    store.ui.trashOpen = false;
  }

  function key(event: KeyboardEvent): void {
    // Con un diálogo de confirmación encima, Escape solo cierra ese.
    if (event.key === 'Escape' && !event.defaultPrevented && !store.dialog) close();
  }

  async function restore(item: TrashItem): Promise<void> {
    if (await store.restoreTrash(item.id)) await refresh();
  }

  async function purge(item: TrashItem): Promise<void> {
    if (!(await store.askConfirm(t('trash.purgeTitle', { name: item.name }), t('trash.purgeBody'), t('trash.purge'), true))) return;
    try {
      await store.purgeTrash(item.id);
      await refresh();
    } catch (e) {
      store.toastError(e);
    }
  }

  async function empty(): Promise<void> {
    if (!(await store.askConfirm(t('trash.emptyTitle'), t('trash.emptyBody'), t('trash.empty'), true))) return;
    try {
      await store.purgeTrash();
      await refresh();
    } catch (e) {
      store.toastError(e);
    }
  }

  const when = (ms: number) => new Date(ms).toLocaleString(locale.lang, { dateStyle: 'short', timeStyle: 'short' });
  const icon = (kind: TrashItem['kind']) => ({ collection: '🗂', folder: '📁', request: '↗', environment: '🌐' })[kind];
</script>

<svelte:window onkeydown={key} />

<div class="overlay" role="presentation" onclick={close}>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="trash-title" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={() => undefined}>
    <header>
      <h2 id="trash-title">{t('trash.title')}</h2>
      <span class="spacer"></span>
      <button class="ghost" onclick={close}>{t('trash.close')}</button>
    </header>
    <p class="muted small">{t('trash.hint')}</p>
    <div class="list">
      {#if loaded && !items.length}<p class="muted empty">{t('trash.nothing')}</p>{/if}
      {#each items as item (item.id)}
        <div class="row">
          <span class="icon" aria-hidden="true">{icon(item.kind)}</span>
          <div class="what">
            <b>{item.name}</b>
            <span class="muted small">
              {t(`trash.kind.${item.kind}`)}{item.location ? ` · ${item.location}` : ''}{item.kind !== 'request' && item.requests
                ? ` · ${t('trash.requests', { count: item.requests })}`
                : ''}
            </span>
          </div>
          <span class="muted small when">{when(item.deleted_at)}</span>
          <button class="small" onclick={() => restore(item)}>{t('trash.restore')}</button>
          <button class="ghost small danger" onclick={() => purge(item)}>{t('trash.purge')}</button>
        </div>
      {/each}
    </div>
    <div class="actions">
      <button class="ghost danger" disabled={!items.length} onclick={empty}>{t('trash.empty')}</button>
    </div>
  </div>
</div>

<style>
  .overlay { position: fixed; inset: 0; background: var(--overlay); display: grid; place-items: center; z-index: 60; }
  .dialog { width: min(640px, 94vw); max-height: 86vh; display: flex; flex-direction: column; gap: 10px; background: var(--bg); border: 1px solid var(--border); border-radius: 10px; box-shadow: var(--shadow); padding: 16px 20px 18px; }
  header { display: flex; align-items: center; }
  h2 { margin: 0; font-size: 16px; }
  .spacer { flex: 1; }
  .small { font-size: 12px; }
  p.small { margin: 0; line-height: 1.5; }
  .list { flex: 1; min-height: 100px; overflow: auto; border: 1px solid var(--border); border-radius: 8px; }
  .empty { margin: 14px; }
  .row { display: flex; align-items: center; gap: 10px; padding: 8px 12px; }
  .row + .row { border-top: 1px solid var(--border); }
  .icon { width: 20px; text-align: center; }
  .what { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .what b, .what span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .when { white-space: nowrap; flex: none; }
  .danger { color: var(--err); }
  .actions { display: flex; justify-content: flex-end; }
</style>
