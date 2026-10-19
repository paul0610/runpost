<script lang="ts">
  // Auth (sección 5.4): heredar / ninguna / bearer / basic / API key / digest / OAuth 2.0. Si
  // hereda, muestra de quién. OAuth 2.0 es un solo tipo con su "Tipo de concesión", como en Postman
  // (client credentials o authorization code con PKCE), y el panel del token actual.
  import { store, type RequestTab } from '../stores/app.svelte';
  import { t } from '../i18n.svelte';
  import { inheritedAuth } from '../stores/requests';
  import type { Auth, ClientAuth } from '../types';
  import OAuthTokenPanel from './OAuthTokenPanel.svelte';

  let { tab }: { tab: RequestTab } = $props();

  type Choice = 'inherit' | 'none' | 'bearer' | 'basic' | 'apikey' | 'digest' | 'oauth2';
  const DEFAULT_REDIRECT = 'http://127.0.0.1:7880/callback';

  const auth = $derived(tab.draft.auth);
  const choice = $derived<Choice>(auth.type === 'oauth2_cc' || auth.type === 'oauth2_code' ? 'oauth2' : auth.type);
  const inherited = $derived.by(() => {
    const collection = store.collection(tab.collectionId);
    const located = tab.requestId ? store.locateItem(tab.collectionId ?? '', tab.requestId) : null;
    return inheritedAuth(collection, located?.parents ?? []);
  });
  /** OAuth 2.0 que se usará al enviar (la propia o la heredada). */
  const oauthGrant = $derived.by(() => {
    const effective = auth.type === 'inherit' ? inherited.auth : auth;
    return effective.type === 'oauth2_cc' || effective.type === 'oauth2_code' ? effective.type : null;
  });

  const typeLabel = (type: Auth['type']): string => t(`auth.${type}`);

  /** Lo común de las dos concesiones, para no perderlo al cambiar de una a otra. */
  function oauthFields(current: Auth) {
    const any = current as Partial<Record<string, string>>;
    return {
      token_url: any.token_url ?? '',
      client_id: any.client_id ?? '',
      client_secret: any.client_secret ?? '',
      scope: any.scope ?? '',
      client_auth: ((current.type === 'oauth2_cc' || current.type === 'oauth2_code') && current.client_auth) || ('basic' as ClientAuth),
    };
  }

  function setType(type: Choice): void {
    const current = tab.draft.auth;
    const same = <T extends Auth['type']>(kind: T) => (current.type === kind ? (current as Extract<Auth, { type: T }>) : null);
    switch (type) {
      case 'bearer':
        tab.draft.auth = { type, token: same('bearer')?.token ?? '' };
        break;
      case 'basic':
      case 'digest': {
        const from = same('basic') ?? same('digest');
        tab.draft.auth = { type, username: from?.username ?? '', password: from?.password ?? '' };
        break;
      }
      case 'apikey': {
        const from = same('apikey');
        tab.draft.auth = { type, key: from?.key ?? '', value: from?.value ?? '', in: from?.in ?? 'header' };
        break;
      }
      case 'oauth2':
        if (current.type !== 'oauth2_cc' && current.type !== 'oauth2_code') tab.draft.auth = { type: 'oauth2_cc', ...oauthFields(current) };
        break;
      default:
        tab.draft.auth = { type };
    }
  }

  function setGrant(grant: 'oauth2_cc' | 'oauth2_code'): void {
    const current = tab.draft.auth;
    if (current.type === grant) return;
    const common = oauthFields(current);
    tab.draft.auth =
      grant === 'oauth2_cc'
        ? { type: grant, ...common }
        : { type: grant, auth_url: '', redirect_uri: DEFAULT_REDIRECT, ...common };
  }

  function setClientAuth(value: ClientAuth): void {
    const current = tab.draft.auth;
    if (current.type === 'oauth2_cc' || current.type === 'oauth2_code') current.client_auth = value;
  }
</script>

