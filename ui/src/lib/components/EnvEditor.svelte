<script lang="ts">
  // Gestor de un entorno o de las globales (P3 de docs/UX.md): tabla Variable · Valor · Secreto · 🔒,
  // fila nueva al escribir en la última, ojo para ver un secreto, "Mover al llavero" (R5: el valor sale
  // del archivo y va al llavero del sistema; la fila muestra un candado), Importar y Exportar (con
  // "Quitar secretos" marcado).
  import { tick } from 'svelte';
  import { GLOBALS, store, type EnvTab } from '../stores/app.svelte';
  import { t } from '../i18n.svelte';
  import { openImport } from '../importer';

  let { tab }: { tab: EnvTab } = $props();
  let body = $state<HTMLTableSectionElement>();

  const globals = $derived(tab.envId === GLOBALS);
  const env = $derived(store.environments.find((e) => e.id === tab.envId) ?? null);
  const active = $derived(!globals && store.activeEnvironment?.id === tab.envId);
  const fileSecrets = $derived(tab.draft.variables.filter((v) => v.secret && v.store !== 'keyring' && v.key.trim() !== '').length);

  async function addVariable(field: 'key' | 'value' = 'key', text = ''): Promise<void> {
    tab.draft.variables.push({ key: field === 'key' ? text : '', value: field === 'value' ? text : '', secret: false, enabled: true });
    await tick();
    const inputs = body?.querySelectorAll<HTMLInputElement>(`tr.data input[data-field="${field}"]`);
    const input = inputs?.[inputs.length - 1];
    input?.focus();
    input?.setSelectionRange(text.length, text.length);
  }

  function ghostInput(event: Event & { currentTarget: HTMLInputElement }, field: 'key' | 'value'): void {
    const text = event.currentTarget.value;
    event.currentTarget.value = '';
    void addVariable(field, text);
  }

  function remove(index: number): void {
    tab.draft.variables.splice(index, 1);
    tab.revealed = tab.revealed.filter((i) => i !== index).map((i) => (i > index ? i - 1 : i));
  }

  async function toggleReveal(index: number, focus = false): Promise<void> {
    tab.revealed = tab.revealed.includes(index) ? tab.revealed.filter((i) => i !== index) : [...tab.revealed, index];
    if (!focus) return;
    await tick();
    body?.querySelectorAll<HTMLInputElement>('tr.data')[index]?.querySelector<HTMLInputElement>('input[data-field="value"]')?.focus();
  }

  function setSecret(index: number, secret: boolean): void {
    const variable = tab.draft.variables[index];
    variable.secret = secret;
    if (secret) variable.store = variable.store ?? 'file';
    else delete variable.store;
  }
</script>

