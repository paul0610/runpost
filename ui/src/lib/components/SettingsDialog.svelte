<script lang="ts">
  // Ajustes en una sola página (P6): lo visible siempre arriba; el resto, plegado en "Avanzado".
  import { store } from '../stores/app.svelte';
  import { t } from '../i18n.svelte';
  import { pickFiles, revealWorkspace } from '../native';
  import { isTauri } from '../transport';
  import type { ClientCert, Settings, ThemeMode } from '../types';

  const settings = $derived(store.settings);
  let advanced = $state(false);
  // Certificados de cliente (fase 4): rutas a los PEM; la clave nunca se copia.
  let certsOpen = $state(false);
  let cert = $state<ClientCert>({ host: '', port: null, cert: '', key: '' });
  const certs = $derived(settings?.http.certificates ?? []);
  const certReady = $derived(cert.host.trim() !== '' && cert.cert.trim() !== '' && cert.key.trim() !== '');

  async function pick(field: 'cert' | 'key'): Promise<void> {
    const picked = await pickFiles([{ name: 'PEM', extensions: ['pem', 'crt', 'cer', 'key'] }], false);
    if (picked?.kind === 'paths' && picked.paths[0]) cert[field] = picked.paths[0];
  }

  function addCert(): void {
    if (!certReady) return;
    const added: ClientCert = { host: cert.host.trim(), cert: cert.cert.trim(), key: cert.key.trim() };
    if (cert.port) added.port = cert.port;
    update((s) => (s.http.certificates = [...(s.http.certificates ?? []), added]));
    cert = { host: '', port: null, cert: '', key: '' };
  }

  function removeCert(index: number): void {
    update((s) => (s.http.certificates = (s.http.certificates ?? []).filter((_, i) => i !== index)));
  }

  function update(change: (s: Settings) => void): void {
    void store.updateSettings(change);
  }

  const int = (value: string, fallback: number) => {
    const n = Number.parseInt(value, 10);
    return Number.isFinite(n) && n > 0 ? n : fallback;
  };

  function key(event: KeyboardEvent): void {
    if (event.key === 'Escape') store.ui.settings = false;
  }
</script>

<svelte:window onkeydown={key} />

