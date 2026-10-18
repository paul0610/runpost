<script lang="ts">
  // Diálogos propios de la app (nombre, confirmar, cambios sin guardar, guardar como): no dependen de
  // window.prompt/confirm, que el webview puede bloquear.
  import { tick } from 'svelte';
  import { store } from '../stores/app.svelte';
  import { t } from '../i18n.svelte';
  import type { Folder, Item } from '../types';

  const dialog = $derived(store.dialog);
  let text = $state('');
  let collectionId = $state('');
  let parentId = $state('');
  let strip = $state(true);
  let first = $state<HTMLInputElement | HTMLButtonElement>();

  $effect(() => {
    const current = store.dialog;
    if (!current) return;
    if (current.kind === 'prompt') text = current.value;
    if (current.kind === 'export') strip = current.strip;
    if (current.kind === 'saveAs') {
      text = current.name;
      collectionId = current.collectionId ?? store.collections[0]?.id ?? '';
      parentId = '';
    }
    void tick().then(() => {
      first?.focus();
      if (first instanceof HTMLInputElement) first.select();
    });
  });

  /** Carpetas de la colección elegida, aplanadas con sangría, para "Guardar como". */
  const folders = $derived.by(() => {
    const collection = store.collection(collectionId);
    const out: { id: string; label: string }[] = [];
    const walk = (items: Item[], depth: number) => {
      for (const item of items) {
        if (item.type !== 'folder') continue;
        out.push({ id: item.id, label: `${'  '.repeat(depth)}${item.name}` });
        walk((item as Folder).items, depth + 1);
      }
    };
    walk(collection?.items ?? [], 0);
    return out;
  });

  function cancel(): void {
    const current = store.dialog;
    store.closeDialog();
    if (!current) return;
    if (current.kind === 'prompt') current.resolve(null);
    else if (current.kind === 'confirm') current.resolve(false);
    else if (current.kind === 'unsaved') current.resolve('cancel');
    else if (current.kind === 'text') current.resolve();
    else if (current.kind === 'export') current.resolve(null);
    else current.resolve(null);
  }

  function accept(): void {
    const current = store.dialog;
    if (!current) return;
    if (current.kind === 'prompt') {
      if (!text.trim()) return;
      store.closeDialog();
      current.resolve(text.trim());
    } else if (current.kind === 'confirm') {
      store.closeDialog();
      current.resolve(true);
    } else if (current.kind === 'export') {
      store.closeDialog();
      current.resolve(strip);
    } else if (current.kind === 'saveAs') {
      if (!text.trim() || !collectionId) return;
      store.closeDialog();
      current.resolve({ collectionId, parentId: parentId || null, name: text.trim() });
    }
  }

  function unsaved(choice: 'save' | 'discard'): void {
    const current = store.dialog;
    store.closeDialog();
    if (current?.kind === 'unsaved') current.resolve(choice);
  }

  async function newCollection(): Promise<void> {
    // El diálogo de nombre reemplaza al de "guardar como"; al terminar, se vuelve a él.
    const current = store.dialog;
    if (current?.kind !== 'saveAs') return;
    const pending = { ...current, name: text };
    store.closeDialog();
    const created = await store.createCollection();
    store.dialog = { ...pending, collectionId: created?.id ?? pending.collectionId };
  }

  function key(event: KeyboardEvent): void {
    if (event.key === 'Escape') {
      event.preventDefault();
      cancel();
    } else if (event.key === 'Enter' && !(event.target instanceof HTMLButtonElement)) {
      event.preventDefault();
      accept();
    }
  }
</script>

