<script lang="ts">
  // "Gestionar cookies" (fase 4), como en Postman: las cookies del tarro de la app por dominio, para
  // verlas, añadirlas, editarlas (texto Set-Cookie) y borrarlas. Viven en memoria mientras la app
  // está abierta: no se guardan en el workspace.
  import { store } from '../stores/app.svelte';
  import { locale, t } from '../i18n.svelte';
  import type { CookieInfo } from '../types';

  let cookies = $state<CookieInfo[]>([]);
  let filter = $state('');
  let domain = $state('');
  let raw = $state('');
  let editing = $state<CookieInfo | null>(null);
  let error = $state('');

  const groups = $derived.by(() => {
    const needle = filter.trim().toLowerCase();
    const map = new Map<string, CookieInfo[]>();
    for (const cookie of cookies) {
      if (needle && !cookie.domain.includes(needle)) continue;
      const list = map.get(cookie.domain) ?? [];
      list.push(cookie);
      map.set(cookie.domain, list);
    }
    return [...map.entries()];
  });

  $effect(() => {
    void refresh();
  });

  async function refresh(): Promise<void> {
    try {
      cookies = await store.listCookies();
    } catch (e) {
      store.toastError(e);
    }
  }

  function close(): void {
    store.ui.cookiesOpen = false;
  }

  function key(event: KeyboardEvent): void {
    if (event.key === 'Escape') close();
  }

  function edit(cookie: CookieInfo): void {
    editing = cookie;
    domain = cookie.domain;
    raw = cookie.raw;
    error = '';
  }

  function reset(): void {
    editing = null;
    raw = '';
    error = '';
  }

  const nameOf = (text: string) => text.split(';')[0].split('=')[0].trim();
  const pathOf = (text: string) => {
    const attr = text
      .split(';')
      .slice(1)
      .map((part) => part.trim())
      .find((part) => part.toLowerCase().startsWith('path='));
    return attr?.slice(5).trim() || '/';
  };

  async function save(): Promise<void> {
    if (!domain.trim() || !raw.trim()) return;
    try {
      const old = editing;
      cookies = await store.saveCookie(domain, raw);
      // Con otro dominio, nombre o ruta es otra cookie: la de antes se borra.
      const host = domain.trim().replace(/^\./, '').toLowerCase();
      if (old && (old.domain !== host || old.name !== nameOf(raw) || old.path !== pathOf(raw))) {
        cookies = await store.deleteCookies(old.domain, old.path, old.name);
      }
      reset();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  async function remove(cookie: CookieInfo): Promise<void> {
    try {
      cookies = await store.deleteCookies(cookie.domain, cookie.path, cookie.name);
      if (editing === cookie) reset();
    } catch (e) {
      store.toastError(e);
    }
  }

  async function removeDomain(name: string): Promise<void> {
    try {
      cookies = await store.deleteCookies(name);
    } catch (e) {
      store.toastError(e);
    }
  }

  const expiry = (cookie: CookieInfo) =>
    cookie.expires == null
      ? t('cookies.session')
      : new Date(cookie.expires * 1000).toLocaleString(locale.lang, { dateStyle: 'short', timeStyle: 'short' });
</script>

<svelte:window onkeydown={key} />

<div class="overlay" role="presentation" onclick={close}>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="cookies-title" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={() => undefined}>
    <header>
      <h2 id="cookies-title">{t('cookies.title')}</h2>
      <span class="spacer"></span>
      <button class="ghost" onclick={close}>{t('cookies.close')}</button>
    </header>
    <p class="muted small">{t('cookies.hint')}</p>

    <input class="filter" placeholder={t('cookies.filter')} bind:value={filter} aria-label={t('cookies.filter')} />

    <div class="list">
      {#if !groups.length}
        <p class="muted empty">{cookies.length ? t('cookies.noMatch') : t('cookies.empty')}</p>
      {/if}
      {#each groups as [name, list] (name)}
        <section>
          <div class="domain">
            <b class="mono">{name}</b>
            <span class="muted small">{list.length}</span>
            <span class="spacer"></span>
            <button class="ghost small" onclick={() => removeDomain(name)}>{t('cookies.deleteDomain')}</button>
          </div>
          {#each list as cookie (cookie.path + cookie.name)}
            <div class="cookie" class:on={editing === cookie}>
              <button class="ghost name mono" title={cookie.raw} onclick={() => edit(cookie)}>{cookie.name}</button>
              <span class="value mono" title={cookie.value}>{cookie.value}</span>
              <span class="meta muted">{cookie.path} · {expiry(cookie)}{cookie.secure ? ' · Secure' : ''}{cookie.http_only ? ' · HttpOnly' : ''}{cookie.host_only ? '' : ` · ${t('cookies.subdomains')}`}</span>
              <button class="icon ghost" aria-label={t('cookies.delete')} title={t('cookies.delete')} onclick={() => remove(cookie)}>×</button>
            </div>
          {/each}
        </section>
      {/each}
    </div>

    <form class="editor" onsubmit={(e) => (e.preventDefault(), void save())}>
      <b>{editing ? t('cookies.editTitle') : t('cookies.addTitle')}</b>
      <label><span>{t('cookies.domain')}</span><input class="mono" bind:value={domain} placeholder="api.example.com" spellcheck="false" /></label>
      <label><span>{t('cookies.raw')}</span><input class="mono" bind:value={raw} placeholder="sid=abc123; Path=/; HttpOnly" spellcheck="false" /></label>
      {#if error}<p class="err">{error}</p>{/if}
      <div class="actions">
        {#if editing}<button type="button" class="ghost" onclick={reset}>{t('cookies.cancel')}</button>{/if}
        <button type="submit" class="primary" disabled={!domain.trim() || !raw.trim()}>{t('cookies.save')}</button>
      </div>
    </form>
  </div>
</div>

<style>
  .overlay { position: fixed; inset: 0; background: var(--overlay); display: grid; place-items: center; z-index: 60; }
  .dialog { width: min(720px, 94vw); max-height: 88vh; display: flex; flex-direction: column; gap: 10px; background: var(--bg); border: 1px solid var(--border); border-radius: 10px; box-shadow: var(--shadow); padding: 16px 20px 18px; }
  header { display: flex; align-items: center; }
  h2 { margin: 0; font-size: 16px; }
  .spacer { flex: 1; }
  .small { font-size: 12px; }
  p.small { margin: 0; }
  .filter { width: 100%; }
  .list { flex: 1; min-height: 120px; overflow: auto; border: 1px solid var(--border); border-radius: 8px; padding: 6px 10px; }
  .empty { margin: 12px 0; }
  section + section { border-top: 1px solid var(--border); margin-top: 6px; padding-top: 6px; }
  .domain { display: flex; align-items: center; gap: 8px; padding: 4px 0; }
  .cookie { display: grid; grid-template-columns: minmax(80px, 160px) minmax(0, 1fr) auto auto; align-items: center; gap: 10px; padding: 2px 0 2px 12px; border-radius: 6px; }
  .cookie.on { background: var(--code-bg); }
  .name { padding: 2px 4px; text-align: left; color: var(--accent-text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .value { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12px; }
  .meta { font-size: 11px; white-space: nowrap; }
  .editor { display: grid; grid-template-columns: 200px 1fr; gap: 8px 12px; align-items: end; }
  .editor b { grid-column: 1 / -1; font-size: 13px; }
  .editor label { display: flex; flex-direction: column; gap: 4px; }
  .editor label span { color: var(--muted); font-size: 11px; text-transform: uppercase; letter-spacing: .06em; }
  .err { grid-column: 1 / -1; color: var(--err); margin: 0; font-size: 12px; }
  .actions { grid-column: 1 / -1; display: flex; justify-content: flex-end; gap: 8px; }
</style>
