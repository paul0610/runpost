<script lang="ts">
  // Auth: heredar / ninguna / bearer / basic / API key (sección 5.4). Si hereda, muestra de quién.
  import { store, type RequestTab } from '../stores/app.svelte';
  import { t } from '../i18n.svelte';
  import { inheritedAuth } from '../stores/requests';
  import type { Auth } from '../types';

  let { tab }: { tab: RequestTab } = $props();

  const auth = $derived(tab.draft.auth);
  const inherited = $derived.by(() => {
    const collection = store.collection(tab.collectionId);
    const located = tab.requestId ? store.locateItem(tab.collectionId ?? '', tab.requestId) : null;
    return inheritedAuth(collection, located?.parents ?? []);
  });

  const typeLabel = (type: Auth['type']): string => t(`auth.${type}`);

  function setType(type: Auth['type']): void {
    const current = tab.draft.auth;
    tab.draft.auth =
      type === 'bearer'
        ? { type, token: current.type === 'bearer' ? current.token : '' }
        : type === 'basic'
          ? { type, username: current.type === 'basic' ? current.username : '', password: current.type === 'basic' ? current.password : '' }
          : type === 'apikey'
            ? { type, key: current.type === 'apikey' ? current.key : '', value: current.type === 'apikey' ? current.value : '', in: current.type === 'apikey' ? current.in : 'header' }
            : { type };
  }
</script>

<div class="auth">
  <label class="field">
    <span>{t('auth.type')}</span>
    <select value={auth.type} onchange={(e) => setType(e.currentTarget.value as Auth['type'])}>
      <option value="inherit">{t('auth.inherit')}</option>
      <option value="none">{t('auth.none')}</option>
      <option value="bearer">{t('auth.bearer')}</option>
      <option value="basic">{t('auth.basic')}</option>
      <option value="apikey">{t('auth.apikey')}</option>
    </select>
  </label>

  <div class="fields">
    {#if auth.type === 'inherit'}
      <p class="muted">
        {#if inherited.from}
          {t('auth.inheritsFrom', {
            from: inherited.from.kind === 'folder' ? t('auth.fromFolder', { name: inherited.from.name }) : t('auth.fromCollection'),
            type: typeLabel(inherited.auth.type),
          })}
        {:else}
          {t('auth.inheritsNothing')}
        {/if}
      </p>
    {:else if auth.type === 'none'}
      <p class="muted">{t('auth.noneHint')}</p>
    {:else if auth.type === 'bearer'}
      <label class="field"><span>{t('auth.token')}</span><input class="mono" bind:value={auth.token} placeholder={'{{accessToken}}'} spellcheck="false" /></label>
    {:else if auth.type === 'basic'}
      <label class="field"><span>{t('auth.username')}</span><input class="mono" bind:value={auth.username} spellcheck="false" /></label>
      <label class="field"><span>{t('auth.password')}</span><input class="mono" type="password" bind:value={auth.password} spellcheck="false" /></label>
    {:else if auth.type === 'apikey'}
      <label class="field"><span>{t('auth.key')}</span><input class="mono" bind:value={auth.key} placeholder="X-Api-Key" spellcheck="false" /></label>
      <label class="field"><span>{t('auth.value')}</span><input class="mono" bind:value={auth.value} spellcheck="false" /></label>
      <label class="field">
        <span>{t('auth.addTo')}</span>
        <select bind:value={auth.in}>
          <option value="header">{t('auth.header')}</option>
          <option value="query">{t('auth.query')}</option>
        </select>
      </label>
    {/if}
  </div>
</div>

<style>
  .auth { display: grid; grid-template-columns: 240px 1fr; gap: 18px; align-items: start; }
  .field { display: flex; flex-direction: column; gap: 4px; margin-bottom: 10px; }
  .field span { color: var(--muted); font-size: 11px; text-transform: uppercase; letter-spacing: .06em; }
  .fields { max-width: 520px; }
  .fields p { margin: 4px 0; line-height: 1.5; }
</style>
