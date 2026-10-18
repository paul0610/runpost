<script lang="ts">
  // Visor de respuesta (P1): estado · ms · tamaño, Body (Pretty/Raw/Preview), Headers, Cookies, Tests;
  // buscar, copiar y guardar a archivo.
  import { store, type BodyView, type RequestTab, type RespTab } from '../stores/app.svelte';
  import { locale, t } from '../i18n.svelte';
  import { extensionFor, formatBytes, prettyBody, statusClass } from '../format';
  import { api, isTauri } from '../transport';
  import { copyText, downloadText, pickSavePath } from '../native';
  import CodeEditor from './CodeEditor.svelte';

  let { tab }: { tab: RequestTab } = $props();
  let editor = $state<CodeEditor>();

  const result = $derived(tab.result);
  const response = $derived(result?.response ?? null);
  const contentType = $derived(
    response?.headers.find(([k]) => k.toLowerCase() === 'content-type')?.[1] ?? '',
  );
  const binary = $derived(response?.body_encoding === 'base64');
  const pretty = $derived(response && !binary ? prettyBody(response.body, contentType) : null);
  const wrap = $derived(store.settings?.ui.wrap_response ?? true);
  const tests = $derived(result?.tests ?? []);
  // Los tests omitidos (`pm.test.skip`) no cuentan ni como pasados ni como fallidos.
  const counted = $derived(tests.filter((x) => !x.skipped));
  const passed = $derived(counted.filter((x) => x.passed).length);

  const respTabs: { id: RespTab; label: string }[] = [
    { id: 'body', label: 'res.body' },
    { id: 'headers', label: 'res.headers' },
    { id: 'cookies', label: 'res.cookies' },
    { id: 'tests', label: 'res.tests' },
  ];
  const views: BodyView[] = ['pretty', 'raw', 'preview'];

  async function copyBody(): Promise<void> {
    if (!response) return;
    try {
      await copyText(response.body);
      store.toast('ok', t('res.copied'));
    } catch {
      store.toast('err', t('res.copyFailed'));
    }
  }

  async function saveBody(): Promise<void> {
    if (!response || !result) return;
    const name = `response.${extensionFor(contentType)}`;
    if (!isTauri()) {
      downloadText(name, response.body, contentType || 'text/plain');
      return;
    }
    const path = await pickSavePath(name);
    if (!path) return;
    try {
      const saved = await api.post<{ path: string }>('/files/save', { path, history_id: result.history_id });
      store.toast('ok', t('res.saved', { path: saved.path }));
    } catch (error) {
      store.toastError(error);
    }
  }

  const errorTitle = (code: string) => {
    const key = `errors.${code}`;
    const text = t(key);
    return text === key ? t('errors.generic') : text;
  };
</script>

