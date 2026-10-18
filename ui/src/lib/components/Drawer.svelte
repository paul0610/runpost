<script lang="ts">
  // Panel inferior plegable (P1): Consola · Agente · Runner. La Consola (S4) muestra cada envío con el
  // formato del mockup; el Runner (S5) ejecuta colecciones; el Agente (S6) resume las llamadas del
  // modelo por MCP con el medidor y "Ver todo en el panel Agente".
  import { tick } from 'svelte';
  import { store, type DrawerTab } from '../stores/app.svelte';
  import { locale, t } from '../i18n.svelte';
  import { formatBytes, methodClass, statusClass } from '../format';
  import { compact, savedTokens } from '../agent';
  import RunnerPanel from './RunnerPanel.svelte';
  import AgentCalls from './AgentCalls.svelte';

  /** El cliente del medidor: el que habló por última vez. */
  const lastClient = $derived([...store.agent.clients].sort((a, b) => b.last_seen - a.last_seen)[0]?.client ?? t('agent.noClient'));
  const recentCalls = $derived(store.agent.calls.slice(-50));
  let agentLog = $state<HTMLDivElement>();

  const tabs: { id: DrawerTab; label: string }[] = [
    { id: 'console', label: 'drawer.console' },
    { id: 'agent', label: 'drawer.agent' },
    { id: 'runner', label: 'drawer.runner' },
  ];
  let log = $state<HTMLDivElement>();

  function toggle(id: DrawerTab): void {
    store.ui.drawer = store.ui.drawer === id ? null : id;
  }

  const time = (ts: number) =>
    new Date(ts).toLocaleTimeString(locale.lang === 'en' ? 'en-GB' : 'es-ES', { hour: '2-digit', minute: '2-digit', second: '2-digit' });

  // Cada envío nuevo baja la consola hasta el final; igual cada llamada nueva del agente.
  $effect(() => {
    void store.consoleEntries.length;
    if (store.ui.drawer === 'console') void tick().then(() => log?.scrollTo({ top: log.scrollHeight }));
  });
  $effect(() => {
    void store.agent.calls.length;
    if (store.ui.drawer === 'agent') void tick().then(() => agentLog?.scrollTo({ top: agentLog.scrollHeight }));
  });
</script>

