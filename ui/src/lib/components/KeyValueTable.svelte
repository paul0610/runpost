<script lang="ts">
  // Tabla Clave · Valor (P1 y sección 18): casilla por fila y maestra, fila nueva al escribir en la
  // última, "Bulk edit" y píldora "secreto" cuando el valor usa una variable secreta.
  import { tick } from 'svelte';
  import { t } from '../i18n.svelte';
  import { fromBulk, toBulk } from '../kv';
  import { variableNames, type VarLookup } from '../vars';
  import type { KeyValue } from '../types';

  interface Props {
    rows: KeyValue[];
    /** Se llama tras cada cambio (p. ej. para recomponer la URL desde Params). */
    onchange?: () => void;
    keyList?: string;
    valueList?: (key: string) => string | undefined;
    /** Filas informativas de solo lectura (cabeceras que añade Auth). */
    extra?: { key: string; value: string; note: string }[];
    lookup?: VarLookup;
  }

  let { rows = $bindable(), onchange, keyList, valueList, extra = [], lookup }: Props = $props();
  let bulk = $state(false);
  let bulkText = $state('');
  let body = $state<HTMLTableSectionElement>();

  const allOn = $derived(rows.length > 0 && rows.every((r) => r.enabled !== false));
  const someOn = $derived(rows.some((r) => r.enabled !== false));

  function changed(): void {
    onchange?.();
  }

  function toggleAll(checked: boolean): void {
    for (const row of rows) row.enabled = checked;
    changed();
  }

  /** Escribir en la fila fantasma crea una fila real y el foco pasa a ella. */
  async function addRow(event: Event & { currentTarget: HTMLInputElement }, field: 'key' | 'value'): Promise<void> {
    const text = event.currentTarget.value;
    event.currentTarget.value = '';
    rows.push({ key: field === 'key' ? text : '', value: field === 'value' ? text : '', enabled: true });
    changed();
    await tick();
    const inputs = body?.querySelectorAll<HTMLInputElement>(`tr.data input[data-field="${field}"]`);
    const input = inputs?.[inputs.length - 1];
    input?.focus();
    input?.setSelectionRange(text.length, text.length);
  }

  function remove(index: number): void {
    rows.splice(index, 1);
    changed();
  }

  function toggleBulk(): void {
    if (!bulk) bulkText = toBulk(rows);
    bulk = !bulk;
  }

  function bulkInput(): void {
    rows.splice(0, rows.length, ...fromBulk(bulkText));
    changed();
  }

  /** Variables del valor: si alguna es secreta y el texto del hover (como en la barra de URL). */
  function varsOf(value: string): { secret: boolean; missing: boolean; title: string } {
    const names = variableNames(value);
    if (!lookup || !names.length) return { secret: false, missing: false, title: '' };
    const infos = names.map((name) => ({ name, info: lookup(name) }));
    return {
      secret: infos.some((x) => x.info?.secret),
      missing: infos.some((x) => !x.info),
      title: infos
        .map(({ name, info }) =>
          !info ? t('req.varMissing', { name }) : info.secret ? t('req.varSecret', { name }) : info.scope === 'dynamic' ? t('req.varDynamic', { name }) : t('req.varOk', { name, value: info.value }),
        )
        .join('\n'),
    };
  }
</script>

{#if bulk}
  <div class="bulk-bar">
    <span class="muted hint">{t('req.bulkHint')}</span>
    <span class="spacer"></span>
    <button class="ghost small" onclick={toggleBulk}>{t('req.table')}</button>
  </div>
  <textarea class="bulk mono" rows="8" bind:value={bulkText} oninput={bulkInput} spellcheck="false"></textarea>
{:else}
  <table class="kv">
    <thead>
      <tr>
        <th class="check">
          <input
            type="checkbox"
            aria-label={t('req.enabled')}
            checked={allOn}
            indeterminate={someOn && !allOn}
            disabled={!rows.length}
            onchange={(e) => toggleAll(e.currentTarget.checked)}
          />
        </th>
        <th>{t('req.key')}</th>
        <th>{t('req.value')}</th>
        <th class="del"><button class="ghost small" onclick={toggleBulk}>{t('req.bulk')}</button></th>
      </tr>
    </thead>
    <tbody bind:this={body}>
      {#each extra as row (row.key)}
        <tr class="extra">
          <td class="check"><input type="checkbox" checked disabled aria-label={t('req.enabled')} /></td>
          <td class="text">{row.key}</td>
          <td class="text">{row.value}</td>
          <td class="note"><span class="pill">{row.note}</span></td>
        </tr>
      {/each}
      {#each rows as row, index (index)}
        {@const vars = varsOf(row.value)}
        <tr class="data" class:off={row.enabled === false}>
          <td class="check">
            <input
              type="checkbox"
              aria-label={t('req.enabled')}
              checked={row.enabled !== false}
              onchange={(e) => ((row.enabled = e.currentTarget.checked), changed())}
            />
          </td>
          <td><input data-field="key" list={keyList} placeholder={t('req.key')} bind:value={row.key} oninput={changed} spellcheck="false" /></td>
          <td>
            <div class="value">
              <input
                data-field="value"
                class:missing={vars.missing}
                list={valueList?.(row.key)}
                placeholder={t('req.value')}
                title={vars.title}
                bind:value={row.value}
                oninput={changed}
                spellcheck="false"
              />
              {#if vars.secret}<span class="pill" title={vars.title}>{t('quick.secret')}</span>{/if}
            </div>
          </td>
          <td class="del"><button class="ghost small" title={t('req.remove')} aria-label={t('req.remove')} onclick={() => remove(index)}>×</button></td>
        </tr>
      {/each}
      <tr class="ghost-row">
        <td class="check"><input type="checkbox" disabled aria-label={t('req.enabled')} /></td>
        <td><input list={keyList} placeholder={t('req.key')} oninput={(e) => addRow(e, 'key')} spellcheck="false" /></td>
        <td><input placeholder={t('req.value')} oninput={(e) => addRow(e, 'value')} spellcheck="false" /></td>
        <td></td>
      </tr>
    </tbody>
  </table>
{/if}

<style>
  .check { width: 28px; }
  .del { width: 76px; text-align: right; }
  .note { text-align: right; }
  .text { padding-left: 12px !important; }
  .value { display: flex; align-items: center; gap: 6px; }
  .value input { flex: 1; min-width: 0; }
  input.missing { color: var(--err); }
  tr.off td input:not([type="checkbox"]) { color: var(--muted); text-decoration: line-through; }
  tr.extra td { color: var(--muted); }
  .ghost-row input:not([type="checkbox"]) { opacity: .7; }
  .bulk-bar { display: flex; align-items: center; margin-bottom: 4px; }
  .bulk { width: 100%; font-size: 12.5px; }
  .hint { font-size: 11px; }
</style>
