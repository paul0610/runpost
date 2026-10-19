<script lang="ts">
  // Diálogo Importar (fase 4), como el de Postman: pegar un comando cURL o el contenido de una colección
  // (Postman, OpenAPI, HAR, Insomnia, Bruno), o elegir archivos. Un cURL se abre en una pestaña nueva.
  import { store } from '../stores/app.svelte';
  import { t } from '../i18n.svelte';
  import { importFiles } from '../importer';
  import { isTauri } from '../transport';

  let text = $state('');
  let busy = $state(false);
  let area = $state<HTMLTextAreaElement>();

  $effect(() => {
    area?.focus();
  });

  function close(): void {
    store.ui.importOpen = false;
  }

  function key(event: KeyboardEvent): void {
    if (event.key === 'Escape') close();
    if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) void submit();
  }

  async function submit(): Promise<void> {
    if (!text.trim() || busy) return;
    busy = true;
    const done = await store.importText(text);
    busy = false;
    if (done) close();
  }
</script>

<svelte:window onkeydown={key} />

<div class="overlay" role="presentation" onclick={close}>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="import-title" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={() => undefined}>
    <h2 id="import-title">{t('import.title')}</h2>
    <label class="paste">
      <span>{t('import.pasteLabel')}</span>
      <textarea bind:this={area} bind:value={text} class="mono" spellcheck="false" placeholder={t('import.pastePlaceholder')}></textarea>
    </label>
    <div class="row">
      <button class="primary" disabled={!text.trim() || busy} onclick={submit}>{t('import.importText')} <kbd>Ctrl+↵</kbd></button>
      <span class="muted or">{t('import.or')}</span>
      <button onclick={importFiles}>{t('import.chooseFiles')}</button>
    </div>
    <p class="muted small">{t('import.formats')}</p>
    <p class="muted small">{isTauri() ? t('import.brunoHint') : t('import.brunoHintBrowser')}</p>
    <div class="actions">
      <button class="ghost" onclick={close}>{t('import.close')}</button>
    </div>
  </div>
</div>

<style>
  .overlay { position: fixed; inset: 0; background: var(--overlay); display: grid; place-items: center; z-index: 60; }
  .dialog { width: min(620px, 94vw); background: var(--bg); border: 1px solid var(--border); border-radius: 10px; box-shadow: var(--shadow); padding: 18px 20px; display: flex; flex-direction: column; gap: 10px; }
  h2 { margin: 0; font-size: 18px; }
  .paste { display: flex; flex-direction: column; gap: 6px; }
  .paste span { color: var(--muted); font-size: 11px; text-transform: uppercase; letter-spacing: .06em; }
  textarea { min-height: 170px; resize: vertical; font-size: 12px; line-height: 1.5; }
  .row { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .row kbd { color: var(--on-accent); border: none; padding: 0; opacity: .7; margin-left: 4px; font-size: 10px; }
  .or { font-size: 12px; }
  .small { margin: 0; font-size: 12px; line-height: 1.5; }
  .actions { display: flex; justify-content: flex-end; }
</style>
