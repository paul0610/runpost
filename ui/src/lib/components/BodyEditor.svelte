<script lang="ts">
  // Body (sección 5.3): none / form-data / x-www-form-urlencoded / raw (CodeMirror + Beautify) / binary.
  import { store, type RequestTab } from '../stores/app.svelte';
  import { t } from '../i18n.svelte';
  import { beautifyJson } from '../format';
  import { isTauri } from '../transport';
  import { pickFiles } from '../native';
  import type { Body, RawLang } from '../types';
  import CodeEditor from './CodeEditor.svelte';
  import KeyValueTable from './KeyValueTable.svelte';

  let { tab }: { tab: RequestTab } = $props();

  const body = $derived(tab.draft.body);
  const describeVar = $derived(store.describerFor(tab));
  const modes: Body['mode'][] = ['none', 'multipart', 'urlencoded', 'raw', 'binary'];
  const langs: RawLang[] = ['json', 'text', 'xml', 'html', 'javascript'];
  const langLabel: Record<RawLang, string> = { json: 'JSON', text: 'Text', xml: 'XML', html: 'HTML', javascript: 'JavaScript' };

  function setMode(mode: Body['mode']): void {
    const current = tab.draft.body;
    if (current.mode === mode) return;
    const text = current.mode === 'raw' ? current.text : '';
    tab.draft.body =
      mode === 'raw'
        ? { mode, lang: 'json', text }
        : mode === 'urlencoded'
          ? { mode, fields: [] }
          : mode === 'multipart'
            ? { mode, fields: [] }
            : mode === 'binary'
              ? { mode, path: '' }
              : { mode: 'none' };
  }

  function beautify(): void {
    if (body.mode !== 'raw') return;
    const pretty = beautifyJson(body.text);
    if (pretty === null) store.toast('err', t('body.beautifyFailed'));
    else body.text = pretty;
  }

  async function choose(apply: (path: string) => void): Promise<void> {
    const picked = await pickFiles([], false);
    if (picked?.kind === 'paths') apply(picked.paths[0]);
  }
</script>

<div class="body-editor">
  <div class="modes">
    {#each modes as mode (mode)}
      <label><input type="radio" name="mode-{tab.id}" checked={body.mode === mode} onchange={() => setMode(mode)} /> {t(`body.${mode}`)}</label>
    {/each}
    {#if body.mode === 'graphql'}<label><input type="radio" checked disabled /> {t('body.graphql')}</label>{/if}
    {#if body.mode === 'raw'}
      <select class="lang" bind:value={body.lang} aria-label="lang">
        {#each langs as lang (lang)}<option value={lang}>{langLabel[lang]}</option>{/each}
      </select>
      {#if body.lang === 'json'}<button class="ghost small" onclick={beautify}>{t('body.beautify')}</button>{/if}
    {/if}
  </div>

  {#if body.mode === 'none'}
    <p class="muted">{t('body.noBody')}</p>
  {:else if body.mode === 'raw'}
    <div class="code">
      <CodeEditor value={body.text} lang={body.lang} {describeVar} label={t('req.body')} onchange={(v) => (body.text = v)} />
    </div>
  {:else if body.mode === 'urlencoded'}
    <KeyValueTable bind:rows={body.fields} lookup={store.lookupFor(tab)} />
  {:else if body.mode === 'multipart'}
    <table class="kv">
      <thead><tr><th class="check"></th><th>{t('req.key')}</th><th>{t('body.kind')}</th><th>{t('req.value')}</th><th class="del"></th></tr></thead>
      <tbody>
        {#each body.fields as field, index (index)}
          <tr>
            <td class="check"><input type="checkbox" checked={field.enabled !== false} onchange={(e) => (field.enabled = e.currentTarget.checked)} aria-label={t('req.enabled')} /></td>
            <td><input bind:value={field.key} placeholder={t('req.key')} /></td>
            <td>
              <select bind:value={field.kind}>
                <option value="text">{t('body.text')}</option>
                <option value="file">{t('body.file')}</option>
              </select>
            </td>
            <td>
              {#if field.kind === 'file'}
                <div class="file">
                  <input class="mono" bind:value={field.path} placeholder={t('body.path')} title={isTauri() ? '' : t('body.pathHint')} />
                  {#if isTauri()}<button class="small" onclick={() => choose((p) => (field.path = p))}>{t('body.choose')}</button>{/if}
                </div>
              {:else}
                <input bind:value={field.value} placeholder={t('req.value')} />
              {/if}
            </td>
            <td class="del"><button class="ghost small" aria-label={t('req.remove')} onclick={() => body.fields.splice(index, 1)}>×</button></td>
          </tr>
        {/each}
        <tr>
          <td colspan="5"><button class="ghost small" onclick={() => body.fields.push({ key: '', value: '', kind: 'text', path: '', enabled: true })}>＋ {t('req.key')}</button></td>
        </tr>
      </tbody>
    </table>
  {:else if body.mode === 'binary'}
    <div class="file">
      <input class="mono" bind:value={body.path} placeholder={t('body.path')} title={isTauri() ? '' : t('body.pathHint')} />
      {#if isTauri()}<button onclick={() => choose((p) => (body.path = p))}>{t('body.choose')}</button>{/if}
    </div>
    {#if !isTauri()}<p class="muted small">{t('body.pathHint')}</p>{/if}
  {:else if body.mode === 'graphql'}
    <div class="code">
      <CodeEditor value={body.query} {describeVar} label={t('body.query')} onchange={(v) => (body.query = v)} />
    </div>
    <p class="muted small">{t('body.variables')}</p>
    <div class="code small-code">
      <CodeEditor value={body.variables} lang="json" {describeVar} label={t('body.variables')} onchange={(v) => (body.variables = v)} />
    </div>
  {/if}
</div>

<style>
  .body-editor { display: flex; flex-direction: column; gap: 8px; height: 100%; min-height: 0; }
  .modes { display: flex; gap: 14px; align-items: center; flex-wrap: wrap; }
  .modes label { display: flex; gap: 5px; align-items: center; cursor: pointer; }
  .lang { padding: 2px 6px; }
  .code { flex: 1; min-height: 120px; }
  .small-code { flex: none; height: 90px; }
  .check { width: 28px; }
  .del { width: 32px; text-align: right; }
  .file { display: flex; gap: 6px; align-items: center; }
  .file input { flex: 1; }
  .small { font-size: 11px; margin: 0; }
  p { margin: 4px 0; }
</style>
