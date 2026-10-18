<script lang="ts">
  // Barra superior (P1): logo, buscador Ctrl+K, entorno ▾, ojo de variables, tema y ajustes.
  import { tick } from 'svelte';
  import { store } from '../stores/app.svelte';
  import { t } from '../i18n.svelte';
  import { THEME_ICON } from '../theme';
  import type { Variable } from '../types';

  const env = $derived(store.activeEnvironment);
  const mode = $derived(store.settings?.theme ?? 'system');
  let revealed = $state<string[]>([]);
  let envButton = $state<HTMLButtonElement>();
  let menu = $state<HTMLDivElement>();

  // Al abrir el menú (clic o Ctrl+E) el foco va al entorno activo: se elige con ↑ ↓ y Enter, Esc cierra.
  $effect(() => {
    if (!store.ui.envMenu) return;
    void tick().then(() =>
      (menu?.querySelector<HTMLElement>('[aria-checked="true"]') ?? menu?.querySelector<HTMLElement>('[role="menuitemradio"]'))?.focus(),
    );
  });

  function pickEnv(id: string | null): void {
    store.ui.envMenu = false;
    envButton?.focus();
    void store.setActiveEnvironment(id);
  }

  function menuKey(event: KeyboardEvent): void {
    const items = [...(menu?.querySelectorAll<HTMLElement>('[role="menuitemradio"]') ?? [])];
    const index = items.indexOf(document.activeElement as HTMLElement);
    const go = (i: number) => items[(i + items.length) % items.length]?.focus();
    switch (event.key) {
      case 'ArrowDown':
        go(index + 1);
        break;
      case 'ArrowUp':
        go(index - 1);
        break;
      case 'Home':
        go(0);
        break;
      case 'End':
        go(items.length - 1);
        break;
      case 'Escape':
        store.ui.envMenu = false;
        envButton?.focus();
        break;
      case 'Tab':
        store.ui.envMenu = false;
        return;
      default:
        return;
    }
    event.preventDefault();
    event.stopPropagation();
  }

  function windowKey(event: KeyboardEvent): void {
    if (event.key === 'Escape' && store.ui.quicklook && !store.dialog) store.ui.quicklook = false;
  }

  function outside(event: MouseEvent): void {
    const target = event.target as HTMLElement;
    if (!target.closest('.env-wrap')) store.ui.envMenu = false;
    if (!target.closest('.quick-wrap')) store.ui.quicklook = false;
  }

  const shown = (scope: string, variable: Variable) => revealed.includes(`${scope}:${variable.key}`);
  function toggleReveal(scope: string, variable: Variable): void {
    const id = `${scope}:${variable.key}`;
    revealed = revealed.includes(id) ? revealed.filter((r) => r !== id) : [...revealed, id];
  }
</script>

<svelte:window onclick={outside} onkeydown={windowKey} />