{#if dialog}
  <div class="overlay" role="presentation" onclick={cancel}>
    <div class="dialog" role="dialog" aria-modal="true" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={key}>
      {#if dialog.kind === 'prompt'}
        <h3>{dialog.title}</h3>
        <label class="field"><span>{t('dialog.name')}</span><input bind:this={first} bind:value={text} /></label>
        <div class="actions">
          <button onclick={cancel}>{t('dialog.cancel')}</button>
          <button class="primary" disabled={!text.trim()} onclick={accept}>{dialog.confirm}</button>
        </div>
      {:else if dialog.kind === 'confirm'}
        <h3>{dialog.title}</h3>
        <p class="muted">{dialog.body}</p>
        <div class="actions">
          <button bind:this={first} onclick={cancel}>{t('dialog.cancel')}</button>
          <button class="primary" class:danger-btn={dialog.danger} onclick={accept}>{dialog.confirm}</button>
        </div>
      {:else if dialog.kind === 'unsaved'}
        <h3>{t('dialog.unsavedTitle')}</h3>
        <p class="muted">{t('dialog.unsavedBody', { name: dialog.name })}</p>
        <div class="actions">
          <button onclick={cancel}>{t('dialog.cancel')}</button>
          <button onclick={() => unsaved('discard')}>{t('dialog.dontSave')}</button>
          <button bind:this={first} class="primary" onclick={() => unsaved('save')}>{t('dialog.save')}</button>
        </div>
      {:else if dialog.kind === 'export'}
        <h3>{dialog.title}</h3>
        <label class="check-line"><input type="checkbox" bind:checked={strip} /> {t('export.strip')}</label>
        <p class="muted small">{strip ? t('export.stripOn') : t('export.stripOff')}</p>
        <div class="actions">
          <button onclick={cancel}>{t('dialog.cancel')}</button>
          <button bind:this={first} class="primary" onclick={accept}>{t('export.confirm')}</button>
        </div>
      {:else if dialog.kind === 'text'}
        <h3>{dialog.title}</h3>
        <p class="muted">{dialog.body}</p>
        <textarea class="mono copy" readonly rows="10" value={dialog.text} onfocus={(e) => e.currentTarget.select()}></textarea>
        <div class="actions">
          <button bind:this={first} class="primary" onclick={cancel}>{t('dialog.close')}</button>
        </div>
      {:else}
        <h3>{t('dialog.saveAsTitle')}</h3>
        <label class="field"><span>{t('dialog.name')}</span><input bind:this={first} bind:value={text} /></label>
        <label class="field">
          <span>{t('dialog.collection')}</span>
          <select
            value={collectionId}
            onchange={(e) => {
              if (e.currentTarget.value === '__new__') void newCollection();
              else {
                collectionId = e.currentTarget.value;
                parentId = '';
              }
            }}
          >
            {#each store.collections as collection (collection.id)}<option value={collection.id}>{collection.name}</option>{/each}
            <option value="__new__">{t('dialog.newCollectionOption')}</option>
          </select>
        </label>
        <label class="field">
          <span>{t('dialog.folder')}</span>
          <select bind:value={parentId} disabled={!collectionId}>
            <option value="">{t('dialog.root')}</option>
            {#each folders as folder (folder.id)}<option value={folder.id}>{folder.label}</option>{/each}
          </select>
        </label>
        <div class="actions">
          <button onclick={cancel}>{t('dialog.cancel')}</button>
          <button class="primary" disabled={!text.trim() || !collectionId} onclick={accept}>{t('dialog.save')}</button>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .overlay { position: fixed; inset: 0; background: var(--overlay); display: grid; place-items: center; z-index: 70; }
  .dialog { width: min(520px, 92vw); background: var(--bg); border: 1px solid var(--border); border-radius: 10px; box-shadow: var(--shadow); padding: 16px 18px; }
  h3 { margin: 0 0 12px; font-size: 15px; }
  p { margin: 0 0 12px; line-height: 1.5; }
  .field { display: flex; flex-direction: column; gap: 4px; margin-bottom: 12px; }
  .field span { color: var(--muted); font-size: 11px; text-transform: uppercase; letter-spacing: .06em; }
  .actions { display: flex; gap: 8px; justify-content: flex-end; margin-top: 6px; }
  .danger-btn { background: var(--err); border-color: var(--err); }
  .copy { width: 100%; font-size: 12px; margin-bottom: 8px; }
  .check-line { display: flex; gap: 8px; align-items: center; margin-bottom: 8px; }
  .small { font-size: 12px; }
</style>
