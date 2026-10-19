<script lang="ts">
  // Estado vacío (P2): Importar de Postman · Nueva petición · Conectar tu agente.
  import { store } from '../stores/app.svelte';
  import { t } from '../i18n.svelte';
  import { openImport } from '../importer';

  function agent(event: MouseEvent): void {
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    store.agentMenu(rect.left + 16, rect.bottom - 8);
  }
</script>

<div class="empty">
  <div>
    <div class="cards">
      <button class="card" onclick={openImport}>
        <div class="ic">⬇</div>
        <b>{t('empty.import')}</b>
        <span>{t('empty.importHint')}</span>
      </button>
      <button class="card" onclick={() => store.newTab()}>
        <div class="ic">＋</div>
        <b>{t('empty.newRequest')}</b>
        <span>{t('empty.newRequestHint')}</span>
      </button>
      <button class="card" onclick={agent}>
        <div class="ic">🤖</div>
        <b>{t('empty.agent')}</b>
        <span>{t('empty.agentHint')}</span>
      </button>
    </div>
    <p>{t('empty.footer', { path: store.backend?.workspace ?? './workspace' })}</p>
  </div>
</div>

<style>
  .empty { flex: 1; display: grid; place-items: center; padding: 30px; overflow: auto; }
  .cards { display: grid; grid-template-columns: repeat(3, 200px); gap: 14px; }
  .card { border: 1px solid var(--border); border-radius: 12px; padding: 18px; background: var(--bg-2); cursor: pointer; text-align: left; display: block; }
  .card:hover:not(:disabled) { border-color: var(--accent); background: var(--bg-2); }
  .ic { font-size: 24px; margin-bottom: 8px; }
  .card b { display: block; margin-bottom: 4px; }
  .card span { color: var(--muted); font-size: 12px; }
  p { color: var(--muted); margin-top: 18px; font-size: 12px; text-align: center; }
</style>
