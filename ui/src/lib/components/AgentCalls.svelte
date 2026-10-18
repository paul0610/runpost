<script lang="ts">
  // Llamadas del agente (R4.2): hora, tool, petición, estado y bytes crudos → enviados. En el panel
  // lateral, cinco columnas estrechas (como el mockup); en el panel inferior, además ms y "Abrir".
  // Cada fila abre su petición como pestaña normal.
  import { store } from '../stores/app.svelte';
  import { locale, t } from '../i18n.svelte';
  import { callTone, compact, savedTokens } from '../agent';
  import { formatBytes } from '../format';
  import type { AgentCall } from '../types';

  let { calls, wide = false }: { calls: AgentCall[]; wide?: boolean } = $props();

  const time = (ts: number) =>
    new Date(ts).toLocaleTimeString(locale.lang === 'en' ? 'en-GB' : 'es-ES', { hour: '2-digit', minute: '2-digit' });
  const tip = (call: AgentCall) =>
    [
      `${call.client}${call.session ? ` · ${t('agent.session')} ${call.session}` : ''}`,
      `${call.tool} · ${call.target}`,
      `${formatBytes(call.raw, locale.lang)} → ${formatBytes(call.sent, locale.lang)} · ≈ ${savedTokens(call)} ${t('agent.tokensSaved')}`,
      call.request_id ? t('agent.openHint') : '',
    ]
      .filter(Boolean)
      .join('\n');
</script>

<table class="agent" class:wide>
  {#if !wide}
    <!-- 290 px de panel: anchos fijos y puntos suspensivos en vez de desbordar. -->
    <colgroup><col style="width: 34px" /><col style="width: 30%" /><col /><col style="width: 40px" /><col style="width: 58px" /></colgroup>
  {/if}
  <thead>
    <tr>
      <th>{t('agent.colTime')}</th>
      <th>{t('agent.colTool')}</th>
      <th>{t('agent.colRequest')}</th>
      <th>{t('agent.colStatus')}</th>
      {#if wide}
        <th class="num">ms</th>
        <th class="num">{t('agent.colBytes')}</th>
        <th></th>
      {:else}
        <th class="num" title={t('agent.colBytes')}>t</th>
      {/if}
    </tr>
  </thead>
  <tbody>
    {#each calls as call (call.id)}
      <tr class:link={!!call.request_id} title={tip(call)} onclick={() => call.request_id && store.openAgentCall(call)}>
        <td>{time(call.ts)}</td>
        <td>{call.tool}</td>
        <td class="target">{call.target || '—'}</td>
        <td class={callTone(call)}>{call.status}{wide && /^\d+\/\d+$/.test(call.status) ? ' tests' : ''}</td>
        {#if wide}
          <td class="num">{call.ms.toLocaleString(locale.lang === 'en' ? 'en-US' : 'es-ES')}</td>
          <td class="num">{formatBytes(call.raw, locale.lang)} → {formatBytes(call.sent, locale.lang)}</td>
          <td>
            {#if call.request_id}
              <button class="ghost small" onclick={(e) => (e.stopPropagation(), store.openAgentCall(call))}>{t('agent.open')}</button>
            {/if}
          </td>
        {:else}
          <td class="num">{compact(call.raw, locale.lang)}→{compact(call.sent, locale.lang)}</td>
        {/if}
      </tr>
    {/each}
  </tbody>
</table>

<style>
  table.agent { width: 100%; border-collapse: collapse; font-size: 12px; table-layout: auto; }
  table.agent td, table.agent th { padding: 5px 8px; border-bottom: 1px solid var(--border); text-align: left; white-space: nowrap; }
  table.agent th { color: var(--muted); font-weight: normal; font-size: 11px; }
  /* En el panel inferior la cabecera queda visible al bajar (debajo del medidor, --sticky-top). */
  .wide thead th { position: sticky; top: var(--sticky-top, 0); background: var(--bg); z-index: 1; }
  table.agent:not(.wide) { table-layout: fixed; font-size: 11.5px; }
  table.agent:not(.wide) td, table.agent:not(.wide) th { padding: 5px 3px; overflow: hidden; text-overflow: ellipsis; }
  table.agent:not(.wide) td:first-child, table.agent:not(.wide) th:first-child { padding-left: 0; }
  .wide .target { max-width: 420px; overflow: hidden; text-overflow: ellipsis; }
  .num { text-align: right; }
  tr.link { cursor: pointer; }
  tr.link:hover { background: var(--bg-3); }
  .ok { color: var(--ok); }
  .warn { color: var(--warn); }
  .err { color: var(--err); }
  .small { font-size: 11px; padding: 0 6px; }
</style>
