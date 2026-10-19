<script lang="ts">
  // Buscar y reemplazar en una colección (fase 4), como en Postman: nombres, URL, parámetros,
  // cabeceras, cuerpo, auth, scripts, descripciones y variables. Las credenciales salen como ***.
  import { store } from '../stores/app.svelte';
  import { t } from '../i18n.svelte';
  import type { FindMatch, FindOptions } from '../types';

  let { collectionId }: { collectionId: string } = $props();

  let query = $state('');
  let replacement = $state('');
  let options = $state<FindOptions>({ case_sensitive: false, whole_word: false, regex: false });
  let matches = $state<FindMatch[]>([]);
  let picked = $state<Set<string>>(new Set());
  let error = $state('');
  let searched = $state(false);
  let input = $state<HTMLInputElement>();

  const collection = $derived(store.collection(collectionId));
  const keyOf = (m: FindMatch) => `${m.item_id}|${m.field}`;
  const total = $derived(matches.reduce((sum, m) => sum + m.count, 0));

  $effect(() => {
    input?.focus();
  });

  // Busca mientras se escribe (con una pausa) y al cambiar las opciones.
  $effect(() => {
    const text = query;
    const opts = { ...options };
    if (!text) {
      matches = [];
      searched = false;
      error = '';
      return;
    }
    let cancelled = false;
    const timer = setTimeout(async () => {
      try {
        const found = await store.findInCollection(collectionId, text, opts);
        if (cancelled) return;
        matches = found;
        picked = new Set(found.map(keyOf));
        error = '';
        searched = true;
      } catch (e) {
        if (!cancelled) {
          matches = [];
          error = e instanceof Error ? e.message : String(e);
        }
      }
    }, 250);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  });

  function close(): void {
    store.ui.findIn = null;
  }

  function key(event: KeyboardEvent): void {
    // Con un diálogo de confirmación encima, Escape solo cierra ese.
    if (event.key === 'Escape' && !event.defaultPrevented && !store.dialog) close();
  }

  function toggle(match: FindMatch): void {
    const next = new Set(picked);
    const k = keyOf(match);
    if (next.has(k)) next.delete(k);
    else next.add(k);
    picked = next;
  }

  async function replace(all: boolean): Promise<void> {
    const targets = all ? undefined : matches.filter((m) => picked.has(keyOf(m))).map((m) => ({ item_id: m.item_id, field: m.field }));
    if (targets && !targets.length) return;
    const count = all ? total : matches.filter((m) => picked.has(keyOf(m))).reduce((sum, m) => sum + m.count, 0);
    if (!(await store.askConfirm(t('find.confirmTitle', { count }), t('find.confirmBody'), t('find.replace'), true))) return;
    try {
      const done = await store.replaceInCollection(collectionId, query, replacement, options, targets);
      store.toast('ok', t('find.replaced', { count: done }));
      // Vuelve a buscar: lo reemplazado ya no aparece.
      matches = await store.findInCollection(collectionId, query, options);
      picked = new Set(matches.map(keyOf));
    } catch (e) {
      store.toastError(e);
    }
  }

  const fieldLabel = (field: string) => t(`find.field.${field}`);

  /** Nombre y carpetas de una coincidencia, sin la colección (ya está en el título). */
  function where(match: FindMatch): { name: string; parents: string } {
    const prefix = `${collection?.name ?? ''}/`;
    const path = match.path.startsWith(prefix) ? match.path.slice(prefix.length) : match.path;
    const cut = path.lastIndexOf('/');
    return cut === -1 ? { name: path, parents: '' } : { name: path.slice(cut + 1), parents: path.slice(0, cut) };
  }

  const escapeRegExp = (text: string) => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

  /** La vista previa con lo encontrado resaltado (con expresión regular no: su sintaxis es la de Rust). */
  function pieces(text: string): { text: string; hit: boolean }[] {
    if (options.regex || !query || text === '***') return [{ text, hit: false }];
    const out: { text: string; hit: boolean }[] = [];
    let from = 0;
    for (const found of text.matchAll(new RegExp(escapeRegExp(query), options.case_sensitive ? 'g' : 'gi'))) {
      const at = found.index ?? 0;
      if (at > from) out.push({ text: text.slice(from, at), hit: false });
      out.push({ text: found[0], hit: true });
      from = at + found[0].length;
    }
    if (from < text.length) out.push({ text: text.slice(from), hit: false });
    return out;
  }
