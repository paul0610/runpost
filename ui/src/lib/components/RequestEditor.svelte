<script lang="ts">
  // Constructor de petición (P1): [Método ▾] [URL] [Enviar] [Guardar] [⋯] y Params · Auth · Headers · Body · Scripts · Ajustes.
  import { onMount } from 'svelte';
  import { store, type RequestTab, type SubTab } from '../stores/app.svelte';
  import { t } from '../i18n.svelte';
  import { COMMON_HEADERS, MEDIA_TYPES } from '../kv';
  import { inheritedAuth } from '../stores/requests';
  import { isEnabled } from '../url';
  import UrlInput from './UrlInput.svelte';
  import KeyValueTable from './KeyValueTable.svelte';
  import AuthEditor from './AuthEditor.svelte';
  import BodyEditor from './BodyEditor.svelte';
  import CodeEditor from './CodeEditor.svelte';
  import MarkdownBlock from './MarkdownBlock.svelte';

  let { tab }: { tab: RequestTab } = $props();

  const METHODS = ['GET', 'POST', 'PUT', 'PATCH', 'DELETE', 'HEAD', 'OPTIONS'];
  const draft = $derived(tab.draft);
  const describeVar = $derived(store.describerFor(tab));
  const lookup = $derived(store.lookupFor(tab));
  let urlInput = $state<UrlInput>();
  let scriptTab = $state<'pre' | 'test'>('pre');
  let docsEditing = $state(false);

  const count = (rows: { key: string; enabled?: boolean }[] | undefined) =>
    (rows ?? []).filter((r) => isEnabled(r) && r.key.trim() !== '').length;

  /** Cabeceras que añadirá la auth (propia o heredada), solo para mostrarlas. */
  const authRows = $derived.by(() => {
    let auth = draft.auth;
    let note = t('req.fromAuth');
    if (auth.type === 'inherit') {
      const located = tab.requestId ? store.locateItem(tab.collectionId ?? '', tab.requestId) : null;
      const found = inheritedAuth(store.collection(tab.collectionId), located?.parents ?? []);
      auth = found.auth;
      if (found.from) note = found.from.kind === 'folder' ? t('auth.fromFolder', { name: found.from.name }) : t('auth.fromCollection');
    }
    if (auth.type === 'bearer') return [{ key: 'Authorization', value: `Bearer ${auth.token}`, note }];
    if (auth.type === 'basic') return [{ key: 'Authorization', value: 'Basic …', note }];
    if (auth.type === 'apikey' && auth.in === 'header' && auth.key) return [{ key: auth.key, value: auth.value, note }];
    return [];
  });

  const tabs: { id: SubTab; label: string; badge: () => string }[] = [
    { id: 'params', label: 'req.params', badge: () => (count(draft.params) ? String(count(draft.params)) : '') },
    { id: 'auth', label: 'req.auth', badge: () => '' },
    { id: 'headers', label: 'req.headers', badge: () => (count(draft.headers) ? String(count(draft.headers)) : '') },
    { id: 'body', label: 'req.body', badge: () => (draft.body.mode !== 'none' ? '●' : '') },
    { id: 'scripts', label: 'req.scripts', badge: () => (draft.scripts.pre || draft.scripts.test ? '●' : '') },
    { id: 'settings', label: 'req.settings', badge: () => '' },
    { id: 'docs', label: 'req.docs', badge: () => (draft.description.trim() ? '●' : '') },
  ];

  function moreMenu(event: MouseEvent): void {
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    store.openMenu(rect.right - 220, rect.bottom + 4, [
      {
        label: t('menu.saveAs'),
        action: async () => {
          // Guardar como: se pide destino y la pestaña pasa a apuntar a la copia nueva.
          const previous = tab.requestId;
          tab.requestId = null;
          if (!(await store.saveTab(tab))) tab.requestId = previous;
        },
      },
      { label: t('menu.copyId'), disabled: !tab.requestId, action: () => store.copyId(tab.requestId ?? '') },
      { label: t('menu.copyCurl'), action: () => store.copyCurl(tab.collectionId, { ...$state.snapshot(tab.draft), id: tab.requestId ?? '' }) },
    ]);
  }

  onMount(() => {
    if (!draft.url) urlInput?.focus();
  });

  const triState = (value: boolean | null) => (value === null ? '' : value ? 'yes' : 'no');
  const fromTri = (value: string) => (value === '' ? null : value === 'yes');