<div class="auth">
  <div>
    <label class="field">
      <span>{t('auth.type')}</span>
      <select value={choice} onchange={(e) => setType(e.currentTarget.value as Choice)}>
        <option value="inherit">{t('auth.inherit')}</option>
        <option value="none">{t('auth.none')}</option>
        <option value="bearer">{t('auth.bearer')}</option>
        <option value="basic">{t('auth.basic')}</option>
        <option value="apikey">{t('auth.apikey')}</option>
        <option value="digest">{t('auth.digest')}</option>
        <option value="oauth2">{t('auth.oauth2')}</option>
      </select>
    </label>
    {#if auth.type === 'oauth2_cc' || auth.type === 'oauth2_code'}
      <label class="field">
        <span>{t('auth.grant')}</span>
        <select value={auth.type} onchange={(e) => setGrant(e.currentTarget.value as 'oauth2_cc' | 'oauth2_code')}>
          <option value="oauth2_cc">{t('auth.grantCc')}</option>
          <option value="oauth2_code">{t('auth.grantCode')}</option>
        </select>
      </label>
    {/if}
  </div>

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
    {:else if auth.type === 'basic' || auth.type === 'digest'}
      <label class="field"><span>{t('auth.username')}</span><input class="mono" bind:value={auth.username} spellcheck="false" /></label>
      <label class="field"><span>{t('auth.password')}</span><input class="mono" type="password" bind:value={auth.password} spellcheck="false" /></label>
      {#if auth.type === 'digest'}<p class="muted">{t('auth.digestHint')}</p>{/if}
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
    {:else if auth.type === 'oauth2_cc' || auth.type === 'oauth2_code'}
      {#if auth.type === 'oauth2_code'}
        <label class="field"><span>{t('auth.authUrl')}</span><input class="mono" bind:value={auth.auth_url} placeholder="https://id.example.com/authorize" spellcheck="false" /></label>
      {/if}
      <label class="field"><span>{t('auth.tokenUrl')}</span><input class="mono" bind:value={auth.token_url} placeholder="https://id.example.com/oauth/token" spellcheck="false" /></label>
      <label class="field"><span>{t('auth.clientId')}</span><input class="mono" bind:value={auth.client_id} spellcheck="false" /></label>
      <label class="field"><span>{t('auth.clientSecret')}</span><input class="mono" type="password" bind:value={auth.client_secret} placeholder={'{{clientSecret}}'} spellcheck="false" /></label>
      <label class="field"><span>{t('auth.scope')}</span><input class="mono" bind:value={auth.scope} spellcheck="false" /></label>
      {#if auth.type === 'oauth2_code'}
        <label class="field">
          <span>{t('auth.callbackUrl')}</span>
          <input class="mono" bind:value={auth.redirect_uri} placeholder={DEFAULT_REDIRECT} spellcheck="false" />
        </label>
        <p class="muted hint">{t('auth.callbackHint')}</p>
      {/if}
      <label class="field">
        <span>{t('auth.clientAuth')}</span>
        <select value={auth.client_auth ?? 'basic'} onchange={(e) => setClientAuth(e.currentTarget.value as ClientAuth)}>
          <option value="basic">{t('auth.clientAuthBasic')}</option>
          <option value="body">{t('auth.clientAuthBody')}</option>
        </select>
      </label>
    {/if}
    {#if oauthGrant}<OAuthTokenPanel {tab} grant={oauthGrant} />{/if}
  </div>
</div>

<style>
  .auth { display: grid; grid-template-columns: 240px 1fr; gap: 18px; align-items: start; }
  .field { display: flex; flex-direction: column; gap: 4px; margin-bottom: 10px; }
  .field span { color: var(--muted); font-size: 11px; text-transform: uppercase; letter-spacing: .06em; }
  .fields { max-width: 520px; }
  .fields p { margin: 4px 0; line-height: 1.5; }
  .hint { margin-top: -6px !important; margin-bottom: 10px !important; font-size: 12px; }
</style>
