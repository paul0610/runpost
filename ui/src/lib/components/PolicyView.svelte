<script lang="ts">
  // Pestaña Agente de una colección (R4.4 de docs/UX.md): tres controles, no más. Métodos permitidos,
  // qué requiere aprobación (métodos o peticiones concretas) y límite de llamadas por minuto. Solo
  // afecta al agente por MCP; desde la app se puede enviar cualquier petición.
  // SPEC-DEVIATION: S6-D3 — es una pestaña propia, "Agente · <colección>" (clic derecho → Agente…),
  // porque la app no tiene una vista de colección con subpestañas.
  import { store, type PolicyTab } from '../stores/app.svelte';
  import { t } from '../i18n.svelte';
  import { HTTP_METHODS, isMethod } from '../policy';
  import { methodClass } from '../format';
  import { allRequests } from '../stores/requests';

  let { tab }: { tab: PolicyTab } = $props();

  const collection = $derived(store.collection(tab.collectionId));
  const requests = $derived(collection ? allRequests([collection]) : []);
  const pickedRequests = $derived(tab.draft.approval.filter((entry) => !isMethod(entry)));
  const available = $derived(requests.filter((r) => !tab.draft.approval.includes(r.request.id)));

  function toggle(list: 'allow' | 'approval', method: string, on: boolean): void {
    const current = tab.draft[list].filter((m) => m !== method);
    tab.draft[list] = on ? [...current, method] : current;
  }

  function label(id: string): string {
    const entry = requests.find((r) => r.request.id === id);
    return entry ? `${entry.request.method} ${entry.path.split('/').slice(1).join('/')}` : id;
  }

  function addRequest(event: Event & { currentTarget: HTMLSelectElement }): void {
    const id = event.currentTarget.value;
    event.currentTarget.value = '';
    if (id && !tab.draft.approval.includes(id)) tab.draft.approval = [...tab.draft.approval, id];
  }

  function setLimit(event: Event & { currentTarget: HTMLInputElement }): void {
    const text = event.currentTarget.value.trim();
    const value = Number(text);
    tab.draft.limit = text === '' || !Number.isFinite(value) || value <= 0 ? null : Math.round(value);
  }
</script>

<div class="policy-view">
  <header>
    <h2>🤖 {t('policy.title', { name: collection?.name ?? '' })}</h2>
    <span class="spacer"></span>
    <button class="primary" disabled={!store.isDirty(tab) || !collection} onclick={() => store.saveTab(tab)}>{t('req.save')}</button>
  </header>
  <p class="muted intro">{t('policy.intro')}</p>

  <section aria-labelledby="policy-allowed">
    <h3 id="policy-allowed">{t('policy.allowed')}</h3>
    <div class="methods">
      {#each HTTP_METHODS as method (method)}
        <label class="method">
          <input type="checkbox" checked={tab.draft.allow.includes(method)} onchange={(e) => toggle('allow', method, e.currentTarget.checked)} />
          <span class="mono {methodClass(method)}">{method}</span>
        </label>
      {/each}
    </div>
    <p class="hint">{t('policy.allowedHint')}</p>
  </section>

  <section aria-labelledby="policy-approval">
    <h3 id="policy-approval">{t('policy.approval')}</h3>
    <div class="methods">
      {#each HTTP_METHODS as method (method)}
        <label class="method">
          <input type="checkbox" checked={tab.draft.approval.includes(method)} onchange={(e) => toggle('approval', method, e.currentTarget.checked)} />
          <span class="mono {methodClass(method)}">{method}</span>
        </label>
      {/each}
    </div>
    <div class="requests">
      {#each pickedRequests as id (id)}
        <span class="chip mono">
          {label(id)}
          <button class="ghost x" title={t('req.remove')} aria-label={t('req.remove')} onclick={() => (tab.draft.approval = tab.draft.approval.filter((e) => e !== id))}>×</button>
        </span>
      {/each}
      <select aria-label={t('policy.addRequest')} onchange={addRequest} disabled={available.length === 0}>
        <option value="">{t('policy.addRequest')}</option>
        {#each available as entry (entry.request.id)}
          <option value={entry.request.id}>{entry.request.method} {entry.path.split('/').slice(1).join('/')}</option>
        {/each}
      </select>
    </div>
    <p class="hint">{t('policy.approvalHint')}</p>
  </section>

  <section aria-labelledby="policy-limit">
    <h3 id="policy-limit">{t('policy.limit')}</h3>
    <label class="limit">
      <input type="number" min="1" max="1000" step="1" aria-labelledby="policy-limit" value={tab.draft.limit ?? ''} oninput={setLimit} />
      {t('policy.perMinute')}
    </label>
    <p class="hint">{t('policy.limitHint')}</p>
  </section>

  <p class="muted note">{t('policy.stdioNote')}</p>
</div>

<style>
  .policy-view { flex: 1; min-height: 0; display: flex; flex-direction: column; padding: 12px 16px; gap: 14px; overflow: auto; max-width: 760px; }
  header { display: flex; gap: 8px; align-items: center; }
  h2 { margin: 0; font-size: 15px; }
  h3 { margin: 0 0 8px; font-size: 13px; }
  .intro, .note { margin: 0; line-height: 1.5; }
  section { border: 1px solid var(--border); border-radius: 8px; padding: 12px 14px; background: var(--bg-2); }
  .methods { display: flex; flex-wrap: wrap; gap: 6px 14px; }
  .method { display: flex; gap: 6px; align-items: center; cursor: pointer; }
  .method span { font-weight: 700; font-size: 12px; }
  .requests { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; margin-top: 10px; }
  .chip { display: inline-flex; align-items: center; gap: 2px; border: 1px solid var(--border); border-radius: 999px; padding: 1px 4px 1px 10px; background: var(--bg); font-size: 12px; }
  .x { padding: 0 6px; border: none; }
  .requests select { max-width: 340px; }
  .limit { display: flex; gap: 8px; align-items: center; }
  .limit input { width: 90px; }
  .hint { color: var(--muted); font-size: 12px; margin: 8px 0 0; line-height: 1.5; }
</style>