</script>

<div class="editor">
  <div class="urlbar">
    <select class="method {`m-${draft.method}`}" bind:value={draft.method} aria-label="method">
      {#each METHODS as method (method)}<option value={method}>{method}</option>{/each}
      {#if !METHODS.includes(draft.method)}<option value={draft.method}>{draft.method}</option>{/if}
    </select>
    <UrlInput
      bind:this={urlInput}
      value={tab.urlText}
      {describeVar}
      placeholder={t('req.urlPlaceholder')}
      label="URL"
      onchange={(text) => store.setUrlText(tab, text)}
      onsubmit={() => store.send(tab)}
    />
    <button class="primary send" disabled={tab.sending || !draft.url.trim()} onclick={() => store.send(tab)}>
      {tab.sending ? t('req.sending') : t('req.send')} <kbd>Ctrl+↵</kbd>
    </button>
    <button onclick={() => store.saveTab(tab)}>{t('req.save')}</button>
    <button
      class="icon ghost snippet"
      class:on={store.ui.code}
      title={t('code.title')}
      aria-label={t('code.button')}
      aria-pressed={store.ui.code}
      onclick={() => (store.ui.code = !store.ui.code)}>&lt;/&gt;</button>
    <button class="icon ghost" title={t('req.more')} aria-label={t('req.more')} onclick={moreMenu}>⋯</button>
  </div>

  <div class="subtabs" role="tablist">
    {#each tabs as item (item.id)}
      {@const badge = item.badge()}
      <button role="tab" aria-selected={tab.subtab === item.id} class:on={tab.subtab === item.id} onclick={() => (tab.subtab = item.id)}>
        {t(item.label)}{#if badge}<span class="n">{badge}</span>{/if}
      </button>
    {/each}
  </div>

  <div class="pane">
    {#if tab.subtab === 'params'}
      <KeyValueTable bind:rows={draft.params} {lookup} onchange={() => store.paramsChanged(tab)} />
    {:else if tab.subtab === 'auth'}
      <AuthEditor {tab} />
    {:else if tab.subtab === 'headers'}
      <KeyValueTable
        bind:rows={draft.headers}
        {lookup}
        keyList="runpost-headers"
        valueList={(key) => (['content-type', 'accept'].includes(key.toLowerCase()) ? 'runpost-media' : undefined)}
        extra={authRows}
      />
      <datalist id="runpost-headers">{#each COMMON_HEADERS as h (h)}<option value={h}></option>{/each}</datalist>
      <datalist id="runpost-media">{#each MEDIA_TYPES as m (m)}<option value={m}></option>{/each}</datalist>
    {:else if tab.subtab === 'body'}
      <BodyEditor {tab} />
    {:else if tab.subtab === 'scripts'}
      <div class="scripts">
        <div class="subtabs small">
          <button class:on={scriptTab === 'pre'} onclick={() => (scriptTab = 'pre')}>{t('scripts.pre')}{#if draft.scripts.pre}<span class="n">●</span>{/if}</button>
          <button class:on={scriptTab === 'test'} onclick={() => (scriptTab = 'test')}>{t('scripts.test')}{#if draft.scripts.test}<span class="n">●</span>{/if}</button>
          <span class="spacer"></span>
          <span class="muted note">{t('scripts.note')}</span>
        </div>
        <div class="code">
          {#key scriptTab}
            <CodeEditor
              value={draft.scripts[scriptTab]}
              lang="javascript"
              label={t(`scripts.${scriptTab}`)}
              onchange={(v) => (draft.scripts[scriptTab] = v)}
            />
          {/key}
        </div>
      </div>
    {:else if tab.subtab === 'settings'}
      {@const http = store.settings?.http}
      <div class="settings">
        <label>
          <span>{t('reqSettings.follow')}</span>
          <select value={triState(draft.settings.follow_redirects)} onchange={(e) => (draft.settings.follow_redirects = fromTri(e.currentTarget.value))}>
            <option value="">{t('reqSettings.inherit', { value: http?.follow_redirects ? t('reqSettings.yes') : t('reqSettings.no') })}</option>
            <option value="yes">{t('reqSettings.yes')}</option>
            <option value="no">{t('reqSettings.no')}</option>
          </select>
        </label>
        <label>
          <span>{t('reqSettings.verify')}</span>
          <select value={triState(draft.settings.verify_tls)} onchange={(e) => (draft.settings.verify_tls = fromTri(e.currentTarget.value))}>
            <option value="">{t('reqSettings.inherit', { value: http?.verify_tls ? t('reqSettings.yes') : t('reqSettings.no') })}</option>
            <option value="yes">{t('reqSettings.yes')}</option>
            <option value="no">{t('reqSettings.no')}</option>
          </select>
        </label>
        {#if (draft.settings.verify_tls ?? http?.verify_tls) === false}
          <p class="warn">⚠ {t('reqSettings.tlsWarning')}</p>
        {/if}
        <label>
          <span>{t('reqSettings.timeout')}</span>
          <input
            type="number"
            min="1"
            placeholder={t('reqSettings.timeoutPlaceholder', { value: http?.timeout_ms ?? 30000 })}
            value={draft.settings.timeout_ms ?? ''}
            oninput={(e) => {
              const n = Number.parseInt(e.currentTarget.value, 10);
              draft.settings.timeout_ms = Number.isFinite(n) && n > 0 ? n : null;
            }}
          />
        </label>
      </div>
    {:else if tab.subtab === 'docs'}
      <div class="docs">
        <div class="docs-bar">
          <span class="muted">{t('docs.requestHint')}</span>
          <span class="spacer"></span>
          <button class="ghost small" onclick={() => (docsEditing = !docsEditing)}>{docsEditing || !draft.description.trim() ? t('docs.preview') : t('docs.edit')}</button>
        </div>
        {#if docsEditing || !draft.description.trim()}
          <textarea class="mono docs-source" bind:value={draft.description} placeholder={t('docs.placeholder')} aria-label={t('req.docs')}></textarea>
        {:else}
          <MarkdownBlock source={draft.description} empty={t('docs.empty')} />
        {/if}
      </div>
    {/if}
  </div>
</div>

<style>
  .editor { display: flex; flex-direction: column; height: 100%; min-height: 0; padding: 10px 12px 0; }
  .urlbar { display: flex; gap: 8px; align-items: center; flex: none; }
  .method { font-weight: 700; width: 104px; }
  .send kbd { color: var(--on-accent); border: none; padding: 0; opacity: .7; margin-left: 4px; font-size: 10px; }
  .snippet { font-family: var(--mono, monospace); font-size: 12px; font-weight: 700; }
  .snippet.on { color: var(--accent); }
  .subtabs { margin-top: 10px; flex: none; }
  .pane { flex: 1; min-height: 0; overflow: auto; padding: 10px 0; }
  .scripts { display: flex; flex-direction: column; height: 100%; gap: 8px; }
  .scripts .subtabs.small { margin-top: 0; align-items: center; }
  .note { font-size: 11px; }
  .code { flex: 1; min-height: 120px; }
  .settings { display: flex; flex-direction: column; gap: 12px; max-width: 420px; }
  .settings label { display: flex; flex-direction: column; gap: 4px; }
  .settings span { color: var(--muted); font-size: 11px; text-transform: uppercase; letter-spacing: .06em; }
  .warn { color: var(--warn); margin: 0; }
  .docs { display: flex; flex-direction: column; gap: 8px; height: 100%; }
  .docs-bar { display: flex; align-items: center; font-size: 12px; }
  .docs-source { flex: 1; min-height: 140px; font-size: 12.5px; line-height: 1.55; }
</style>
