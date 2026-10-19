<script lang="ts">
  // "Token actual" de OAuth 2.0, como en Postman, para la auth efectiva de la pestaña (propia o
  // heredada): estado, "Obtener token nuevo" y "Borrar token". El token nunca llega a la UI: el
  // servidor solo dice su tipo, caducidad, scope y si se renueva solo.
  import { store, type RequestTab } from '../stores/app.svelte';
  import { locale, t } from '../i18n.svelte';
  import { copyText } from '../native';
  import type { OAuthToken } from '../types';

  let { tab, grant }: { tab: RequestTab; grant: 'oauth2_cc' | 'oauth2_code' } = $props();

  let token = $state<OAuthToken | null>(null);
  let busy = $state(false);
  let signInUrl = $state<string | null>(null);
  let error = $state('');

  const target = () => [$state.snapshot(tab.draft), tab.requestId, tab.collectionId] as const;

  // Estado al abrir y cada vez que cambia la auth o el entorno activo (con una pausa al escribir).
  $effect(() => {
    void $state.snapshot(tab.draft.auth);
    void store.settings?.active_environment;
    const [draft, requestId, collectionId] = target();
    let cancelled = false;
    const timer = setTimeout(async () => {
      try {
        const result = await store.oauth('status', draft, requestId, collectionId);
        if (!cancelled && 'token' in result) {
          token = result.token;
          error = '';
        }
      } catch (e) {
        if (!cancelled) error = e instanceof Error ? e.message : String(e);
      }
    }, 300);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  });

  async function getToken(): Promise<void> {
    busy = true;
    error = '';
    try {
      const [draft, requestId, collectionId] = target();
      const result = await store.oauth('get', draft, requestId, collectionId);
      if ('flow' in result) {
        signInUrl = result.authorization_url;
        token = await store.oauthWait(result.flow);
      } else {
        token = result.token;
      }
      store.toast('ok', t('auth.tokenSaved'));
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
      signInUrl = null;
    }
  }

  async function clearToken(): Promise<void> {
    try {
      const [draft, requestId, collectionId] = target();
      await store.oauth('clear', draft, requestId, collectionId);
      token = null;
      store.toast('ok', t('auth.tokenCleared'));
    } catch (e) {
      store.toastError(e);
    }
  }

  async function copyLink(): Promise<void> {
    if (!signInUrl) return;
    try {
      await copyText(signInUrl);
      store.toast('ok', t('auth.linkCopied'));
    } catch {
      store.toast('err', t('code.copyFailed'));
    }
  }

  const expired = $derived(token?.expires_at != null && token.expires_at <= Date.now());
  const until = $derived(
    token?.expires_at != null
      ? new Date(token.expires_at).toLocaleString(locale.lang, { dateStyle: 'short', timeStyle: 'short' })
      : '',
  );
</script>

<section class="token" aria-label={t('auth.currentToken')}>
  <h4>{t('auth.currentToken')}</h4>
  {#if busy && signInUrl}
    <p class="waiting">{t('auth.waiting')}</p>
    <button class="small ghost" onclick={copyLink}>{t('auth.copyLink')}</button>
  {:else if !token}
    <p class="muted">{grant === 'oauth2_cc' ? t('auth.noTokenCc') : t('auth.noToken')}</p>
  {:else if expired}
    <p class="warn">{t('auth.tokenExpired')} {token.refresh || grant === 'oauth2_cc' ? t('auth.tokenRenews') : ''}</p>
  {:else}
    <p>
      {token.expires_at != null ? t('auth.tokenReady', { type: token.token_type, time: until }) : t('auth.tokenNoExpiry', { type: token.token_type })}
      {token.refresh || grant === 'oauth2_cc' ? t('auth.tokenRenews') : ''}
    </p>
    {#if token.scope}<p class="muted mono scope">{token.scope}</p>{/if}
  {/if}
  {#if error}<p class="err">{error}</p>{/if}
  <div class="actions">
    <button class="primary" disabled={busy} onclick={getToken}>{t('auth.getToken')}</button>
    {#if token}<button class="ghost" disabled={busy} onclick={clearToken}>{t('auth.clearToken')}</button>{/if}
  </div>
</section>

<style>
  .token { margin-top: 6px; padding: 10px 12px; border: 1px solid var(--border); border-radius: 8px; background: var(--code-bg); }
  h4 { margin: 0 0 6px; font-size: 11px; color: var(--muted); text-transform: uppercase; letter-spacing: .06em; font-weight: 600; }
  p { margin: 4px 0; line-height: 1.5; font-size: 13px; }
  .scope { font-size: 12px; }
  .waiting { color: var(--accent-text); }
  .warn { color: var(--warn); }
  .err { color: var(--err); font-size: 12px; }
  .actions { display: flex; gap: 8px; margin-top: 8px; }
  .small { font-size: 12px; }
</style>
