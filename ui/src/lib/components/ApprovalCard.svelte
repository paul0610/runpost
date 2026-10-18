<script lang="ts">
  // Petición protegida esperando al usuario (R4.3 y mockup): como aviso arriba a la derecha y, en
  // versión breve, como tarjeta fija del panel Agente. Aprobar / Denegar y cuenta atrás de 60 s.
  import { onDestroy } from 'svelte';
  import { store } from '../stores/app.svelte';
  import { t } from '../i18n.svelte';
  import { methodClass } from '../format';
  import { urlParts } from '../agent';
  import type { Approval } from '../types';

  let { approval, compact = false }: { approval: Approval; compact?: boolean } = $props();

  let now = $state(Date.now());
  const timer = setInterval(() => (now = Date.now()), 500);
  onDestroy(() => clearInterval(timer));

  const remaining = $derived(Math.max(0, Math.ceil((approval.expires - now) / 1000)));
  const busy = $derived(store.agent.deciding.includes(approval.id));
  const title = $derived(
    approval.count > 1
      ? t('agent.wantsMany', { client: approval.client, count: approval.count, collection: approval.collection })
      : t('agent.wants', { client: approval.client }),
  );
</script>

{#if compact}
  <div class="card" role="group" aria-label={title}>
    <b>{t('agent.pending')}</b><br />
    <span class="mono {methodClass(approval.method)}">{approval.method}</span>
    <span class="mono">{#each urlParts(approval.url) as part, i (i)}{part}<wbr />{/each}</span>
    {#if approval.count > 1}<span class="muted"> {t('agent.andMore', { count: approval.count - 1 })}</span>{/if}
    <div class="row">
      <button class="primary" disabled={busy} onclick={() => store.decideApproval(approval.id, true)}>{t('agent.approve')}</button>
      <button disabled={busy} onclick={() => store.decideApproval(approval.id, false)}>{t('agent.deny')}</button>
      <span class="pill right" aria-label={t('agent.remaining', { seconds: remaining })}>{remaining} s</span>
    </div>
  </div>
{:else}
  <div class="toast" role="alert">
    <h4>🤖 {title}</h4>
    <p>
      <span class="mono {methodClass(approval.method)}">{approval.method}</span>
      <span class="mono">{#each urlParts(approval.url) as part, i (i)}{part}<wbr />{/each}</span>
      {#if approval.count > 1}<span> {t('agent.andMore', { count: approval.count - 1 })}</span>{/if}
      <br />
      {t('agent.collection')} <b>{approval.collection}</b>
      {#if approval.environment}· {t('agent.environment')} <b>{approval.environment}</b>{/if}
      · {t('agent.rule')}: <code>{approval.rule}</code>
    </p>
    <div class="row">
      <button class="primary" disabled={busy} onclick={() => store.decideApproval(approval.id, true)}>{t('agent.approve')}</button>
      <button disabled={busy} onclick={() => store.decideApproval(approval.id, false)}>{t('agent.deny')}</button>
      <span class="spacer"></span>
      <span class="pill" aria-label={t('agent.remaining', { seconds: remaining })}>{remaining} s</span>
    </div>
  </div>
{/if}

<style>
  .toast { background: var(--bg); border: 1px solid var(--border); border-left: 4px solid var(--warn); border-radius: 8px; box-shadow: var(--shadow); padding: 12px 14px; pointer-events: auto; }
  .toast h4 { margin: 0 0 6px; font-size: 13px; }
  .toast p { margin: 0 0 10px; color: var(--muted); line-height: 1.5; overflow-wrap: anywhere; }
  .toast p b, .toast code { color: var(--fg); }
  .row { display: flex; gap: 8px; align-items: center; }
  .card { border: 1px solid var(--warn); border-radius: 8px; padding: 8px 10px; margin-bottom: 8px; line-height: 1.5; overflow-wrap: anywhere; background: var(--bg-2); }
  .card .row { gap: 6px; margin-top: 6px; }
  .right { margin-left: auto; }
</style>