<div class="drawer" class:open={store.ui.drawer !== null}>
  <div class="bar" role="tablist">
    {#each tabs as tab (tab.id)}
      <button role="tab" aria-selected={store.ui.drawer === tab.id} class:on={store.ui.drawer === tab.id} onclick={() => toggle(tab.id)}>
        {store.ui.drawer === tab.id ? '▾' : '▸'} {t(tab.label)}
        {#if tab.id === 'console' && store.consoleEntries.length}<span class="n">{store.consoleEntries.length}</span>{/if}
        {#if tab.id === 'agent' && (store.agent.unseen || store.agent.pending.length)}
          <span class="n" title={t('agent.newCalls')}>{store.agent.pending.length ? `${store.agent.pending.length}!` : store.agent.unseen}</span>
        {/if}
        {#if tab.id === 'runner' && store.runner.status === 'running'}<span class="n">{store.runner.items.length}/{store.runner.total || '…'}</span>{/if}
      </button>
    {/each}
    <span class="spacer"></span>
    {#if store.ui.drawer === 'console' && store.consoleEntries.length}
      <button class="ghost small" onclick={() => store.clearConsole()}>{t('drawer.clear')}</button>
    {/if}
    <span class="pill">{t('drawer.shortcut')}</span>
  </div>
  {#if store.ui.drawer === 'console'}
    <div class="content log" bind:this={log} role="log" aria-live="polite">
      {#each store.consoleEntries as entry (entry.id)}
        <div class="entry">
          <div><span class="t">{time(entry.ts)}</span><span class={methodClass(entry.method)}>{entry.method}</span> {entry.url}</div>
          {#if entry.headers.length}
            <div><span class="t"></span>→ {entry.headers.map(([k, v]) => `${k}: ${v}`).join(' · ')}</div>
          {/if}
          {#if entry.unresolved.length}
            <div class="warn"><span class="t"></span>⚠ {t('res.unresolved', { names: entry.unresolved.join(', ') })}</div>
          {/if}
          {#if entry.error}
            <div class="err"><span class="t"></span>✘ {entry.error}</div>
          {:else if entry.skipped}
            <div class="c"><span class="t"></span>↷ {t('drawer.skipped')}</div>
          {:else}
            <div>
              <span class="t"></span>← <span class="code-{statusClass(entry.status ?? 0)}">{entry.status}</span> · {entry.ms} ms ·
              {formatBytes(entry.size ?? 0, locale.lang)}{entry.contentType ? ` · content-type: ${entry.contentType}` : ''}
            </div>
          {/if}
          {#each entry.console as line, index (index)}
            <div><span class="t"></span><span class="c">console.log</span> {line}</div>
          {/each}
          {#each entry.failures ?? [] as failure, index (index)}
            <div><span class="t"></span><span class="err">test</span> "{failure.name}"{#if failure.msg}: {failure.msg.replace(/^AssertionError: /, '')}{/if}</div>
          {/each}
        </div>
      {:else}
        <p class="muted">{t('drawer.consoleEmpty')}</p>
      {/each}
    </div>
  {:else if store.ui.drawer === 'runner'}
    <div class="content runner-pane">
      <RunnerPanel />
    </div>
  {:else if store.ui.drawer}
    <div class="content agent-pane" bind:this={agentLog}>
      <div class="meter">
        <span>
          {lastClient} · <b>{store.agent.meter.calls}</b> {t('agent.callsWord')} ·
          <b>{formatBytes(store.agent.meter.raw, locale.lang)} → {formatBytes(store.agent.meter.sent, locale.lang)}</b> · ≈
          <b>{compact(savedTokens(store.agent.meter), locale.lang)}</b> {t('agent.tokensSavedWord')}
        </span>
        <span class="spacer"></span>
        <button class="ghost small" onclick={() => store.showAgentPanel()}>{t('agent.seeAll')}</button>
      </div>
      {#if recentCalls.length}
        <AgentCalls calls={recentCalls} wide />
      {:else}
        <p class="muted empty">{store.agent.mcp.url ? t('agent.drawerEmptyAt', { url: store.agent.mcp.url }) : t('drawer.agentEmpty')}</p>
      {/if}
    </div>
  {/if}
</div>

<style>
  .drawer { border-top: 1px solid var(--border); background: var(--bg-2); flex: none; }
  .bar { display: flex; align-items: center; gap: 2px; padding: 0 8px; height: 32px; }
  .bar button[role='tab'] { border: none; border-radius: 0; padding: 6px 10px; color: var(--muted); border-bottom: 2px solid transparent; }
  .bar button[role='tab']:hover { background: none; color: var(--fg); }
  .bar button[role='tab'].on { color: var(--fg); border-color: var(--accent); }
  .n { color: var(--accent-text); font-size: 11px; margin-left: 4px; }
  .content { height: 210px; overflow: auto; background: var(--bg); border-top: 1px solid var(--border); padding: 8px 12px; font-family: Consolas, monospace; font-size: 12px; }
  .content p { margin: 0; }
  .content.runner-pane { height: 300px; }
  .content.agent-pane { padding: 0; font-family: inherit; --sticky-top: 33px; }
  .meter { display: flex; gap: 18px; align-items: center; padding: 8px 12px; border-bottom: 1px solid var(--border); font-size: 12px; color: var(--muted); position: sticky; top: 0; background: var(--bg); z-index: 2; height: 33px; box-sizing: border-box; }
  .meter b { color: var(--fg); }
  .agent-pane .empty { padding: 8px 12px; }
  .small { font-size: 11px; padding: 1px 6px; }
  .entry { padding: 2px 0 6px; word-break: break-all; }
  .entry div { padding: 1px 0; }
  .t { color: var(--muted); display: inline-block; width: 72px; }
  .c { color: var(--muted); }
  .warn { color: var(--warn); }
  .err { color: var(--err); }
  .code-ok { color: var(--ok); }
  .code-info { color: var(--info); }
  .code-err { color: var(--err); }
</style>
