<script lang="ts">
  // Runner (P4 de docs/UX.md; pestaña del panel inferior, como el mockup): colección o carpeta,
  // entorno, iteraciones, datos, retardo y ▶ Ejecutar; progreso en vivo por WebSocket; tabla
  // Petición · Estado · ms · Tests con el detalle de cada fila; "Solo fallos"; Exportar informe.
  import { store } from '../stores/app.svelte';
  import { t } from '../i18n.svelte';
  import { methodClass, methodLabel, statusClass } from '../format';
  import type { Item, RunnerItem } from '../types';

  function folders(items: Item[], prefix: string): { id: string; label: string }[] {
    return items.flatMap((item) =>
      item.type === 'folder'
        ? [{ id: item.id, label: `${prefix} / ${item.name}` }, ...folders(item.items, `${prefix} / ${item.name}`)]
        : [],
    );
  }

  /** Cada colección y, debajo, sus carpetas (`Colección / Carpeta`). */
  const targets = $derived(
    store.collections.flatMap((c) => [
      { value: c.id, label: c.name },
      ...folders(c.items, c.name).map((f) => ({ value: `${c.id}|${f.id}`, label: f.label })),
    ]),
  );
  const target = $derived(
    store.runner.collectionId
      ? store.runner.folderId
        ? `${store.runner.collectionId}|${store.runner.folderId}`
        : store.runner.collectionId
      : (targets[0]?.value ?? ''),
  );

  function setTarget(value: string): void {
    const [collectionId, folderId] = value.split('|');
    store.runner.collectionId = collectionId || null;
    store.runner.folderId = folderId ?? null;
  }

  const running = $derived(store.runner.status === 'running');
  const items = $derived(store.runner.items);
  const failed = (item: RunnerItem) => !!item.error || item.tests.some((x) => !x.passed && !x.skipped);
  const rows = $derived(
    items.map((item, index) => ({ item, index })).filter(({ item }) => !store.runner.onlyFailures || failed(item)),
  );
  const several = $derived((store.runner.report?.iterations ?? 1) > 1 || items.some((i) => i.iteration > 0));
  const counts = $derived.by(() => {
    const out = { passed: 0, failed: 0, skipped: 0, errors: 0 };
    for (const item of items) {
      if (item.skipped) out.skipped += 1;
      if (item.error) out.errors += 1;
      for (const test of item.tests) {
        if (test.skipped) continue;
        if (test.passed) out.passed += 1;
        else out.failed += 1;
      }
    }
    return out;
  });
  const progress = $derived(
    store.runner.total ? Math.min(100, Math.round((items.length / store.runner.total) * 100)) : store.runner.status === 'done' ? 100 : 0,
  );

  function testCount(item: RunnerItem): string {
    const counted = item.tests.filter((x) => !x.skipped);
    return counted.length ? `${counted.filter((x) => x.passed).length}/${counted.length}` : '—';
  }

  function toggle(index: number): void {
    const expanded = store.runner.expanded;
    store.runner.expanded = expanded.includes(index) ? expanded.filter((i) => i !== index) : [...expanded, index];
  }

  function statusText(item: RunnerItem): string {
    if (item.skipped) return t('runner.skipped');
    if (item.error) return item.error;
    return String(item.status ?? '');
  }
</script>