<div class="env-editor">
  <header>
    {#if globals}
      <h2>🌐 {t('side.globals')}</h2>
    {:else}
      <input class="name" aria-label={t('env.name')} placeholder={t('env.name')} bind:value={tab.draft.name} />
      <button class="ghost status" class:on={active} title={active ? t('side.active') : t('side.activate')} onclick={() => store.setActiveEnvironment(active ? null : tab.envId)}>
        {active ? '●' : '○'} {active ? t('side.active') : t('side.activate')}
      </button>
    {/if}
    <span class="spacer"></span>
    <button onclick={() => addVariable()}>＋ {t('env.newVariable')}</button>
    <button disabled={fileSecrets === 0} title={t('env.moveAllTitle')} onclick={() => store.moveToKeyring(tab)}>🔒 {t('env.moveAll')}</button>
    <button onclick={openImport}>{t('env.import')}</button>
    <button disabled={globals || !env} onclick={() => env && store.exportEnvironment(env)}>{t('env.export')}</button>
    <button class="primary" disabled={!store.isDirty(tab)} onclick={() => store.saveTab(tab)}>{t('req.save')}</button>
  </header>

  {#if fileSecrets > 0}
    <p class="notice">
      🔐 {fileSecrets === 1 ? t('env.secretsNoticeOne') : t('env.secretsNotice', { count: fileSecrets })} ·
      <button class="link" onclick={() => store.moveToKeyring(tab)}>{t('env.moveOne')}</button>
    </p>
  {/if}

  <div class="table-wrap">
    <table class="kv">
      <thead>
        <tr>
          <th class="check"><span class="sr">{t('req.enabled')}</span></th>
          <th>{t('env.variable')}</th>
          <th>{t('env.value')}</th>
          <th class="secret">{t('env.secret')}</th>
          <th class="lock" title={t('env.keyring')}>🔒</th>
          <th class="del"></th>
        </tr>
      </thead>
      <tbody bind:this={body}>
        {#each tab.draft.variables as variable, index (index)}
          {@const hidden = variable.secret && !tab.revealed.includes(index)}
          <tr class="data" class:off={variable.enabled === false}>
            <td class="check">
              <input type="checkbox" aria-label={t('req.enabled')} checked={variable.enabled !== false} onchange={(e) => (variable.enabled = e.currentTarget.checked)} />
            </td>
            <td><input data-field="key" aria-label={t('env.variable')} placeholder={t('env.variable')} bind:value={variable.key} spellcheck="false" /></td>
            <td>
              <div class="value">
                {#if hidden}
                  <!-- Máscara fija como en el mockup: ni el valor ni su longitud salen en pantalla. -->
                  <button class="masked" class:empty={!variable.value} aria-label={t('env.showValue', { name: variable.key })} onclick={() => toggleReveal(index, true)}>
                    {variable.value ? '••••••••' : t('env.value')}
                  </button>
                {:else}
                  <input
                    data-field="value"
                    aria-label={t('env.value')}
                    placeholder={t('env.value')}
                    bind:value={variable.value}
                    autocomplete="off"
                    spellcheck="false"
                  />
                {/if}
                {#if variable.secret}
                  <button class="ghost small eye" title={hidden ? t('quick.show') : t('quick.hide')} aria-label={hidden ? t('quick.show') : t('quick.hide')} onclick={() => toggleReveal(index)}>👁</button>
                {/if}
              </div>
            </td>
            <td class="secret">
              <input type="checkbox" aria-label={t('env.secret')} checked={variable.secret === true} onchange={(e) => setSecret(index, e.currentTarget.checked)} />
            </td>
            <td class="lock">
              {#if variable.secret && variable.store === 'keyring'}
                <span class="locked" title={t('env.inKeyringTitle')}>🔒 <span class="muted">{t('env.inKeyring')}</span></span>
              {:else if variable.secret}
                <button class="ghost small" title={t('env.moveOneTitle')} onclick={() => store.moveToKeyring(tab, index)}>{t('env.moveOne')}</button>
              {/if}
            </td>
            <td class="del"><button class="ghost small" title={t('req.remove')} aria-label={t('req.remove')} onclick={() => remove(index)}>×</button></td>
          </tr>
        {/each}
        <tr class="ghost-row">
          <td class="check"><input type="checkbox" disabled aria-label={t('req.enabled')} /></td>
          <td><input placeholder={t('env.newVariablePlaceholder')} aria-label={t('env.newVariable')} oninput={(e) => ghostInput(e, 'key')} spellcheck="false" /></td>
          <td><input placeholder={t('env.value')} aria-label={t('env.value')} oninput={(e) => ghostInput(e, 'value')} spellcheck="false" /></td>
          <td></td>
          <td></td>
          <td></td>
        </tr>
      </tbody>
    </table>
  </div>
  <p class="muted hint">{globals ? t('env.globalsHint') : t('env.hint')}</p>
</div>

<style>
  .env-editor { flex: 1; min-height: 0; display: flex; flex-direction: column; padding: 12px 16px; gap: 10px; overflow: auto; }
  header { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
  h2 { margin: 0; font-size: 15px; }
  .name { font-size: 15px; font-weight: 600; min-width: 240px; }
  .status.on { color: var(--ok); }
  .notice { margin: 0; padding: 8px 12px; border: 1px solid var(--border); border-left: 4px solid var(--warn); border-radius: 6px; background: var(--bg-2); }
  .link { border: none; padding: 0; text-decoration: underline; }
  .table-wrap { flex: none; }
  .check { width: 28px; }
  .secret { width: 72px; text-align: center; }
  .lock { width: 140px; text-align: center; }
  .del { width: 32px; text-align: right; }
  .value { display: flex; align-items: center; gap: 4px; }
  .value input { flex: 1; min-width: 0; }
  .masked { flex: 1; min-width: 0; text-align: left; border-color: transparent; padding: 4px 6px; letter-spacing: .1em; }
  .masked.empty { color: var(--muted); letter-spacing: normal; }
  .eye { padding: 0 6px; }
  tr.off td input:not([type="checkbox"]) { color: var(--muted); text-decoration: line-through; }
  .ghost-row input:not([type="checkbox"]) { opacity: .7; }
  .hint { font-size: 12px; margin: 0; }
  .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
</style>
