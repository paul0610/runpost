<script lang="ts">
  // "Acerca de" (sección 21 y SPRINTS S7): crédito "Creado por Paul Rojas" con enlace a GitHub, licencia
  // y, plegados, los datos técnicos para depurar (docs/UX.md: puerto solo visible aquí). Nunca el token.
  import { version } from '../../../package.json';
  import { store } from '../stores/app.svelte';
  import { t } from '../i18n.svelte';
  import { openExternal } from '../native';

  const REPO_URL = 'https://github.com/paul0610/runpost';

  let technical = $state(false);
  const mcp = $derived(store.agent.mcp);

  function close(): void {
    store.ui.about = false;
  }

  function key(event: KeyboardEvent): void {
    if (event.key === 'Escape') close();
  }
</script>

<svelte:window onkeydown={key} />

<div class="overlay" role="presentation" onclick={close}>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="about-title" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={() => undefined}>
    <div class="head">
      <svg class="logo" viewBox="0 0 1024 1024" aria-hidden="true">
        <rect class="bg" x="48" y="48" width="928" height="928" rx="208" />
        <path
          class="fg"
          fill-rule="evenodd"
          d="M262 236 H532 C668 236 742 316 742 420 C742 506 694 570 616 594 L748 788 H614 L497 606 H382 V788 H262 Z M382 340 V506 H524 C586 506 620 472 620 423 C620 374 586 340 524 340 Z"
        />
        <circle class="dot" cx="830" cy="738" r="64" />
      </svg>
      <div>
        <h2 id="about-title">RunPost <span class="muted version">{version}</span></h2>
        <p class="muted">{t('about.tagline')}</p>
      </div>
    </div>

    <p>
      {t('about.createdBy')} <b>Paul Rojas</b> ·
      <a href={REPO_URL} onclick={(e) => (e.preventDefault(), void openExternal(REPO_URL))}>github.com/paul0610/runpost</a>
    </p>
    <p class="muted">{t('about.license')}</p>

    <button class="ghost toggle" aria-expanded={technical} onclick={() => (technical = !technical)}>{technical ? '▾' : '▸'} {t('about.technical')}</button>
    {#if technical}
      <dl class="grid">
        <dt>{t('about.mode')}</dt>
        <dd>{store.backend?.mode === 'tauri' ? t('about.modeTauri') : t('about.modeBrowser')}</dd>
        <dt>{t('about.api')}</dt>
        <dd class="mono">127.0.0.1:{store.backend?.port}</dd>
        <dt>{t('about.mcp')}</dt>
        <dd class="mono">{mcp.url ?? (mcp.error ? t('agent.unavailable', { error: mcp.error }) : t('agent.off'))}</dd>
        <dt>{t('about.workspace')}</dt>
        <dd class="mono path">{store.backend?.workspace}</dd>
      </dl>
    {/if}

    <div class="actions">
      <button class="primary" onclick={close}>{t('dialog.close')}</button>
    </div>
  </div>
</div>

<style>
  .overlay { position: fixed; inset: 0; background: var(--overlay); display: grid; place-items: center; z-index: 60; }
  .dialog { width: min(480px, 94vw); background: var(--bg); border: 1px solid var(--border); border-radius: 10px; box-shadow: var(--shadow); padding: 18px 20px; }
  .head { display: flex; gap: 14px; align-items: center; margin-bottom: 10px; }
  .logo { width: 56px; height: 56px; flex: none; }
  .logo .bg { fill: var(--logo-bg); }
  .logo .fg { fill: var(--logo-fg); }
  .logo .dot { fill: var(--logo-dot); }
  h2 { margin: 0; font-size: 18px; }
  .version { font-size: 13px; font-weight: normal; }
  p { margin: 6px 0; line-height: 1.5; }
  a { color: var(--accent-text); }
  .toggle { margin: 10px 0 4px; padding-left: 0; font-weight: 600; }
  .grid { display: grid; grid-template-columns: 140px 1fr; gap: 6px 12px; margin: 6px 0 0; font-size: 12px; }
  dt { color: var(--muted); }
  dd { margin: 0; min-width: 0; }
  .path { overflow-wrap: anywhere; }
  .actions { display: flex; justify-content: flex-end; margin-top: 14px; }
</style>
