<script lang="ts">
  // Sección Agente de la barra lateral (P5 de docs/UX.md y mockup): estado de conexión, "Conectar
  // agente ▾", medidor (llamadas, crudo → enviado, tokens ahorrados), tarjeta fija de "Pendiente de
  // aprobación" y la lista de llamadas con filtro por tool y por sesión.
  import { onDestroy, tick } from 'svelte';
  import { store } from '../stores/app.svelte';
  import { locale, t } from '../i18n.svelte';
  import { agentStatus, compact, filterCalls, savedTokens } from '../agent';
  import { formatBytes } from '../format';
  import AgentCalls from './AgentCalls.svelte';
  import ApprovalCard from './ApprovalCard.svelte';

  // El estado "conectado" caduca solo (10 min sin hablar): se reevalúa cada 30 s.
  let now = $state(Date.now());
  const timer = setInterval(() => (now = Date.now()), 30_000);
  onDestroy(() => clearInterval(timer));

  const agent = store.agent;
  const status = $derived(agentStatus(agent.clients, agent.mcp, now));
  const tools = $derived([...new Set(agent.calls.map((c) => c.tool))].sort());
  const sessions = $derived(
    [...new Map(agent.calls.filter((c) => c.session).map((c) => [c.session, `${c.client} · ${c.session}`])).entries()],
  );
  const calls = $derived(filterCalls(agent.calls, agent.tool, agent.session));
  let body = $state<HTMLDivElement>();

  // Lo más nuevo está abajo (como el mockup): si el usuario estaba al final, se queda al final.
  $effect(() => {
    void calls.length;
    const el = body;
    if (!el) return;
    const atEnd = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
    if (atEnd) void tick().then(() => el.scrollTo({ top: el.scrollHeight }));
  });
  const time = (ts: number) =>
    new Date(ts).toLocaleTimeString(locale.lang === 'en' ? 'en-GB' : 'es-ES', { hour: '2-digit', minute: '2-digit' });
</script>

<header>
  {t('rail.agent')}
  <span class="spacer"></span>
  <button class="primary small-pad" onclick={(e) => store.agentMenu(e.clientX - 160, e.clientY + 14)}>{t('side.connectAgent')}</button>
</header>
<div class="body" bind:this={body}>
  <div class="meter">
    <div class="status">
      <span class="dot {status.kind}" aria-hidden="true"></span>
      {#if status.kind === 'connected'}
        {t('agent.connected', { client: status.client })}
      {:else if status.kind === 'idle'}
        {t('agent.idle', { client: status.client, time: time(status.since) })}
      {:else if status.kind === 'none'}
        {t('agent.none')}
      {:else if status.kind === 'off'}
        {t('agent.off')}
      {:else}
        {t('agent.unavailable', { error: status.error })}
      {/if}
    </div>
    <div>
      {t('agent.callsLabel')} <b>{agent.meter.calls}</b> · {t('agent.rawLabel')} <b>{formatBytes(agent.meter.raw, locale.lang)}</b>
      → {t('agent.sentLabel')} <b>{formatBytes(agent.meter.sent, locale.lang)}</b> · ≈
      <b>{compact(savedTokens(agent.meter), locale.lang)} {t('agent.tokensWord')}</b> {t('agent.savedWord')}
    </div>
  </div>
  <div class="pane">
    {#if agent.pending.length}
      <!-- Tarjeta fija arriba mientras haya algo pendiente (P5). -->
      <div class="pending">
        {#each agent.pending as approval (approval.id)}
          <ApprovalCard {approval} compact />
        {/each}
      </div>
    {/if}
    {#if agent.calls.length}
      <div class="filters">
        <select aria-label={t('agent.filterTool')} bind:value={agent.tool}>
          <option value="">{t('agent.allTools')}</option>
          {#each tools as tool (tool)}<option value={tool}>{tool}</option>{/each}
        </select>
        <select aria-label={t('agent.filterSession')} bind:value={agent.session}>
          <option value="">{t('agent.allSessions')}</option>
          {#each sessions as [session, label] (session)}<option value={session}>{label}</option>{/each}
        </select>
        <span class="spacer"></span>
        <button class="ghost small" title={t('agent.exportTitle')} onclick={() => store.exportAgentMeter()}>{t('agent.export')}</button>
      </div>
      <AgentCalls {calls} />
      {#if calls.length === 0}<p class="hint">{t('agent.noMatches')}</p>{/if}
    {:else}
      <p class="hint">{t('side.agentIntro')}</p>
      <p class="hint">{agent.mcp.url ? t('agent.liveAt', { url: agent.mcp.url }) : t('side.agentLive')}</p>
    {/if}
  </div>
</div>

<style>
  header { display: flex; align-items: center; gap: 6px; padding: 8px 10px; border-bottom: 1px solid var(--border); font-weight: 600; min-height: 41px; }
  .small-pad { padding: 3px 10px; }
  .body { overflow: auto; flex: 1; display: flex; flex-direction: column; min-height: 0; }
  .meter { display: flex; flex-direction: column; gap: 4px; padding: 8px 12px; border-bottom: 1px solid var(--border); font-size: 12px; color: var(--muted); line-height: 1.5; }
  .meter b { color: var(--fg); }
  .status { color: var(--fg); }
  .dot { display: inline-block; width: 7px; height: 7px; border-radius: 50%; background: var(--muted); margin-right: 2px; vertical-align: middle; }
  .dot.connected { background: var(--ok); }
  .dot.error { background: var(--err); }
  .pane { padding: 6px 10px; }
  .pending { position: sticky; top: 0; z-index: 1; background: var(--bg-2); padding-top: 2px; }
  .filters { display: flex; gap: 4px; align-items: center; margin-bottom: 4px; }
  .filters select { font-size: 11px; padding: 2px 4px; flex: 1; min-width: 0; }
  .small { font-size: 11px; padding: 1px 6px; }
  .hint { color: var(--muted); padding: 6px 2px; margin: 0; font-size: 12px; line-height: 1.5; white-space: normal; }
</style>
