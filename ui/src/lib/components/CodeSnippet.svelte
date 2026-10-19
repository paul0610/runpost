<script lang="ts">
  // Panel "Código" (fase 4), a la derecha como en Postman: snippet de curl, fetch, PowerShell o Python
  // que genera el servidor (/codegen). Las variables normales se sustituyen; los secretos quedan como
  // {{nombre}}. Se regenera al cambiar la petición, el lenguaje o el entorno activo.
  import { store, type RequestTab } from '../stores/app.svelte';
  import { t } from '../i18n.svelte';
  import { copyText } from '../native';
  import type { SnippetLang } from '../types';
  import CodeEditor from './CodeEditor.svelte';

  let { tab }: { tab: RequestTab } = $props();

  const LANGS: { id: SnippetLang; label: string }[] = [
    { id: 'curl', label: 'cURL' },
    { id: 'fetch', label: 'JavaScript – Fetch' },
    { id: 'powershell', label: 'PowerShell – Invoke-RestMethod' },
    { id: 'python', label: 'Python – Requests' },
  ];

  let code = $state('');
  let error = $state('');

  $effect(() => {
    const lang = store.ui.codeLang;
    const draft = $state.snapshot(tab.draft);
    const requestId = tab.requestId;
    const collectionId = tab.collectionId;
    void store.settings?.active_environment;
    let cancelled = false;
    const timer = setTimeout(async () => {
      try {
        const text = await store.snippet(lang, draft, requestId, collectionId);
        if (!cancelled) {
          code = text;
          error = '';
        }
      } catch (e) {
        if (!cancelled) error = e instanceof Error ? e.message : String(e);
      }
    }, 250);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  });

  async function copy(): Promise<void> {
    try {
      await copyText(code);
      store.toast('ok', t('code.copied'));
    } catch {
      store.toast('err', t('code.copyFailed'));
    }
  }
</script>

<aside class="code-panel" aria-label={t('code.title')}>
  <header>
    <b>{t('code.title')}</b>
    <span class="spacer"></span>
    <button class="icon ghost" title={t('code.close')} aria-label={t('code.close')} onclick={() => (store.ui.code = false)}>×</button>
  </header>
  <div class="bar">
    <select bind:value={store.ui.codeLang} aria-label={t('code.language')}>
      {#each LANGS as lang (lang.id)}<option value={lang.id}>{lang.label}</option>{/each}
    </select>
    <button class="small" disabled={!code} onclick={copy}>{t('code.copy')}</button>
  </div>
  {#if error}<p class="err">{error}</p>{/if}
  <div class="snippet">
    <CodeEditor value={code} lang={store.ui.codeLang === 'fetch' ? 'javascript' : 'text'} readonly lineNumbers={false} wrap label={t('code.title')} />
  </div>
  <p class="muted note">{t('code.note')}</p>
</aside>

<style>
  .code-panel { width: 360px; flex: none; display: flex; flex-direction: column; gap: 8px; padding: 10px 12px; border-left: 1px solid var(--border); background: var(--bg); min-height: 0; }
  header { display: flex; align-items: center; gap: 6px; }
  .spacer { flex: 1; }
  .bar { display: flex; gap: 6px; align-items: center; }
  .bar select { flex: 1; min-width: 0; }
  .small { font-size: 12px; }
  .snippet { flex: 1; min-height: 0; }
  .err { color: var(--err); margin: 0; font-size: 12px; }
  .note { margin: 0; font-size: 11px; }
</style>