<div class="runner">
  <div class="controls">
    <select
      aria-label={t('runner.target')}
      title={t('runner.target')}
      value={target}
      onchange={(e) => setTarget(e.currentTarget.value)}
      disabled={running || !targets.length}
    >
      {#each targets as option (option.value)}
        <option value={option.value}>{option.label}</option>
      {/each}
    </select>
    <select
      aria-label={t('runner.env')}
      title={t('runner.env')}
      value={store.runner.envId ?? ''}
      onchange={(e) => (store.runner.envId = e.currentTarget.value || null)}
      disabled={running}
    >
      <option value="">{store.activeEnvironment ? t('runner.activeEnv', { name: store.activeEnvironment.name }) : t('top.envNone')}</option>
      {#each store.environments as env (env.id)}
        <option value={env.id}>{env.name}</option>
      {/each}
    </select>
    <label>
      {t('runner.iterations')}
      <input
        type="number"
        min="1"
        class="num"
        placeholder={t('runner.auto')}
        value={store.runner.iterations ?? ''}
        oninput={(e) => (store.runner.iterations = e.currentTarget.value === '' ? null : Number(e.currentTarget.value))}
        disabled={running}
      />
    </label>
    <span class="data">
      {t('runner.data')}
      <button onclick={() => store.pickRunnerData()} disabled={running} title={store.runner.data?.name ?? t('runner.chooseDataHint')}>
        {store.runner.data ? store.runner.data.name : t('runner.chooseData')}
      </button>
      {#if store.runner.data}
        <button class="ghost small" aria-label={t('runner.clearData')} title={t('runner.clearData')} onclick={() => store.clearRunnerData()} disabled={running}>×</button>
      {/if}
    </span>
    <label>
      {t('runner.delay')}
      <input type="number" min="0" step="100" class="num" bind:value={store.runner.delayMs} disabled={running} /> ms
    </label>
    {#if running}
      <button class="primary" onclick={() => store.cancelRun()}>■ {t('runner.stop')}</button>
    {:else}
      <button class="primary" disabled={!targets.length} onclick={() => store.startRun()}>▶ {t('runner.run')}</button>
    {/if}
    <span class="spacer"></span>
    <label class="check"><input type="checkbox" bind:checked={store.runner.onlyFailures} /> {t('runner.onlyFailures')}</label>
    <button disabled={!store.runner.report} onclick={(e) => store.runnerExportMenu(e.clientX - 140, e.clientY + 12)}>{t('runner.export')}</button>
  </div>

  {#if store.runner.status !== 'idle'}
    <div class="progress" role="progressbar" aria-label={t('runner.progress')} aria-valuemin={0} aria-valuemax={store.runner.total} aria-valuenow={items.length}>
      <div class="bar" class:bad={counts.failed + counts.errors > 0} style:width="{progress}%"></div>
    </div>
    <p class="summary">
      {t('runner.summary', { done: items.length, total: store.runner.total })}
      · <span class="ok">✔ {counts.passed}</span> · <span class="err">✘ {counts.failed}</span>
      {#if counts.skipped}· {t('runner.skippedCount', { count: counts.skipped })}{/if}
      {#if counts.errors}· <span class="err">{t('runner.errorsCount', { count: counts.errors })}</span>{/if}
      {#if store.runner.report}· {store.runner.report.ms} ms{#if store.runner.report.cancelled}{' · '}{t('runner.cancelled')}{/if}{/if}
      {#if store.runner.error}· <span class="err">{store.runner.error}</span>{/if}
    </p>
  {/if}

  {#if items.length}
    <table class="runs">
      <thead>
        <tr>
          <th>{t('runner.request')}</th>
          <th>{t('runner.status')}</th>
          <th class="num-col">ms</th>
          <th class="num-col">{t('runner.tests')}</th>
        </tr>
      </thead>
      <tbody>
        {#each rows as { item, index } (index)}
          {@const open = store.runner.expanded.includes(index)}
          {@const bad = failed(item)}
          <tr
            class="row"
            class:fail={bad}
            class:skip={item.skipped}
            tabindex="0"
            aria-expanded={open}
            onclick={() => toggle(index)}
            onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), toggle(index))}
          >
            <td class="what">
              <span class="caret">{open ? '▾' : '▸'}</span>
              <span class="meth {methodClass(item.method)}">{methodLabel(item.method)}</span>
              {several ? `#${item.iteration + 1} · ` : ''}{item.path}
            </td>
            <td class={item.skipped ? 'muted' : item.error ? 'err' : `code-${statusClass(item.status ?? 0)}`}>{statusText(item)}</td>
            <td class="num-col">{item.skipped || item.error ? '' : item.ms}</td>
            <td class="num-col" class:err={item.tests.some((x) => !x.passed && !x.skipped)} class:ok={!bad && item.tests.length > 0}>{testCount(item)}</td>
          </tr>
          {#if open}
            <tr class="details">
              <td colspan="4">
                <div class="mono muted url">{item.url}</div>
                {#if item.tests.length}
                  <ul class="tests">
                    {#each item.tests as test, n (n)}
                      <li class:ok={test.passed && !test.skipped} class:err={!test.passed && !test.skipped} class:muted={test.skipped}>
                        {test.skipped ? '–' : test.passed ? '✔' : '✘'} {test.name}
                        {#if !test.passed && !test.skipped && test.msg}<div class="msg mono">{test.msg}</div>{/if}
                      </li>
                    {/each}
                  </ul>
                {:else}
                  <p class="muted">{t('runner.noTests')}</p>
                {/if}
                {#each item.console as line, n (n)}
                  <div class="mono console"><span class="muted">console</span> {line}</div>
                {/each}
              </td>
            </tr>
          {/if}
        {/each}
      </tbody>
    </table>
    {#if !rows.length}<p class="muted">{t('runner.noFailures')}</p>{/if}
  {:else if store.runner.status === 'idle'}
    <p class="muted">{targets.length ? t('drawer.runnerEmpty') : t('runner.noCollections')}</p>
  {/if}
</div>

<style>
  .runner { display: flex; flex-direction: column; gap: 8px; }
  .controls { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; font-family: "Segoe UI", system-ui, sans-serif; font-size: 12.5px; }
  .controls select { max-width: 280px; }
  .controls label, .data { display: inline-flex; gap: 6px; align-items: center; color: var(--muted); }
  .num { width: 64px; padding: 3px 6px; }
  .data button:not(.ghost) { max-width: 180px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .check { cursor: pointer; }
  .progress { height: 4px; background: var(--bg-3); border-radius: 2px; overflow: hidden; }
  .bar { height: 100%; background: var(--ok); transition: width .2s; }
  .bar.bad { background: var(--err); }
  .summary { margin: 0; font-family: "Segoe UI", system-ui, sans-serif; font-size: 12px; color: var(--muted); }
  .runs { width: 100%; border-collapse: collapse; font-family: "Segoe UI", system-ui, sans-serif; font-size: 12.5px; }
  .runs th { text-align: left; color: var(--muted); font-weight: normal; font-size: 11px; text-transform: uppercase; letter-spacing: .06em; padding: 4px 8px; border-bottom: 1px solid var(--border); }
  .runs td { padding: 4px 8px; border-bottom: 1px solid var(--border); vertical-align: top; }
  .row { cursor: pointer; }
  .row:hover { background: var(--bg-2); }
  .row:focus-visible { outline: 2px solid var(--focus); outline-offset: -2px; }
  .row.skip .what { color: var(--muted); }
  .row.fail td:first-child { box-shadow: inset 2px 0 0 var(--err); }
  .what { word-break: break-word; }
  .caret { color: var(--muted); display: inline-block; width: 12px; }
  .meth { font-weight: 700; font-size: 10px; margin-right: 4px; }
  .num-col { text-align: right; width: 64px; white-space: nowrap; }
  .details td { background: var(--bg-2); }
  .url { word-break: break-all; margin-bottom: 4px; }
  .tests { list-style: none; margin: 0; padding: 0; }
  .tests li { padding: 2px 0; }
  .msg { margin: 2px 0 2px 16px; color: var(--fg); white-space: pre-wrap; word-break: break-word; }
  .console { margin-top: 2px; word-break: break-all; }
  .ok, .code-ok { color: var(--ok); }
  .err, .code-err { color: var(--err); }
  .code-info { color: var(--info); }
  p { margin: 0; }
</style>
