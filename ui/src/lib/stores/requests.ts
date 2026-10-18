// Funciones puras sobre peticiones y árboles (sin estado): normalizar, comparar, buscar.
import type { Auth, Collection, Folder, Item, KeyValue, Request } from '../types';
import { normalizeUrl } from '../url';

/** Petición en el editor: `params` y `headers` siempre presentes. */
export type Draft = Request & { params: KeyValue[]; headers: KeyValue[] };

export function emptyRequest(name = ''): Draft {
  return {
    id: '',
    name,
    method: 'GET',
    url: '',
    params: [],
    headers: [],
    auth: { type: 'inherit' },
    body: { mode: 'none' },
    scripts: { pre: '', test: '' },
    settings: { follow_redirects: null, verify_tls: null, timeout_ms: null },
    description: '',
  };
}

const rows = (list: KeyValue[] | undefined): KeyValue[] =>
  (list ?? []).map((row) => ({ key: row.key, value: row.value ?? '', enabled: row.enabled !== false }));

/** Forma canónica del editor: query de la URL en `params`, filas con `enabled` explícito. */
export function normalizeRequest(request: Request): Draft {
  // JSON y no structuredClone: el objeto puede ser un proxy de $state.
  const base: Request = { ...emptyRequest(), ...JSON.parse(JSON.stringify(request)) };
  const { url, params } = normalizeUrl(base.url ?? '', base.params ?? []);
  return {
    id: base.id,
    name: base.name,
    method: (base.method || 'GET').toUpperCase(),
    url,
    params: rows(params),
    headers: rows(base.headers),
    auth: base.auth ?? { type: 'inherit' },
    body: base.body ?? { mode: 'none' },
    scripts: { pre: base.scripts?.pre ?? '', test: base.scripts?.test ?? '' },
    settings: {
      follow_redirects: base.settings?.follow_redirects ?? null,
      verify_tls: base.settings?.verify_tls ?? null,
      timeout_ms: base.settings?.timeout_ms ?? null,
    },
    description: base.description ?? '',
  };
}

/** Huella para detectar cambios sin guardar (orden de claves fijo). */
export function snapshot(request: Request): string {
  return JSON.stringify(normalizeRequest(request));
}

/** Carpetas padre (de fuera hacia dentro) y la petición, o null. */
export function locate(items: Item[], id: string, parents: Folder[] = []): { parents: Folder[]; item: Item } | null {
  for (const item of items) {
    if (item.id === id) return { parents, item };
    if (item.type === 'folder') {
      const found = locate(item.items, id, [...parents, item]);
      if (found) return found;
    }
  }
  return null;
}

export function findRequest(collection: Collection, id: string): Request | null {
  const found = locate(collection.items, id);
  return found?.item.type === 'request' ? found.item : null;
}

/** De dónde sale la auth efectiva de una petición que hereda (como el servidor). */
export function inheritedAuth(
  collection: Collection | null,
  parents: Folder[],
): { auth: Auth; from: { kind: 'folder'; name: string } | { kind: 'collection' } | null } {
  for (const folder of [...parents].reverse()) {
    if (folder.auth && folder.auth.type !== 'inherit') return { auth: folder.auth, from: { kind: 'folder', name: folder.name } };
  }
  if (collection?.auth && collection.auth.type !== 'inherit') return { auth: collection.auth, from: { kind: 'collection' } };
  return { auth: { type: 'none' }, from: null };
}

export interface RequestEntry {
  collectionId: string;
  collectionName: string;
  request: Request;
  path: string;
}

/** Todas las peticiones con su ruta `Colección/Carpeta/Nombre` (paleta, búsqueda). */
export function allRequests(collections: Collection[]): RequestEntry[] {
  const out: RequestEntry[] = [];
  const walk = (collection: Collection, items: Item[], prefix: string) => {
    for (const item of items) {
      const path = `${prefix}/${item.name}`;
      if (item.type === 'folder') walk(collection, item.items, path);
      else out.push({ collectionId: collection.id, collectionName: collection.name, request: item, path });
    }
  };
  for (const collection of collections) walk(collection, collection.items, collection.name);
  return out;
}

/** Lo que /send espera para una pestaña sin guardar (RequestDraft del servidor). */
export function draftPayload(request: Request, collectionId: string | null) {
  const { method, url, params, headers, auth, body, settings, scripts } = request;
  return { method, url, params, headers, auth, body, settings, scripts, collection_id: collectionId };
}