{#if settings}
  <div class="overlay" role="presentation" onclick={() => (store.ui.settings = false)}>
    <div class="dialog" role="dialog" aria-modal="true" aria-label={t('settings.title')} tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={() => undefined}>
      <header>
        <h2>{t('settings.title')}</h2>
        <span class="spacer"></span>
        <button class="ghost" onclick={() => (store.ui.settings = false)}>{t('settings.close')}</button>
      </header>
      <div class="grid">
        <label for="s-theme">{t('settings.theme')}</label>
        <select id="s-theme" value={settings.theme} onchange={(e) => update((s) => (s.theme = e.currentTarget.value as ThemeMode))}>
          <option value="light">{t('theme.light')}</option>
          <option value="dark">{t('theme.dark')}</option>
          <option value="system">{t('theme.system')}</option>
        </select>

        <label for="s-lang">{t('settings.language')}</label>
        <select id="s-lang" value={settings.ui.lang} onchange={(e) => update((s) => (s.ui.lang = e.currentTarget.value))}>
          <option value="es">{t('lang.es')}</option>
          <option value="en">{t('lang.en')}</option>
        </select>

        <label for="s-env">{t('settings.env')}</label>
        <select id="s-env" value={store.activeEnvironment?.id ?? ''} onchange={(e) => update((s) => (s.active_environment = e.currentTarget.value || null))}>
          <option value="">{t('top.envNone')}</option>
          {#each store.environments as env (env.id)}<option value={env.id}>{env.name}</option>{/each}
        </select>

        <label for="s-tls">{t('settings.verifyTls')}</label>
        <div>
          <input id="s-tls" type="checkbox" checked={settings.http.verify_tls} onchange={(e) => update((s) => (s.http.verify_tls = e.currentTarget.checked))} />
          {#if !settings.http.verify_tls}<p class="warn">⚠ {t('settings.tlsWarning')}</p>{/if}
        </div>

        <label for="s-timeout">{t('settings.timeout')}</label>
        <input
          id="s-timeout"
          type="number"
          min="1"
          value={Math.round(settings.http.timeout_ms / 1000)}
          onchange={(e) => update((s) => (s.http.timeout_ms = int(e.currentTarget.value, 30) * 1000))}
        />

        <label for="s-font">{t('settings.fontSize')}</label>
        <input id="s-font" type="number" min="10" max="20" value={settings.ui.font_size} onchange={(e) => update((s) => (s.ui.font_size = Math.min(20, Math.max(10, int(e.currentTarget.value, 13)))))} />

        <span class="label">{t('settings.workspace')}</span>
        <div class="ws">
          <code>{store.backend?.workspace}</code>
          {#if isTauri()}
            <button onclick={() => revealWorkspace()}>{t('settings.open')}</button>
          {:else}
            <span class="muted small">{t('settings.openBrowser')}</span>
          {/if}
        </div>
      </div>

      <div class="about-row">
        <button class="ghost link" onclick={() => ((store.ui.settings = false), (store.ui.about = true))}>{t('settings.about')}</button>
      </div>

      <button class="advanced ghost" aria-expanded={advanced} onclick={() => (advanced = !advanced)}>{advanced ? '▾' : '▸'} {t('settings.advanced')}</button>
      {#if advanced}
        <div class="grid">
          <label for="s-follow">{t('settings.follow')}</label>
          <input id="s-follow" type="checkbox" checked={settings.http.follow_redirects} onchange={(e) => update((s) => (s.http.follow_redirects = e.currentTarget.checked))} />

          <label for="s-max">{t('settings.maxRedirects')}</label>
          <input id="s-max" type="number" min="0" value={settings.http.max_redirects} onchange={(e) => update((s) => (s.http.max_redirects = Math.max(0, Number.parseInt(e.currentTarget.value, 10) || 0)))} />

          <label for="s-proxy">{t('settings.proxy')}</label>
          <input id="s-proxy" class="mono" placeholder={t('settings.proxyPlaceholder')} value={settings.http.proxy ?? ''} onchange={(e) => update((s) => (s.http.proxy = e.currentTarget.value.trim() || null))} />

          <label for="s-ua">{t('settings.userAgent')}</label>
          <input id="s-ua" class="mono" value={settings.http.user_agent} onchange={(e) => update((s) => (s.http.user_agent = e.currentTarget.value))} />

          <label for="s-body">{t('settings.bodyMax')}</label>
          <input id="s-body" type="number" min="100" value={settings.mcp.body_max_chars} onchange={(e) => update((s) => (s.mcp.body_max_chars = int(e.currentTarget.value, 2000)))} />

          <label for="s-mask">{t('settings.mask')}</label>
          <div><input id="s-mask" type="checkbox" checked disabled /> <span class="muted small">{t('settings.maskInfo')}</span></div>

          <label for="s-cache">{t('settings.cache')}</label>
          <input id="s-cache" type="number" min="0" value={settings.mcp.cache_responses} onchange={(e) => update((s) => (s.mcp.cache_responses = Math.max(0, Number.parseInt(e.currentTarget.value, 10) || 0)))} />
        </div>
      {/if}

      <button class="advanced ghost" aria-expanded={certsOpen} onclick={() => (certsOpen = !certsOpen)}>
        {certsOpen ? '▾' : '▸'} {t('certs.title')}{certs.length ? ` (${certs.length})` : ''}
      </button>
      {#if certsOpen}
        <p class="muted small hint">{t('certs.hint')}</p>
        {#each certs as item, i (i)}
          <div class="cert">
            <span class="mono host">{item.host}{item.port ? `:${item.port}` : ''}</span>
            <span class="muted small path" title={item.cert}>{item.cert}</span>
            <span class="muted small path" title={item.key}>{item.key}</span>
            <button class="icon ghost" aria-label={t('certs.remove')} title={t('certs.remove')} onclick={() => removeCert(i)}>×</button>
          </div>
        {/each}
        <div class="cert-form">
          <label><span>{t('certs.host')}</span><input class="mono" bind:value={cert.host} placeholder="api.example.com" spellcheck="false" /></label>
          <label>
            <span>{t('certs.port')}</span>
            <input type="number" min="1" max="65535" value={cert.port ?? ''} onchange={(e) => (cert.port = Number.parseInt(e.currentTarget.value, 10) || null)} />
          </label>
          <label class="wide">
            <span>{t('certs.cert')}</span>
            <span class="pick">
              <input class="mono" bind:value={cert.cert} placeholder="C:\certs\client.crt" spellcheck="false" />
              {#if isTauri()}<button onclick={() => pick('cert')}>{t('certs.choose')}</button>{/if}
            </span>
          </label>
          <label class="wide">
            <span>{t('certs.key')}</span>
            <span class="pick">
              <input class="mono" bind:value={cert.key} placeholder="C:\certs\client.key" spellcheck="false" />
              {#if isTauri()}<button onclick={() => pick('key')}>{t('certs.choose')}</button>{/if}
            </span>
          </label>
          <div class="wide add"><button class="primary" disabled={!certReady} onclick={addCert}>{t('certs.add')}</button></div>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .overlay { position: fixed; inset: 0; background: var(--overlay); display: grid; place-items: center; z-index: 60; }
  .dialog { width: min(640px, 94vw); max-height: 88vh; overflow: auto; background: var(--bg); border: 1px solid var(--border); border-radius: 10px; box-shadow: var(--shadow); padding: 16px 20px 20px; }
  header { display: flex; align-items: center; margin-bottom: 8px; }
  h2 { margin: 0; font-size: 16px; }
  .grid { display: grid; grid-template-columns: 220px 1fr; gap: 10px 16px; align-items: center; }
  .grid label, .label { color: var(--muted); }
  .grid input[type="number"] { width: 110px; }
  .ws { display: flex; gap: 8px; align-items: center; min-width: 0; }
  .ws code { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .advanced { display: block; margin: 16px 0 8px; padding-left: 0; font-weight: 600; }
  .about-row { margin-top: 12px; }
  .link { padding-left: 0; color: var(--accent-text); text-decoration: underline; }
  .warn { color: var(--warn); margin: 4px 0 0; font-size: 12px; }
  .small { font-size: 12px; }
  .hint { margin: 0 0 8px; line-height: 1.5; }
  .cert { display: grid; grid-template-columns: 160px minmax(0, 1fr) minmax(0, 1fr) auto; gap: 8px; align-items: center; padding: 4px 0; border-bottom: 1px solid var(--border); }
  .cert .host { overflow: hidden; text-overflow: ellipsis; }
  .path { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; direction: rtl; text-align: left; }
  .cert-form { display: grid; grid-template-columns: 1fr 140px; gap: 8px 12px; margin-top: 10px; }
  .cert-form label { display: flex; flex-direction: column; gap: 4px; }
  .cert-form label > span:first-child { color: var(--muted); font-size: 11px; text-transform: uppercase; letter-spacing: .06em; }
  .cert-form .wide { grid-column: 1 / -1; }
  .pick { display: flex; gap: 6px; }
  .pick input { flex: 1; min-width: 0; }
  .add { display: flex; justify-content: flex-end; }
</style>