</script>

<svelte:window onkeydown={key} />

<div class="overlay" role="presentation" onclick={close}>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="find-title" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={() => undefined}>
    <header>
      <h2 id="find-title">{t('find.title')}</h2>
      <span class="muted name">{collection?.name ?? ''}</span>
      <span class="spacer"></span>
      <button class="ghost" onclick={close}>{t('find.close')}</button>
    </header>
    <div class="inputs">
      <label><span>{t('find.find')}</span><input bind:this={input} class="mono" bind:value={query} spellcheck="false" /></label>
      <label><span>{t('find.with')}</span><input class="mono" bind:value={replacement} spellcheck="false" /></label>
    </div>
    <div class="options">
      <label><input type="checkbox" bind:checked={options.case_sensitive} /> {t('find.caseSensitive')}</label>
      <label><input type="checkbox" bind:checked={options.whole_word} /> {t('find.wholeWord')}</label>
      <label><input type="checkbox" bind:checked={options.regex} /> {t('find.regex')}</label>
      <span class="spacer"></span>
      {#if searched}<span class="muted small">{t('find.count', { count: total, fields: matches.length })}</span>{/if}
    </div>
    {#if error}<p class="err">{error}</p>{/if}
    <div class="list">
      {#if searched && !matches.length}<p class="muted empty">{t('find.none')}</p>{/if}
      {#each matches as match (keyOf(match))}
        {@const place = where(match)}
        <label class="match" title={match.path}>
          <input type="checkbox" checked={picked.has(keyOf(match))} onchange={() => toggle(match)} />
          <span class="where">
            <span class="path">{place.name}</span>
            <span class="field">{fieldLabel(match.field)} · {match.count}{place.parents ? ` · ${place.parents}` : ''}</span>
          </span>
          <code class="preview">{#each pieces(match.preview) as piece, index (index)}{#if piece.hit}<mark>{piece.text}</mark>{:else}{piece.text}{/if}{/each}</code>
        </label>
      {/each}
    </div>
    <div class="actions">
      <button disabled={!matches.length || !picked.size} onclick={() => replace(false)}>{t('find.replacePicked')}</button>
      <button class="primary" disabled={!matches.length} onclick={() => replace(true)}>{t('find.replaceAll')}</button>
    </div>
  </div>
</div>

<style>
  .overlay { position: fixed; inset: 0; background: var(--overlay); display: grid; place-items: center; z-index: 60; }
  .dialog { width: min(760px, 94vw); max-height: 88vh; display: flex; flex-direction: column; gap: 10px; background: var(--bg); border: 1px solid var(--border); border-radius: 10px; box-shadow: var(--shadow); padding: 16px 20px 18px; }
  header { display: flex; align-items: baseline; gap: 10px; }
  h2 { margin: 0; font-size: 16px; }
  .name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .spacer { flex: 1; }
  .inputs { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; }
  .inputs label { display: flex; flex-direction: column; gap: 4px; }
  .inputs label span { color: var(--muted); font-size: 11px; text-transform: uppercase; letter-spacing: .06em; }
  .options { display: flex; gap: 16px; align-items: center; flex-wrap: wrap; font-size: 13px; }
  .options label { display: flex; gap: 6px; align-items: center; }
  .small { font-size: 12px; }
  .err { color: var(--err); margin: 0; font-size: 12px; }
  .list { flex: 1; min-height: 140px; overflow: auto; border: 1px solid var(--border); border-radius: 8px; }
  .empty { margin: 14px; }
  .match { display: grid; grid-template-columns: auto minmax(160px, 1fr) minmax(0, 1.4fr); gap: 10px; align-items: center; padding: 6px 12px; cursor: pointer; }
  .match + .match { border-top: 1px solid var(--border); }
  .where { display: flex; flex-direction: column; min-width: 0; }
  .path { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .field { color: var(--muted); font-size: 11px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .preview { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12px; color: var(--muted); }
  .preview mark { background: var(--selection); color: var(--fg); border-radius: 2px; }
  .actions { display: flex; justify-content: flex-end; gap: 8px; }
</style>