{#snippet testList()}
  <ul class="tests">
    {#each tests as test, index (index)}
      <li class:ok={test.passed && !test.skipped} class:bad={!test.passed && !test.skipped} class:skip={test.skipped}>
        <span class="mark">{test.skipped ? '–' : test.passed ? '✔' : '✘'}</span>
        <span class="name">{test.name}</span>
        {#if test.skipped}<span class="pill">{t('res.testSkipped')}</span>{/if}
        {#if !test.passed && !test.skipped && test.msg}<div class="msg mono">{test.msg}</div>{/if}
      </li>
    {/each}
  </ul>
{/snippet}

<div class="resp">
  {#if tab.sending}
    <div class="placeholder muted">{t('res.sending')}</div>
  {:else if tab.error}
    <div class="error">
      <b>{errorTitle(tab.error.code)}</b>
      <p class="mono">{tab.error.message}</p>
      {#if tab.error.url}<p class="mono muted">{tab.error.url}</p>{/if}
      {#if tab.error.historyId}<p class="muted">{t('errors.historyHint')}</p>{/if}
    </div>
  {:else if result?.skipped}
    <div class="skipped">
      <p><b>↷ {t('res.skipped')}</b></p>
      {#if tests.length}{@render testList()}{/if}
    </div>
  {:else if !response}
    <div class="placeholder muted">{t('res.empty')} <kbd>{t('res.emptyHint')}</kbd></div>
  {:else}
    <div class="status">
      <b class="code {statusClass(response.status)}">{response.status} {response.status_text}</b>
      <span class="pill">{response.ms} ms</span>
      <span class="pill">{formatBytes(response.size, locale.lang)}</span>
      {#if result?.resolved.unresolved.length}
        <span class="pill warn" title={result.resolved.url}>{t('res.unresolved', { names: result.resolved.unresolved.join(', ') })}</span>
      {/if}
      <div class="subtabs inline" role="tablist">
        {#each respTabs as item (item.id)}
          <button role="tab" aria-selected={tab.respTab === item.id} class:on={tab.respTab === item.id} onclick={() => (tab.respTab = item.id)}>
            {t(item.label)}
            {#if item.id === 'headers'}<span class="n">{response.headers.length}</span>{/if}
            {#if item.id === 'cookies' && response.cookies.length}<span class="n">{response.cookies.length}</span>{/if}
            {#if item.id === 'tests' && counted.length}<span class="n" class:bad={passed < counted.length}>{passed}/{counted.length}</span>{/if}
          </button>
        {/each}
      </div>
      {#if tab.respTab === 'body'}
        <div class="tools">
          {#each views as view (view)}
            <button class="pill-btn" class:on={tab.bodyView === view} onclick={() => (tab.bodyView = view)}>{t(`res.${view}`)}</button>
          {/each}
          <button class="icon ghost" title={t('res.search')} aria-label={t('res.search')} disabled={tab.bodyView === 'preview' || binary} onclick={() => editor?.openSearch()}>🔍</button>
          <button class="icon ghost" title={t('res.copy')} aria-label={t('res.copy')} onclick={copyBody}>⧉</button>
          <button class="icon ghost" title={t('res.save')} aria-label={t('res.save')} onclick={saveBody}>💾</button>
          <button class="ghost small" disabled title={t('res.snapshotSoon')}>{t('res.snapshot')}</button>
        </div>
      {/if}
    </div>

    <div class="content">
      {#if tab.respTab === 'body'}
        {#if binary}
          <p class="muted pad">{t('res.binary', { size: formatBytes(response.size, locale.lang) })}</p>
        {:else if tab.bodyView === 'pretty' && pretty}
          <CodeEditor bind:this={editor} value={pretty.text} lang={pretty.lang} readonly fold {wrap} label={t('res.body')} />
        {:else if tab.bodyView === 'raw'}
          <CodeEditor bind:this={editor} value={response.body} readonly {wrap} label={t('res.body')} />
        {:else if contentType.toLowerCase().includes('html')}
          <!-- Sin allow-scripts ni allow-same-origin: el HTML de la respuesta no puede ejecutar nada. -->
          <iframe class="preview" title={t('res.preview')} sandbox="" srcdoc={response.body}></iframe>
        {:else}
          <p class="muted pad">{t('res.previewText')}</p>
          <pre class="mono raw">{response.body}</pre>
        {/if}
      {:else if tab.respTab === 'headers'}
        <table class="kv mono">
          <thead><tr><th>{t('res.name')}</th><th>{t('res.value')}</th></tr></thead>
          <tbody>
            {#each response.headers as [name, value], index (index)}
              <tr><td class="hname">{name}</td><td class="hval">{value}</td></tr>
            {/each}
          </tbody>
        </table>
      {:else if tab.respTab === 'cookies'}
        {#if response.cookies.length}
          <table class="kv mono">
            <thead><tr><th>{t('res.name')}</th><th>{t('res.value')}</th><th>{t('res.domain')}</th><th>{t('res.path')}</th><th>{t('res.flags')}</th></tr></thead>
            <tbody>
              {#each response.cookies as cookie, index (index)}
                <tr>
                  <td>{cookie.name}</td><td class="hval">{cookie.value}</td><td>{cookie.domain ?? ''}</td><td>{cookie.path ?? ''}</td>
                  <td>{[cookie.secure ? 'Secure' : '', cookie.http_only ? 'HttpOnly' : ''].filter(Boolean).join(' · ')}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        {:else}
          <p class="muted pad">{t('res.noCookies')}</p>
        {/if}
      {:else if tests.length}
        {@render testList()}
      {:else}
        <p class="muted pad">{t('res.noTests')}</p>
      {/if}
    </div>
  {/if}
</div>

<style>
  .resp { flex: 1; min-height: 0; display: flex; flex-direction: column; }
  .placeholder { flex: 1; display: grid; place-items: center; }
  .placeholder kbd { margin-left: 6px; }
  .error { padding: 16px; border-left: 4px solid var(--err); margin: 12px; background: var(--bg-2); border-radius: 6px; }
  .error b { color: var(--err); }
  .error p { margin: 6px 0 0; word-break: break-all; }
  .status { display: flex; gap: 12px; align-items: center; padding: 6px 12px; border-bottom: 1px solid var(--border); flex: none; flex-wrap: wrap; }
  .code.ok { color: var(--ok); }
  .code.info { color: var(--info); }
  .code.err { color: var(--err); }
  .pill.warn { color: var(--warn); }
  .subtabs.inline { border: none; }
  .subtabs .n.bad { color: var(--err); }
  .tools { display: flex; gap: 6px; margin-left: auto; align-items: center; }
  .pill-btn { border: none; border-radius: 999px; padding: 1px 9px; font-size: 11px; color: var(--muted); }
  .pill-btn.on { background: var(--bg-3); color: var(--fg); }
  .content { flex: 1; min-height: 0; overflow: auto; padding: 10px 12px; display: flex; flex-direction: column; }
  .content :global(.editor) { flex: 1; }
  .preview { flex: 1; width: 100%; border: 1px solid var(--border); border-radius: 6px; background: var(--bg); }
  .pad { margin: 4px 0; }
  .raw { white-space: pre-wrap; word-break: break-all; margin: 0; }
  .hname { width: 32%; color: var(--info); }
  .hval { word-break: break-all; }
  .tests { list-style: none; margin: 0; padding: 0; }
  .tests li { padding: 5px 0; border-bottom: 1px solid var(--border); }
  .tests .mark { display: inline-block; width: 18px; }
  .tests li.ok .mark { color: var(--ok); }
  .tests li.bad .mark, .tests li.bad .name { color: var(--err); }
  .tests li.skip { color: var(--muted); }
  .tests .msg { margin: 3px 0 0 18px; color: var(--fg); white-space: pre-wrap; word-break: break-word; }
  .skipped { padding: 12px 16px; overflow: auto; }
  .skipped p { margin: 0 0 8px; }
</style>
