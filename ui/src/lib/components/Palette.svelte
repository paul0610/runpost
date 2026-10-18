<script lang="ts">
  // Paleta Ctrl+K (9.1): buscar petición, cambiar entorno, cambiar tema, nueva petición, ajustes.
  import { onMount } from 'svelte';
  import { store } from '../stores/app.svelte';
  import { t } from '../i18n.svelte';
  import { methodClass, methodLabel } from '../format';
  import { allRequests } from '../stores/requests';
  import type { ThemeMode } from '../types';

  interface Entry {
    kind: string;
    label: string;
    detail?: string;
    method?: string;
    run: () => void;
  }

  let query = $state('');
  let index = $state(0);
  let input: HTMLInputElement;

  const entries = $derived.by((): Entry[] => {
    const requests: Entry[] = allRequests(store.collections).map((entry) => ({
      kind: t('palette.request'),
      label: entry.request.name,
      detail: entry.path,
      method: entry.request.method,
      run: () => store.openRequest(entry.collectionId, entry.request.id),
    }));
    const envs: Entry[] = [
      { kind: t('palette.env'), label: t('palette.noEnv'), run: () => void store.setActiveEnvironment(null) },
      ...store.environments.map((env) => ({
        kind: t('palette.env'),
        label: t('palette.useEnv', { name: env.name }),
        run: () => void store.setActiveEnvironment(env.id),
      })),
    ];
    const themes: Entry[] = (['light', 'dark', 'system'] as ThemeMode[]).map((mode) => ({
      kind: t('palette.theme'),
      label: t('palette.setTheme', { mode: t(`theme.${mode}`) }),
      run: () => void store.setTheme(mode),
    }));
    const actions: Entry[] = [
      { kind: t('palette.action'), label: t('palette.newRequest'), run: () => void store.newTab() },
      { kind: t('palette.action'), label: t('palette.settings'), run: () => (store.ui.settings = true) },
      { kind: t('palette.action'), label: t('palette.about'), run: () => (store.ui.about = true) },
    ];
    const all = [...requests, ...envs, ...themes, ...actions];
    const words = query.toLowerCase().split(/\s+/).filter(Boolean);
    if (!words.length) return all.slice(0, 50);
    return all
      .filter((entry) => {
        const text = `${entry.kind} ${entry.label} ${entry.detail ?? ''} ${entry.method ?? ''}`.toLowerCase();
        return words.every((word) => text.includes(word));
      })
      .slice(0, 50);
  });

  $effect(() => {
    void query;
    index = 0;
  });

  function close(): void {
    store.ui.palette = false;
  }

  function choose(entry: Entry | undefined): void {
    if (!entry) return;
    close();
    entry.run();
  }

  function key(event: KeyboardEvent): void {
    if (event.key === 'Escape') close();
    else if (event.key === 'ArrowDown') index = Math.min(index + 1, entries.length - 1);
    else if (event.key === 'ArrowUp') index = Math.max(index - 1, 0);
    else if (event.key === 'Enter') choose(entries[index]);
    else return;
    event.preventDefault();
  }

  onMount(() => input.focus());
</script>

<div class="overlay" role="presentation" onclick={close}>
  <div class="palette" role="dialog" aria-label={t('palette.placeholder')} onclick={(e) => e.stopPropagation()} onkeydown={key} tabindex="-1">
    <input bind:this={input} bind:value={query} placeholder={t('palette.placeholder')} aria-label={t('palette.placeholder')} />
    <ul role="listbox">
      {#each entries as entry, i (i)}
        <li role="option" aria-selected={i === index} class:sel={i === index} onmouseenter={() => (index = i)} onclick={() => choose(entry)} onkeydown={() => undefined}>
          <span class="kind">{entry.kind}</span>
          {#if entry.method}<span class="meth {methodClass(entry.method)}">{methodLabel(entry.method)}</span>{/if}
          <span class="label">{entry.label}</span>
          {#if entry.detail}<span class="detail muted">{entry.detail}</span>{/if}
        </li>
      {:else}
        <li class="muted">{t('palette.noResults')}</li>
      {/each}
    </ul>
  </div>
</div>

<style>
  .overlay { position: fixed; inset: 0; background: var(--overlay); display: flex; justify-content: center; align-items: flex-start; padding-top: 80px; z-index: 60; }
  .palette { width: min(640px, 92vw); background: var(--bg); border: 1px solid var(--border); border-radius: 10px; box-shadow: var(--shadow); overflow: hidden; }
  input { width: 100%; border: none; border-bottom: 1px solid var(--border); border-radius: 0; padding: 12px 14px; font-size: 14px; }
  input:focus { outline: none; }
  ul { list-style: none; margin: 0; padding: 4px; max-height: 50vh; overflow: auto; }
  li { display: flex; gap: 10px; align-items: center; padding: 6px 10px; border-radius: 6px; cursor: pointer; white-space: nowrap; overflow: hidden; }
  li.sel { background: var(--bg-3); }
  .kind { width: 72px; flex: none; color: var(--muted); font-size: 11px; text-transform: uppercase; letter-spacing: .05em; }
  .meth { font-weight: 700; font-size: 10px; width: 38px; flex: none; }
  .label { flex: none; }
  .detail { overflow: hidden; text-overflow: ellipsis; font-size: 12px; }
</style>
