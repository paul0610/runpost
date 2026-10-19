<script lang="ts">
  // Ejemplos guardados de una petición (fase 4), como en Postman: respuestas reales que se guardan
  // desde el visor de respuesta. Salen del historial, así que ya vienen enmascaradas.
  import { store, type RequestTab } from '../stores/app.svelte';
  import { t } from '../i18n.svelte';
  import { prettyBody, statusClass } from '../format';
  import { findRequest } from '../stores/requests';
  import CodeEditor from './CodeEditor.svelte';

  let { tab }: { tab: RequestTab } = $props();

  let selectedId = $state<string | null>(null);
  let view = $state<'body' | 'headers'>('body');

  // Los ejemplos no van en el borrador: se leen de la colección guardada.
  const request = $derived.by(() => {
    const collection = store.collection(tab.collectionId);
    return collection && tab.requestId ? findRequest(collection, tab.requestId) : null;
  });
  const examples = $derived(request?.examples ?? []);
  const selected = $derived(examples.find((e) => e.id === selectedId) ?? examples[0] ?? null);
  const contentType = $derived(selected?.headers?.find((h) => h.key.toLowerCase() === 'content-type')?.value ?? '');
  const pretty = $derived(selected ? prettyBody(selected.body, contentType) : null);

  async function rename(): Promise<void> {
    if (!selected || !tab.collectionId || !tab.requestId) return;
    const name = await store.askPrompt(t('examples.renameTitle', { name: selected.name }), selected.name, t('dialog.rename'));
    if (name?.trim()) await store.renameExample(tab.collectionId, tab.requestId, selected.id, name.trim());
  }

  async function remove(): Promise<void> {
    if (!selected || !tab.collectionId || !tab.requestId) return;
    if (!(await store.askConfirm(t('examples.deleteTitle', { name: selected.name }), t('examples.deleteBody'), t('dialog.delete'), true))) return;
    await store.deleteExample(tab.collectionId, tab.requestId, selected.id);
    selectedId = null;
  }
</script>

{#if !tab.requestId}
  <p class="muted hint">{t('examples.unsaved')}</p>
{:else if !examples.length}
  <p class="muted hint">{t('examples.empty')}</p>
{:else}
  <div class="examples">
    <ul class="list" aria-label={t('req.examples')}>
      {#each examples as example (example.id)}
        <li>
          <button class:on={selected?.id === example.id} onclick={() => (selectedId = example.id)}>
            <b class="code {statusClass(example.status)}">{example.status}</b>
            <span class="name">{example.name}</span>
          </button>
        </li>
      {/each}
    </ul>
    {#if selected}
      <div class="viewer">
        <div class="head">
          <b class="code {statusClass(selected.status)}">{selected.status} {selected.status_text}</b>
          <span class="mono muted url" title={selected.request.url}>{selected.request.method} {selected.request.url}</span>
          <span class="spacer"></span>
          <button class="ghost small" onclick={rename}>{t('examples.rename')}</button>
          <button class="ghost small danger" onclick={remove}>{t('examples.delete')}</button>
        </div>
        <div class="subtabs small" role="tablist">
          <button role="tab" aria-selected={view === 'body'} class:on={view === 'body'} onclick={() => (view = 'body')}>{t('res.body')}</button>
          <button role="tab" aria-selected={view === 'headers'} class:on={view === 'headers'} onclick={() => (view = 'headers')}>
            {t('res.headers')}<span class="n">{selected.headers?.length ?? 0}</span>
          </button>
        </div>
        <div class="content">
          {#if view === 'body'}
            {#if selected.body}
              <CodeEditor value={pretty?.text ?? selected.body} lang={pretty?.lang ?? 'text'} readonly fold label={t('res.body')} />
            {:else}
              <p class="muted">{t('examples.noBody')}</p>
            {/if}
          {:else}
            <table class="kv mono">
              <thead><tr><th>{t('res.name')}</th><th>{t('res.value')}</th></tr></thead>
              <tbody>
                {#each selected.headers ?? [] as header, index (index)}
                  <tr><td class="hname">{header.key}</td><td class="hval">{header.value}</td></tr>
                {/each}
              </tbody>
            </table>
          {/if}
        </div>
      </div>
    {/if}
  </div>
{/if}

<style>
  .hint { margin: 6px 0; }
  .examples { display: grid; grid-template-columns: minmax(150px, 220px) minmax(0, 1fr); gap: 12px; height: 100%; min-height: 160px; }
  .list { list-style: none; margin: 0; padding: 0; overflow: auto; border-right: 1px solid var(--border); padding-right: 8px; }
  .list button { width: 100%; display: flex; gap: 8px; align-items: center; border: none; background: none; padding: 5px 8px; border-radius: 6px; text-align: left; }
  .list button.on { background: var(--bg-3); }
  .list .name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .code { font-size: 12px; }
  .code.ok { color: var(--ok); }
  .code.info { color: var(--info); }
  .code.err { color: var(--err); }
  .viewer { display: flex; flex-direction: column; gap: 6px; min-width: 0; min-height: 0; }
  .head { display: flex; align-items: center; gap: 10px; }
  .head .code { white-space: nowrap; flex: none; }
  .url { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12px; }
  .spacer { flex: 1; }
  .danger { color: var(--err); }
  .content { flex: 1; min-height: 120px; display: flex; flex-direction: column; overflow: auto; }
  .content :global(.editor) { flex: 1; }
  .hname { width: 32%; color: var(--info); }
  .hval { word-break: break-all; }
</style>
