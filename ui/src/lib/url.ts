// Params sincronizados con la query de la URL (sección 18): editar la URL actualiza la tabla y viceversa.
// Modelo del editor: `url` nativa sin query + `params` (todas las filas, activas o no).
import type { KeyValue } from './types';

export const isEnabled = (row: { enabled?: boolean }): boolean => row.enabled !== false;

/** `"a=1&b"` → filas activas. Sin decodificar: igual que el "raw" de Postman. */
export function parseQuery(query: string): KeyValue[] {
  return query
    .split('&')
    .filter((part) => part.length > 0)
    .map((part) => {
      const at = part.indexOf('=');
      return at < 0 ? { key: part, value: '' } : { key: part.slice(0, at), value: part.slice(at + 1) };
    });
}

/** Separa `base?query#hash`. El `#` se conserva en la base para no perderlo. */
export function splitUrl(text: string): { base: string; query: string | null } {
  const hash = text.indexOf('#');
  const head = hash < 0 ? text : text.slice(0, hash);
  const tail = hash < 0 ? '' : text.slice(hash);
  const question = head.indexOf('?');
  if (question < 0) return { base: head + tail, query: null };
  return { base: head.slice(0, question) + tail, query: head.slice(question + 1) };
}

/** Texto de la barra de URL: base + filas activas. */
export function composeUrl(base: string, params: KeyValue[]): string {
  const active = params.filter((p) => isEnabled(p) && (p.key !== '' || p.value !== ''));
  if (!active.length) return base;
  const query = active.map((p) => (p.value === '' ? p.key : `${p.key}=${p.value}`)).join('&');
  const hash = base.indexOf('#');
  return hash < 0 ? `${base}?${query}` : `${base.slice(0, hash)}?${query}${base.slice(hash)}`;
}

/**
 * El usuario escribió en la barra de URL: la base y las filas activas salen del texto;
 * las filas desactivadas de la tabla se conservan al final.
 */
export function applyUrlText(text: string, params: KeyValue[]): { url: string; params: KeyValue[] } {
  const { base, query } = splitUrl(text);
  const disabled = params.filter((p) => !isEnabled(p));
  return { url: base, params: [...(query === null ? [] : parseQuery(query)), ...disabled] };
}

/** Forma canónica al abrir una petición: la query que venga en `url` pasa a `params`. */
export function normalizeUrl(url: string, params: KeyValue[] = []): { url: string; params: KeyValue[] } {
  const { base, query } = splitUrl(url);
  return { url: base, params: [...(query === null ? [] : parseQuery(query)), ...params] };
}