{#snippet varRows(scope: string, vars: Variable[])}
  {#each vars.filter((v) => v.enabled !== false) as variable (variable.key)}
    <tr>
      <td>{variable.key}</td>
      <td>
        {#if variable.secret && !shown(scope, variable)}
          ••••••••
        {:else}
          {variable.value ?? ''}
        {/if}
        {#if variable.secret}
          <span class="pill">{t('quick.secret')}</span>
          <button
            class="ghost small"
            onclick={() => toggleReveal(scope, variable)}
            title={shown(scope, variable) ? t('quick.hide') : t('quick.show')}
            aria-label={shown(scope, variable) ? t('quick.hide') : t('quick.show')}>👁</button
          >
        {/if}
      </td>
    </tr>
  {:else}
    <tr><td class="muted" colspan="2">{t('quick.empty')}</td></tr>
  {/each}
{/snippet}

<header class="topbar">
  <div class="logo">RunPost<span>.</span></div>
  <button class="search" onclick={() => (store.ui.palette = true)}>
    🔍 <span>{t('top.search')}</span> <kbd>Ctrl+K</kbd>
  </button>
  <div class="spacer"></div>

  <div class="env-wrap">
    <button
      class="env"
      bind:this={envButton}
      title={t('top.envTitle')}
      aria-haspopup="menu"
      aria-expanded={store.ui.envMenu}
      onclick={() => (store.ui.envMenu = !store.ui.envMenu)}
    >
      <span class="dot" class:off={!env}></span>
      {env?.name ?? t('top.envNone')} ▾
    </button>
    {#if store.ui.envMenu}
      <div class="menu" role="menu" tabindex="-1" aria-label={t('top.envTitle')} bind:this={menu} onkeydown={menuKey}>
        <button role="menuitemradio" aria-checked={!env} tabindex="-1" class:sel={!env} onclick={() => pickEnv(null)}>{t('top.envNone')}</button>
        {#each store.environments as item (item.id)}
          <button role="menuitemradio" aria-checked={env?.id === item.id} tabindex="-1" class:sel={env?.id === item.id} onclick={() => pickEnv(item.id)}>
            {item.name}
          </button>
        {/each}
      </div>
    {/if}
  </div>

  <div class="quick-wrap">
    <button class="icon ghost" title={t('top.eye')} aria-label={t('top.eye')} aria-expanded={store.ui.quicklook} onclick={() => (store.ui.quicklook = !store.ui.quicklook)}>👁</button>
    {#if store.ui.quicklook}
      <div class="quicklook">
        <h4>{env ? t('quick.title', { name: env.name }) : t('quick.none')}</h4>
        {#if env}
          <table class="mono"><tbody>{@render varRows('env', env.variables)}</tbody></table>
        {/if}
        <h4 class="second">{t('quick.globals')}</h4>
        <table class="mono"><tbody>{@render varRows('globals', store.globals.variables)}</tbody></table>
      </div>
    {/if}
  </div>

  <button class="icon ghost" title={t('top.theme', { mode: t(`theme.${mode}`) })} onclick={() => store.cycleTheme()}>
    {THEME_ICON[mode]}
  </button>
  <button class="icon ghost" title={t('top.settings')} onclick={() => (store.ui.settings = true)}>⚙</button>
</header>

<style>
  .topbar { height: 44px; flex: none; display: flex; align-items: center; gap: 10px; padding: 0 12px; border-bottom: 1px solid var(--border); background: var(--bg-2); }
  .logo { font-weight: 800; letter-spacing: -.03em; font-size: 15px; }
  .logo span { color: var(--accent); }
  .search { flex: 1; max-width: 420px; display: flex; align-items: center; gap: 8px; background: var(--bg); padding: 5px 10px; color: var(--muted); text-align: left; }
  .search span { overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
  .search kbd { margin-left: auto; }
  .env-wrap, .quick-wrap { position: relative; }
  .env { display: flex; align-items: center; gap: 6px; background: var(--bg); padding: 4px 10px; }
  .dot { width: 7px; height: 7px; border-radius: 50%; background: var(--ok); }
  .dot.off { background: var(--muted); }
  .menu { position: absolute; top: 36px; right: 0; min-width: 200px; background: var(--bg); border: 1px solid var(--border); border-radius: 8px; box-shadow: var(--shadow); padding: 4px; z-index: 40; display: flex; flex-direction: column; }
  .menu button { border: none; text-align: left; padding: 6px 10px; }
  .menu button.sel { box-shadow: inset 2px 0 0 var(--accent); background: var(--bg-3); }
  .menu button:focus-visible { outline: 2px solid var(--focus); outline-offset: -2px; }
  .quicklook { position: absolute; top: 36px; right: 0; width: 380px; max-height: 60vh; overflow: auto; background: var(--bg); border: 1px solid var(--border); border-radius: 8px; box-shadow: var(--shadow); padding: 10px; z-index: 40; }
  .quicklook table { width: 100%; border-collapse: collapse; }
  .quicklook td { padding: 4px 6px; border-top: 1px solid var(--border); word-break: break-all; vertical-align: top; }
  .quicklook td:first-child { width: 38%; color: var(--muted); }
  .quicklook h4 { margin: 4px 6px 6px; font-size: 11px; color: var(--muted); text-transform: uppercase; letter-spacing: .08em; }
  .quicklook h4.second { margin-top: 12px; }
</style>
