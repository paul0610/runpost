import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Collection, Request, RunnerItem, RunnerReport, Settings, Variable } from '../types';

const calls: { method: string; path: string; body?: unknown }[] = [];
let collections: Collection[] = [];
/** Respuestas por `"MÉTODO /ruta"` o, para cualquier método, por `"/ruta"`. */
const responses: Record<string, unknown> = {};
const downloadText = vi.fn((_name: string, _text: string, _mime: string) => undefined);

vi.mock('../transport', () => {
  class ApiError extends Error {
    constructor(
      public status: number,
      public code: string,
      message: string,
      public details: Record<string, unknown> = {},
    ) {
      super(message);
    }
  }
  const handle = async (method: string, path: string, body?: unknown) => {
    calls.push({ method, path, body });
    if (path === '/collections' && method === 'GET') return collections.map((c) => ({ id: c.id, name: c.name, slug: c.name, folders: 0, requests: 0 }));
    if (path.startsWith('/collections/') && method === 'GET') return collections.find((c) => path.endsWith(c.id));
    const key = `${method} ${path}` in responses ? `${method} ${path}` : path;
    if (key in responses) {
      const value = responses[key];
      if (value instanceof Error) throw value;
      return value;
    }
    return {};
  };
  return {
    ApiError,
    api: {
      get: (p: string) => handle('GET', p),
      getText: (p: string) => handle('GET', p),
      post: (p: string, b?: unknown) => handle('POST', p, b),
      put: (p: string, b: unknown) => handle('PUT', p, b),
      patch: (p: string, b: unknown) => handle('PATCH', p, b),
      del: (p: string) => handle('DELETE', p),
    },
    backendInfo: async () => ({ port: 1, token: 't', mode: 'browser', workspace: 'ws' }),
    connectWs: () => () => undefined,
    isTauri: () => false,
  };
});
vi.mock('../native', () => ({
  copyText: vi.fn(async () => undefined),
  setWindowTheme: vi.fn(async () => undefined),
  downloadText: (name: string, text: string, mime: string) => downloadText(name, text, mime),
  pickSavePath: vi.fn(async () => null),
}));

import { GLOBALS, makeTab, slug, store, type DocsTab, type EnvTab, type RequestTab } from './app.svelte';
import { emptyRequest } from './requests';

function request(overrides: Partial<Request> = {}): Request {
  return { ...emptyRequest('ping'), id: 'req_1', url: '{{base}}/ping', ...overrides };
}

function collection(req: Request, folderDescription = ''): Collection {
  return {
    schema: 1,
    id: 'col_1',
    name: 'C',
    description: '',
    auth: { type: 'bearer', token: '{{t}}' },
    agent_policy: null,
    variables: [{ key: 'base', value: 'http://h' }],
    scripts: { pre: '', test: '' },
    items: [
      { type: 'folder', id: 'fld_1', name: 'F', description: folderDescription, auth: null, scripts: { pre: '', test: '' }, items: [{ type: 'request', ...req }] },
    ],
  };
}

function tabAt<K extends 'request' | 'env' | 'docs'>(kind: K, index = 0) {
  const tab = store.tabs[index];
  if (tab?.kind !== kind) throw new Error(`la pestaña ${index} no es de tipo ${kind}`);
  return tab as K extends 'request' ? RequestTab : K extends 'env' ? EnvTab : DocsTab;
}

/** El diálogo actual, una vez abierto, con el tipo pedido. */
async function dialogOf<K extends NonNullable<typeof store.dialog>['kind']>(kind: K) {
  await vi.waitFor(() => expect(store.dialog?.kind).toBe(kind));
  const dialog = store.dialog as Extract<NonNullable<typeof store.dialog>, { kind: K }>;
  store.dialog = null;
  return dialog;
}

const DESA = (): { schema: 1; id: string; name: string; variables: Variable[] } => ({
  schema: 1,
  id: 'env_1',
  name: 'DESA',
  variables: [
    { key: 'gatewayUrl', value: 'http://localhost:3080' },
    { key: 'accessToken', value: 'token-falso', secret: true, store: 'file' },
  ],
});

beforeEach(() => {
  calls.length = 0;
  for (const key of Object.keys(responses)) delete responses[key];
  store.tabs = [];
  store.activeTabId = null;
  store.collections = [];
  store.environments = [];
  store.globals = { schema: 1, variables: [] };
  store.settings = null;
  store.dialog = null;
  store.consoleEntries = [];
  store.toasts = [];
  Object.assign(store.runner, {
    collectionId: null,
    folderId: null,
    envId: null,
    iterations: 1,
    delayMs: 0,
    data: null,
    onlyFailures: false,
    runId: null,
    status: 'idle',
    total: 0,
    items: [],
    report: null,
    error: null,
    expanded: [],
  });
  collections = [];
  downloadText.mockClear();
  localStorage.clear();
});

describe('pestañas y cambios sin guardar', () => {
  it('una pestaña recién abierta está limpia; editar la ensucia y deshacer la limpia', () => {
    const tab = makeTab(request(), 'col_1', 'req_1');
    expect(store.isDirty(tab)).toBe(false);
    tab.draft.method = 'POST';
    expect(store.isDirty(tab)).toBe(true);
    tab.draft.method = 'GET';
    expect(store.isDirty(tab)).toBe(false);
  });

  it('la query embebida no cuenta como cambio y queda en Params', () => {
    const tab = makeTab(request({ url: 'http://h/p?a=1', params: [{ key: 'b', value: '2', enabled: false }] }), null, null);
    expect(tab.draft.url).toBe('http://h/p');
    expect(tab.draft.params).toEqual([
      { key: 'a', value: '1', enabled: true },
      { key: 'b', value: '2', enabled: false },
    ]);
    expect(tab.urlText).toBe('http://h/p?a=1');
    expect(store.isDirty(tab)).toBe(false);
  });

  it('URL → Params y Params → URL', () => {
    store.tabs = [makeTab(request(), null, null)];
    const tab = tabAt('request');
    store.setUrlText(tab, '{{base}}/ping?x=1&y=2');
    expect(tab.draft.url).toBe('{{base}}/ping');
    expect(tab.draft.params.map((p) => p.key)).toEqual(['x', 'y']);
    tab.draft.params[1].enabled = false;
    store.paramsChanged(tab);
    expect(tab.urlText).toBe('{{base}}/ping?x=1');
    // Escribir de nuevo conserva la fila desactivada.
    store.setUrlText(tab, '{{base}}/ping?z=3');
    expect(tab.draft.params).toEqual([
      { key: 'z', value: '3', enabled: true },
      { key: 'y', value: '2', enabled: false },
    ]);
  });

  it('abrir una petición ya abierta la activa en vez de duplicarla', () => {
    collections = [collection(request())];
    store.collections = collections;
    store.openRequest('col_1', 'req_1');
    store.openRequest('col_1', 'req_1');
    expect(store.tabs).toHaveLength(1);
    expect(store.activeRequestTab?.requestId).toBe('req_1');
  });

  it('restaura de localStorage peticiones (también las de antes del S4, sin kind), entornos y docs', () => {
    collections = [collection(request())];
    store.collections = collections;
    store.environments = [DESA()];
    store.globals = { schema: 1, variables: [{ key: 'g', value: 'G' }] };
    localStorage.setItem(
      'runpost.tabs',
      JSON.stringify({
        tabs: [
          { id: 'a', collectionId: 'col_1', requestId: 'req_1', draft: request(), saved: '' },
          {
            kind: 'env',
            id: 'b',
            envId: 'env_1',
            draft: {
              name: 'DESA',
              variables: [
                { key: 'gatewayUrl', value: 'http://editada' },
                { key: 'accessToken', value: '', secret: true, store: 'file' },
              ],
            },
          },
          { kind: 'docs', id: 'c', collectionId: 'col_1', itemId: null, draft: 'hola', saved: '' },
          { kind: 'env', id: 'd', envId: GLOBALS },
          { kind: 'env', id: 'e', envId: 'env_borrado' },
        ],
        active: 'b',
      }),
    );
    store.restoreTabs();
    expect(store.tabs.map((tab) => tab.kind)).toEqual(['request', 'env', 'docs', 'env']);
    expect(store.activeTab?.id).toBe('b');
    expect(store.isDirty(tabAt('request', 0))).toBe(false);
    // Lo editado sin guardar sobrevive a la recarga; el secreto vuelve del entorno, no del navegador.
    const env = tabAt('env', 1);
    expect(store.isDirty(env)).toBe(true);
    expect(env.draft.variables.map((v) => v.value)).toEqual(['http://editada', 'token-falso']);
    expect(store.isDirty(tabAt('docs', 2))).toBe(true);
    // Sin cambios: se relee del servidor.
    expect(tabAt('env', 3).draft.variables).toEqual([{ key: 'g', value: 'G', secret: false, enabled: true }]);
    expect(store.isDirty(tabAt('env', 3))).toBe(false);
  });

  it('los valores secretos nunca se guardan en localStorage', () => {
    store.environments = [DESA()];
    store.openEnvironment('env_1');
    store.persist();
    expect(localStorage.getItem('runpost.tabs')).not.toContain('token-falso');
    tabAt('env').draft.variables[0].value = 'http://editada';
    store.persist();
    const stored = localStorage.getItem('runpost.tabs') ?? '';
    expect(stored).toContain('http://editada');
    expect(stored).not.toContain('token-falso');
  });
});

describe('sincronía con el disco (WebSocket)', () => {
  it('refresca pestañas limpias, respeta las editadas y suelta las borradas', async () => {
    collections = [collection(request())];
    store.collections = collections;
    store.openRequest('col_1', 'req_1');
    const clean = tabAt('request');
    const dirty = store.newTab(request(), 'col_1');
    dirty.requestId = 'req_1';
    dirty.draft.method = 'DELETE';

    // Cambio externo (p. ej. upsert_request por MCP).
    collections = [collection(request({ url: '{{base}}/v2/ping' }))];
    await store.onEvent({ type: 'workspace_changed', kind: 'collection', slug: 'c' });
    expect(clean.draft.url).toBe('{{base}}/v2/ping');
    expect(clean.urlText).toBe('{{base}}/v2/ping');
    expect(store.isDirty(clean)).toBe(false);
    expect(dirty.draft.method).toBe('DELETE');
    expect(store.isDirty(dirty)).toBe(true);

    // La petición desaparece: la pestaña queda como no guardada.
    collections = [{ ...collection(request()), items: [] }];
    await store.reloadCollections();
    expect(clean.requestId).toBeNull();
    expect(store.isDirty(clean)).toBe(true);
  });

  it('un entorno cambiado fuera refresca su pestaña si está limpia', async () => {
    store.environments = [DESA()];
    store.openEnvironment('env_1');
    const tab = tabAt('env');
    responses['GET /environments'] = [{ ...DESA(), variables: [{ key: 'gatewayUrl', value: 'http://otro' }] }];
    await store.onEvent({ type: 'workspace_changed', kind: 'environment', slug: 'desa' });
    expect(tab.draft.variables.map((v) => v.value)).toEqual(['http://otro']);
    expect(store.isDirty(tab)).toBe(false);
  });
});

describe('enviar y guardar', () => {
  it('una petición guardada se envía con su id (hereda auth y variables); una nueva, como borrador', async () => {
    responses['/send'] = {
      response: { status: 200, headers: [], ms: 1, size: 0 },
      resolved: { method: 'GET', url: 'http://h/ping', headers: [], unresolved: [] },
      tests: [],
      vars_changed: [],
      history_id: 'h_1',
      console: [],
    };
    responses['/history?limit=100'] = [];
    const saved = makeTab(request(), 'col_1', 'req_1');
    store.tabs = [saved];
    await store.send(saved);
    const sentSaved = calls.find((c) => c.path === '/send')?.body as { request: Record<string, unknown> };
    expect(sentSaved.request.id).toBe('req_1');
    expect(saved.result?.history_id).toBe('h_1');

    calls.length = 0;
    const draft = makeTab(request({ id: '' }), 'col_1', null);
    await store.send(draft);
    const sentDraft = calls.find((c) => c.path === '/send')?.body as { request: Record<string, unknown> };
    expect(sentDraft.request.id).toBeUndefined();
    expect(sentDraft.request.collection_id).toBe('col_1');
    expect(sentDraft.request.url).toBe('{{base}}/ping');
  });

  it('los errores de red quedan en la pestaña', async () => {
    const { ApiError } = await import('../transport');
    responses['/send'] = new ApiError(502, 'connect', 'refused', { url: 'http://h/***', history_id: 'h_9' });
    responses['/history?limit=100'] = [];
    const tab = makeTab(request(), null, null);
    await store.send(tab);
    expect(tab.error).toEqual({ code: 'connect', message: 'refused', url: 'http://h/***', historyId: 'h_9' });
    expect(tab.sending).toBe(false);
  });

  it('guardar una petición existente hace PATCH y la deja limpia', async () => {
    collections = [collection(request())];
    store.collections = collections;
    store.openRequest('col_1', 'req_1');
    const tab = tabAt('request');
    tab.draft.headers.push({ key: 'X-A', value: '1', enabled: true });
    expect(store.isDirty(tab)).toBe(true);
    collections = [collection(request({ headers: [{ key: 'X-A', value: '1' }] }))];
    expect(await store.saveTab(tab)).toBe(true);
    const patch = calls.find((c) => c.method === 'PATCH');
    expect(patch?.path).toBe('/collections/col_1/items/req_1');
    expect((patch?.body as Record<string, unknown>).id).toBeUndefined();
    expect(store.isDirty(tab)).toBe(false);
  });

  it('guardar una pestaña nueva pide destino y crea la petición', async () => {
    collections = [collection(request())];
    store.collections = collections;
    responses['/collections/col_1/items'] = { id: 'req_new' };
    const tab = store.newTab(request({ id: '', name: '' }), null);
    const saving = store.saveTab(tab);
    (await dialogOf('saveAs')).resolve({ collectionId: 'col_1', parentId: 'fld_1', name: 'Nueva' });
    expect(await saving).toBe(true);
    const created = calls.find((c) => c.method === 'POST' && c.path === '/collections/col_1/items');
    expect(created?.body).toMatchObject({ parent_id: 'fld_1', item: { type: 'request', id: '', name: 'Nueva' } });
    expect(tab.requestId).toBe('req_new');
    expect(tab.collectionId).toBe('col_1');
  });

  it('cerrar con cambios pregunta; "Cancelar" no cierra y "No guardar" sí', async () => {
    const tab = store.newTab(request(), null);
    tab.draft.url = 'http://changed';
    const closing = store.closeTab(tab.id);
    (await dialogOf('unsaved')).resolve('cancel');
    await closing;
    expect(store.tabs).toHaveLength(1);
    const again = store.closeTab(tab.id);
    (await dialogOf('unsaved')).resolve('discard');
    await again;
    expect(store.tabs).toHaveLength(0);
  });
});

describe('consola (S4)', () => {
  it('cada envío deja una línea con la URL resuelta, las cabeceras enmascaradas y la respuesta', async () => {
    responses['/send'] = {
      response: { status: 200, headers: [['Content-Type', 'application/json']], ms: 98, size: 1234 },
      resolved: { method: 'GET', url: 'https://api.test/v2/ping', headers: [['Authorization', 'Bearer ***']], unresolved: ['nope'] },
      tests: [],
      vars_changed: [],
      history_id: 'h_1',
      console: ['hola'],
    };
    responses['/history?limit=100'] = [];
    await store.send(makeTab(request(), null, null));
    expect(store.consoleEntries).toHaveLength(1);
    expect(store.consoleEntries[0]).toMatchObject({
      method: 'GET',
      url: 'https://api.test/v2/ping',
      headers: [['Authorization', 'Bearer ***']],
      status: 200,
      ms: 98,
      size: 1234,
      contentType: 'application/json',
      error: null,
      unresolved: ['nope'],
      console: ['hola'],
    });
  });

  it('un error de red enseña el error y las cabeceras que se enviaron (del historial)', async () => {
    const { ApiError } = await import('../transport');
    responses['/send'] = new ApiError(502, 'connect', 'refused', { url: 'http://h/***', history_id: 'h_9' });
    responses['/history/h_9'] = { id: 'h_9', method: 'POST', url: 'http://h/***', req_headers: [['Authorization', 'Bearer ***']] };
    responses['/history?limit=100'] = [];
    await store.send(makeTab(request(), null, null));
    expect(store.consoleEntries[0]).toMatchObject({
      method: 'POST',
      url: 'http://h/***',
      headers: [['Authorization', 'Bearer ***']],
      status: null,
      error: 'connect: refused',
    });
    store.clearConsole();
    expect(store.consoleEntries).toHaveLength(0);
  });

  it('guarda como mucho 200 líneas', () => {
    for (let i = 0; i < 205; i += 1) {
      store.log({ ts: i, method: 'GET', url: `u${i}`, headers: [], status: 200, ms: 1, size: 1, contentType: '', error: null, unresolved: [], console: [] });
    }
    expect(store.consoleEntries).toHaveLength(200);
    expect(store.consoleEntries[0].url).toBe('u5');
  });
});

describe('gestor de entornos (P3)', () => {
  it('abrir dos veces activa la misma pestaña; editar la ensucia', () => {
    store.environments = [DESA()];
    store.openEnvironment('env_1');
    store.openEnvironment('env_1');
    expect(store.tabs).toHaveLength(1);
    const tab = tabAt('env');
    expect(store.tabTitle(tab)).toBe('DESA');
    expect(store.isDirty(tab)).toBe(false);
    tab.draft.variables[0].value = 'http://otro';
    expect(store.isDirty(tab)).toBe(true);
  });

  it('guardar hace PUT del entorno sin las filas sin nombre y lo deja limpio', async () => {
    store.environments = [DESA()];
    store.openEnvironment('env_1');
    const tab = tabAt('env');
    tab.draft.variables.push({ key: '', value: 'x', secret: false, enabled: true });
    tab.draft.variables.push({ key: 'currency', value: 'usd', secret: false, enabled: true });
    responses['GET /environments'] = [{ ...DESA(), variables: [...DESA().variables, { key: 'currency', value: 'usd' }] }];
    expect(await store.saveTab(tab)).toBe(true);
    const put = calls.find((c) => c.method === 'PUT');
    expect(put?.path).toBe('/environments/env_1');
    const body = put?.body as { name: string; variables: Variable[] };
    expect(body.name).toBe('DESA');
    expect(body.variables.map((v) => v.key)).toEqual(['gatewayUrl', 'accessToken', 'currency']);
    expect(body.variables[1]).toMatchObject({ secret: true, store: 'file' });
    expect(store.isDirty(tab)).toBe(false);
  });

  it('las globales se guardan con PUT /globals', async () => {
    store.globals = { schema: 1, variables: [{ key: 'g', value: 'G' }] };
    store.openEnvironment(GLOBALS);
    const tab = tabAt('env');
    expect(store.tabTitle(tab)).toBe('Globales');
    tab.draft.variables[0].value = 'G2';
    responses['GET /globals'] = { schema: 1, variables: [{ key: 'g', value: 'G2' }] };
    expect(await store.saveTab(tab)).toBe(true);
    expect(calls.find((c) => c.method === 'PUT')).toMatchObject({ path: '/globals', body: { schema: 1, variables: [{ key: 'g', value: 'G2' }] } });
    expect(store.isDirty(tab)).toBe(false);
  });

  it('si el entorno se borró fuera, la pestaña queda sin guardar y guardar lo vuelve a crear', async () => {
    store.environments = [DESA()];
    store.openEnvironment('env_1');
    const tab = tabAt('env');
    store.environments = [];
    store.syncEnvTabs();
    expect(store.isDirty(tab)).toBe(true);
    responses['POST /environments'] = { ...DESA(), id: 'env_2' };
    responses['GET /environments'] = [{ ...DESA(), id: 'env_2' }];
    expect(await store.saveTab(tab)).toBe(true);
    expect(calls.find((c) => c.method === 'POST')).toMatchObject({ path: '/environments', body: { environment: { id: '', name: 'DESA' } } });
    expect(tab.envId).toBe('env_2');
    expect(store.isDirty(tab)).toBe(false);
  });

  it('borrar un entorno activo lo desactiva y cierra su pestaña limpia', async () => {
    store.environments = [DESA()];
    store.settings = { active_environment: 'env_1', theme: 'light', ui: { lang: 'es', font_size: 13 } } as unknown as Settings;
    responses['PUT /settings'] = { active_environment: null, theme: 'light', ui: { lang: 'es', font_size: 13 } };
    responses['GET /environments'] = [];
    store.openEnvironment('env_1');
    const deleting = store.deleteEnvironment(store.environments[0]);
    (await dialogOf('confirm')).resolve(true);
    await deleting;
    expect(calls.some((c) => c.method === 'DELETE' && c.path === '/environments/env_1')).toBe(true);
    expect(store.settings?.active_environment).toBeNull();
    expect(store.tabs).toHaveLength(0);
  });
});

describe('documentación (Docs)', () => {
  it('la de una carpeta vacía empieza editando y se guarda con PATCH', async () => {
    collections = [collection(request())];
    store.collections = collections;
    store.openDocs('col_1', 'fld_1');
    const tab = tabAt('docs');
    expect(tab.editing).toBe(true);
    expect(store.tabTitle(tab)).toBe('Documentación · F');
    tab.draft = '# Hola';
    expect(store.isDirty(tab)).toBe(true);
    collections = [collection(request(), '# Hola')];
    expect(await store.saveTab(tab)).toBe(true);
    expect(calls.find((c) => c.method === 'PATCH')).toMatchObject({ path: '/collections/col_1/items/fld_1', body: { description: '# Hola' } });
    expect(store.isDirty(tab)).toBe(false);
    expect(tab.editing).toBe(false);
  });

  it('la de la colección se guarda con PUT de la colección entera', async () => {
    collections = [{ ...collection(request()), description: 'Intro' }];
    store.collections = collections;
    store.openDocs('col_1', null);
    const tab = tabAt('docs');
    expect(tab.editing).toBe(false);
    tab.draft = 'Intro nueva';
    collections = [{ ...collection(request()), description: 'Intro nueva' }];
    expect(await store.saveTab(tab)).toBe(true);
    const put = calls.find((c) => c.method === 'PUT');
    expect(put?.path).toBe('/collections/col_1');
    expect(put?.body).toMatchObject({ id: 'col_1', name: 'C', description: 'Intro nueva' });
  });
});

describe('historial', () => {
  it('abre la petición guardada si todavía existe', async () => {
    collections = [collection(request())];
    store.collections = collections;
    responses['/history/h_1'] = { id: 'h_1', request_id: 'req_1', method: 'GET', url: 'http://h/ping', req_headers: [], req_body: null };
    const tab = await store.openHistory('h_1');
    expect(tab?.requestId).toBe('req_1');
    expect(store.tabs).toHaveLength(1);
  });

  it('si no, abre lo enviado en una pestaña nueva y avisa de los secretos enmascarados', async () => {
    responses['/history/h_2'] = {
      id: 'h_2',
      request_id: null,
      method: 'POST',
      url: 'http://h/api',
      req_headers: [
        ['Authorization', 'Bearer ***'],
        ['User-Agent', 'runpost/0.1'],
        ['Content-Type', 'application/json'],
      ],
      req_body: { text: '{"a":1}', encoding: 'utf8' },
    };
    const tab = await store.openHistory('h_2');
    expect(tab?.requestId).toBeNull();
    expect(tab?.draft.method).toBe('POST');
    expect(tab?.draft.headers.map((h) => h.key)).toEqual(['Authorization', 'Content-Type']);
    expect(tab?.draft.body).toEqual({ mode: 'raw', lang: 'json', text: '{"a":1}' });
    expect(store.toasts.some((toast) => toast.kind === 'info')).toBe(true);
  });

  it('vaciar pide confirmación y hace DELETE /history', async () => {
    responses['/history?limit=100'] = [];
    const clearing = store.clearHistory();
    (await dialogOf('confirm')).resolve(true);
    await clearing;
    expect(calls.some((c) => c.method === 'DELETE' && c.path === '/history')).toBe(true);
  });
});

describe('exportar (navegador)', () => {
  it('pregunta con "Quitar secretos" marcado y descarga la carpeta como colección', async () => {
    collections = [collection(request())];
    store.collections = collections;
    responses['/export/collections/col_1?format=postman_v21&strip_secrets=1&folder=fld_1'] = { info: { name: 'F' } };
    const exporting = store.exportCollection('col_1', 'fld_1');
    const dialog = await dialogOf('export');
    expect(dialog.strip).toBe(true);
    dialog.resolve(true);
    await exporting;
    expect(downloadText).toHaveBeenCalledWith('f.postman_collection.json', expect.stringContaining('"name": "F"'), 'application/json');
  });

  it('cancelar no exporta; desmarcar no manda strip_secrets', async () => {
    store.environments = [{ ...DESA(), name: 'Acme DESA' }];
    const cancelled = store.exportEnvironment(store.environments[0]);
    (await dialogOf('export')).resolve(null);
    await cancelled;
    expect(calls.some((c) => c.path.startsWith('/export'))).toBe(false);
    const exporting = store.exportEnvironment(store.environments[0]);
    (await dialogOf('export')).resolve(false);
    await exporting;
    expect(calls.find((c) => c.path.startsWith('/export'))?.path).toBe('/export/environments/env_1');
    expect(downloadText).toHaveBeenLastCalledWith('acme-desa.postman_environment.json', expect.any(String), 'application/json');
  });

  it('nombre de archivo sin acentos ni símbolos', () => {
    expect(slug('Café Ñandú / v2')).toBe('cafe-nandu-v2');
    expect(slug('***')).toBe('export');
  });
});

describe('variables para el resaltado', () => {
  it('entorno activo, colección y globales; secretos como ***', () => {
    store.collections = [collection(request())];
    store.environments = [{ schema: 1, id: 'env_1', name: 'E', variables: [{ key: 't', value: 's3cr3t', secret: true }] }];
    store.globals = { schema: 1, variables: [{ key: 'g', value: 'G' }] };
    store.settings = { active_environment: 'env_1' } as never;
    const tab = makeTab(request(), 'col_1', 'req_1');
    const describe = store.describerFor(tab);
    expect(describe('base')).toEqual({ ok: true, text: 'base = http://h' });
    expect(describe('t')).toEqual({ ok: true, text: 't = *** (secreto)' });
    expect(describe('g').ok).toBe(true);
    expect(describe('$guid').ok).toBe(true);
    expect(describe('nope').ok).toBe(false);
  });

  it('cambiar de entorno cambia lo resaltado sin recargar', () => {
    store.environments = [
      { schema: 1, id: 'env_1', name: 'DESA', variables: [{ key: 'gatewayUrl', value: 'http://desa' }] },
      { schema: 1, id: 'env_2', name: 'PROD', variables: [{ key: 'gatewayUrl', value: 'http://prod' }] },
    ];
    store.settings = { active_environment: 'env_1' } as never;
    const tab = makeTab(request(), null, null);
    expect(store.describerFor(tab)('gatewayUrl').text).toBe('gatewayUrl = http://desa');
    store.settings = { active_environment: 'env_2' } as never;
    expect(store.describerFor(tab)('gatewayUrl').text).toBe('gatewayUrl = http://prod');
    expect(calls).toHaveLength(0);
  });
});

function runnerItem(name: string, passed: boolean, overrides: Partial<RunnerItem> = {}): RunnerItem {
  return {
    iteration: 0,
    id: `req_${name}`,
    name,
    path: `F/${name}`,
    method: 'GET',
    url: `http://h/${name}`,
    status: 200,
    ms: 5,
    size: 2,
    tests: [{ name: 't', passed, msg: passed ? null : 'AssertionError: x' }],
    error: null,
    skipped: false,
    console: [],
    ...overrides,
  };
}

function report(items: RunnerItem[]): RunnerReport {
  return {
    collection: { id: 'col_1', name: 'C' },
    folder: null,
    environment: null,
    iterations: 1,
    data: null,
    started_at: '2026-10-17T00:00:00.000Z',
    ms: 42,
    cancelled: false,
    totals: { requests: items.length, skipped: 0, errors: 0, tests: items.length, passed: 1, failed: 1 },
    items,
  };
}

describe('runner (S5)', () => {
  it('"Ejecutar" del árbol elige la colección o carpeta y abre la pestaña Runner', () => {
    store.openRunner('col_1', 'fld_1');
    expect(store.runner.collectionId).toBe('col_1');
    expect(store.runner.folderId).toBe('fld_1');
    expect(store.ui.drawer).toBe('runner');
  });

  it('arranca con el payload del formulario y procesa los eventos aunque lleguen antes que el run_id', async () => {
    store.collections = [collection(request())];
    store.runner.data = { name: 'd.csv', text: 'x\n1\n' };
    store.runner.iterations = null;
    store.runner.delayMs = 250;
    responses['POST /runner'] = { run_id: 'run_1' };
    responses['GET /runner/run_1'] = { status: 'done', report: report([runnerItem('a', true), runnerItem('b', false)]) };

    const starting = store.startRun();
    // Evento del WebSocket antes de que vuelva el POST: se guarda y se repite después.
    await store.onEvent({ type: 'runner_progress', run_id: 'run_1', event: 'started', total: 2, iterations: 1 });
    await starting;
    const posted = calls.find((c) => c.method === 'POST' && c.path === '/runner');
    expect(posted?.body).toEqual({
      collection_id: 'col_1',
      folder_id: null,
      env_id: null,
      iterations: null,
      delay_ms: 250,
      data: { name: 'd.csv', text: 'x\n1\n' },
    });
    expect(store.runner.status).toBe('running');
    expect(store.runner.total).toBe(2);

    await store.onEvent({ type: 'runner_progress', run_id: 'run_1', event: 'item', done: 1, total: 2, item: runnerItem('a', true) });
    // Otro run (otra ventana): se ignora.
    await store.onEvent({ type: 'runner_progress', run_id: 'run_x', event: 'item', done: 1, total: 9, item: runnerItem('z', true) });
    expect(store.runner.items.map((i) => i.name)).toEqual(['a']);

    await store.onEvent({ type: 'runner_done', run_id: 'run_1', ms: 42 });
    expect(store.runner.status).toBe('done');
    expect(store.runner.report?.ms).toBe(42);
    expect(store.runner.items.map((i) => i.name)).toEqual(['a', 'b']);
  });

  it('un error al arrancar queda en el panel; cancelar manda DELETE', async () => {
    const { ApiError } = await import('../transport');
    store.collections = [collection(request())];
    responses['POST /runner'] = new ApiError(404, 'folder_not_found', 'folder not found: x');
    await store.startRun();
    expect(store.runner.status).toBe('error');
    expect(store.runner.error).toBe('folder_not_found: folder not found: x');

    responses['POST /runner'] = { run_id: 'run_2' };
    await store.startRun();
    await store.cancelRun();
    expect(calls.some((c) => c.method === 'DELETE' && c.path === '/runner/run_2')).toBe(true);
    await store.onEvent({ type: 'runner_done', run_id: 'run_2', error: 'boom' });
    expect(store.runner.status).toBe('error');
    expect(store.runner.error).toBe('boom');
  });

  it('exporta el informe en el navegador como JSON o HTML', async () => {
    store.runner.runId = 'run_1';
    store.runner.status = 'done';
    store.runner.report = report([runnerItem('a', true)]);
    await store.exportRunReport('json');
    expect(downloadText).toHaveBeenLastCalledWith('runner-c.json', expect.stringContaining('"ms": 42'), 'application/json');
    responses['/runner/run_1/report?format=html'] = '<html>informe</html>';
    await store.exportRunReport('html');
    expect(downloadText).toHaveBeenLastCalledWith('runner-c.html', '<html>informe</html>', 'text/html');
  });
});

describe('scripts al enviar (S5)', () => {
  it('una petición sin guardar manda sus scripts; si un script la salta, se ven los tests y la consola lo dice', async () => {
    responses['/send'] = {
      response: { status: 0, headers: [], ms: 0, size: 0 },
      resolved: { method: 'GET', url: '{{base}}/ping', headers: [], unresolved: [] },
      tests: [{ name: 'before skip', passed: true, msg: null }],
      vars_changed: [],
      history_id: '',
      console: ['skipping'],
      skipped: true,
    };
    responses['/history?limit=100'] = [];
    const draft = makeTab(request({ id: '', scripts: { pre: 'pm.execution.skipRequest()', test: '' } }), 'col_1', null);
    await store.send(draft);
    const sent = calls.find((c) => c.path === '/send')?.body as { request: Record<string, unknown> };
    expect(sent.request.scripts).toEqual({ pre: 'pm.execution.skipRequest()', test: '' });
    expect(draft.respTab).toBe('tests');
    expect(store.consoleEntries[0]).toMatchObject({ skipped: true, status: null, console: ['skipping'] });
  });
});

describe('consola con scripts (S5)', () => {
  it('lista los tests fallidos junto a los console.log', async () => {
    responses['/send'] = {
      response: { status: 401, headers: [], ms: 3, size: 2 },
      resolved: { method: 'POST', url: 'http://h/sign', headers: [], unresolved: [] },
      tests: [
        { name: 'ok', passed: true, msg: null },
        { name: '#7 sign', passed: false, msg: 'AssertionError: RECHAZADO: expected false to be true' },
        { name: 'later', passed: true, msg: null, skipped: true },
      ],
      vars_changed: [],
      history_id: 'h_7',
      console: ['#7 sign -> RECHAZADO'],
    };
    responses['/history?limit=100'] = [];
    await store.send(makeTab(request(), null, null));
    expect(store.consoleEntries[0].failures).toEqual([{ name: '#7 sign', msg: 'AssertionError: RECHAZADO: expected false to be true' }]);
    expect(store.consoleEntries[0].console).toEqual(['#7 sign -> RECHAZADO']);
  });
});

describe('panel Agente (S6)', () => {
  const approval = {
    id: 'apr_1',
    ts: 1,
    expires: Date.now() + 60_000,
    client: 'Claude Code',
    tool: 'run_request',
    method: 'DELETE',
    url: '{{apiUrl}}/v1/usd/accounts/{{accountId}}',
    request: 'Borrar cuenta',
    request_id: 'req_1',
    collection: 'C',
    environment: 'DESA',
    rule: 'require_approval: DELETE',
    count: 1,
  };
  const agentCall = (id: number, raw: number, sent: number) => ({
    id,
    ts: 1,
    tool: 'run_request',
    target: 'ping',
    request_id: 'req_1',
    status: '200',
    ok: true,
    ms: 9,
    raw,
    sent,
    client: 'Claude Code',
    session: 's1',
  });

  beforeEach(() => {
    Object.assign(store.agent, {
      calls: [],
      meter: { calls: 0, raw: 0, sent: 0 },
      clients: [],
      pending: [],
      unseen: 0,
      deciding: [],
    });
    store.ui.drawer = null;
    store.ui.pane = 'collections';
  });

  it('cada llamada suma al medidor y a la insignia hasta que se mira el panel', async () => {
    await store.onEvent({ type: 'agent_call', call: agentCall(1, 1000, 100) });
    await store.onEvent({ type: 'agent_call', call: agentCall(2, 500, 50) });
    await store.onEvent({ type: 'agent_call', call: agentCall(2, 500, 50) });
    expect(store.agent.calls).toHaveLength(2);
    expect(store.agent.meter).toEqual({ calls: 2, raw: 1500, sent: 150 });
    expect(store.agent.unseen).toBe(2);
    store.ui.drawer = 'agent';
    expect(store.agentViewing).toBe(true);
    // Lo que ejecutó el agente aparece también en el Historial.
    expect(calls.some((c) => c.path.startsWith('/history'))).toBe(true);
  });

  it('el aviso de aprobación queda pendiente hasta decidir y se decide por la API', async () => {
    await store.onEvent({ type: 'agent_approval', approval });
    expect(store.agent.pending.map((a) => a.id)).toEqual(['apr_1']);
    await store.decideApproval('apr_1', true);
    expect(calls.at(-1)).toMatchObject({ method: 'POST', path: '/agent/approvals/apr_1', body: { approve: true } });
    expect(store.agent.pending).toHaveLength(0);
  });

  it('si nadie responde en 60 s, el aviso se va y se explica', async () => {
    await store.onEvent({ type: 'agent_approval', approval });
    await store.onEvent({ type: 'agent_approval_done', id: 'apr_1', decision: 'expired' });
    expect(store.agent.pending).toHaveLength(0);
    expect(store.toasts.at(-1)?.text).toContain('60 s');
  });

  it('decidir uno que ya caducó lo quita sin error', async () => {
    const { ApiError } = await import('../transport');
    responses['POST /agent/approvals/apr_1'] = new ApiError(404, 'approval_not_found', 'gone');
    await store.onEvent({ type: 'agent_approval', approval });
    await store.decideApproval('apr_1', false);
    expect(store.agent.pending).toHaveLength(0);
    expect(store.toasts.at(-1)?.kind).toBe('info');
  });

  it('una fila abre su petición como pestaña normal', () => {
    store.collections = [collection(request())];
    store.openAgentCall(agentCall(1, 10, 10));
    expect(tabAt('request').requestId).toBe('req_1');
    store.openAgentCall({ ...agentCall(2, 1, 1), request_id: undefined });
    expect(store.tabs).toHaveLength(1);
  });

  it('carga el estado completo al conectar', async () => {
    responses['GET /agent'] = {
      calls: [agentCall(7, 40, 4)],
      meter: { calls: 1, raw: 40, sent: 4 },
      clients: [],
      pending: [approval],
      mcp: { port: 7879, url: 'http://127.0.0.1:7879/mcp', error: null },
    };
    await store.reloadAgent();
    expect(store.agent.calls[0].id).toBe(7);
    expect(store.agent.pending[0].id).toBe('apr_1');
    expect(store.agent.mcp.url).toBe('http://127.0.0.1:7879/mcp');
  });
});

describe('pestaña Agente de la colección (S6)', () => {
  it('se abre una vez, empieza sin cambios y guarda agent_policy con PUT de la colección', async () => {
    store.collections = [collection(request())];
    store.openPolicy('col_1');
    store.openPolicy('col_1');
    expect(store.tabs).toHaveLength(1);
    const tab = store.tabs[0];
    if (tab.kind !== 'policy') throw new Error('no es la pestaña Agente');
    expect(store.tabTitle(tab)).toBe('Agente · C');
    expect(store.isDirty(tab)).toBe(false);
    tab.draft.approval = ['DELETE'];
    expect(store.isDirty(tab)).toBe(true);
    expect(await store.saveTab(tab)).toBe(true);
    const put = calls.find((c) => c.method === 'PUT');
    expect(put?.path).toBe('/collections/col_1');
    expect((put?.body as Collection).agent_policy).toEqual({ require_approval: ['DELETE'], max_calls_per_minute: 30 });
  });
});

describe('llavero (S6)', () => {
  it('"Mover al llavero" marca store: keyring y guarda el entorno', async () => {
    store.environments = [DESA()];
    const moved = DESA();
    moved.variables[1].store = 'keyring';
    responses['GET /environments'] = [moved];
    store.openEnvironment('env_1');
    const tab = tabAt('env');
    await store.moveToKeyring(tab, 1);
    const put = calls.find((c) => c.method === 'PUT');
    expect(put?.path).toBe('/environments/env_1');
    expect((put?.body as { variables: Variable[] }).variables[1]).toMatchObject({ key: 'accessToken', secret: true, store: 'keyring' });
    expect(store.toasts.at(-1)?.text).toContain('accessToken');
  });

  it('si el llavero del sistema no está disponible, avisa y la fila vuelve a archivo', async () => {
    const { ApiError } = await import('../transport');
    responses['PUT /environments/env_1'] = new ApiError(503, 'keyring', 'keyring: no credential store on this system');
    store.environments = [DESA()];
    store.openEnvironment('env_1');
    const tab = tabAt('env');
    await store.moveToKeyring(tab);
    expect(tab.draft.variables[1].store).toBe('file');
    expect(store.toasts.at(-1)).toMatchObject({ kind: 'err' });
  });
});

describe('snippets (fase 4)', () => {
  it('"Copiar como curl" pide el código al servidor: guardada con su id, borrador con su colección', async () => {
    const { copyText } = await import('../native');
    responses['POST /codegen/curl'] = { lang: 'curl', code: "curl --location 'http://h/ping'" };
    store.settings = { active_environment: 'env_1' } as Settings;
    await store.copyCurl('col_1', request());
    const saved = calls.find((c) => c.path === '/codegen/curl');
    expect(saved?.body).toMatchObject({ request: { id: 'req_1', url: '{{base}}/ping' }, env_id: 'env_1' });
    expect(copyText).toHaveBeenLastCalledWith("curl --location 'http://h/ping'");
    expect(store.toasts.at(-1)).toMatchObject({ kind: 'ok' });

    calls.length = 0;
    await store.copyCurl('col_1', request({ id: '' }));
    const draft = calls.find((c) => c.path === '/codegen/curl')?.body as { request: Record<string, unknown> };
    expect(draft.request.collection_id).toBe('col_1');
    expect(draft.request).not.toHaveProperty('id');
  });

  it('el panel Código recuerda si está abierto y el lenguaje', () => {
    store.ui.code = true;
    store.ui.codeLang = 'python';
    store.persist();
    expect(JSON.parse(localStorage.getItem('runpost.ui') ?? '{}')).toMatchObject({ code: true, codeLang: 'python' });
  });
});

describe('importar (fase 4)', () => {
  it('un cURL pegado se abre en una pestaña nueva sin guardar', async () => {
    responses['POST /import'] = {
      reports: [],
      errors: [],
      requests: [{ ...emptyRequest('https://h.test/x'), id: 'req_server', method: 'POST', url: 'https://h.test/x' }],
    };
    expect(await store.importText('curl -X POST https://h.test/x')).toBe(true);
    expect(calls.find((c) => c.path === '/import')?.body).toEqual({ text: 'curl -X POST https://h.test/x' });
    const tab = tabAt('request');
    expect(tab.requestId).toBeNull();
    expect(tab.draft.method).toBe('POST');
    expect(tab.draft.url).toBe('https://h.test/x');
  });

  it('un texto que no se pudo importar avisa y deja el diálogo abierto', async () => {
    responses['POST /import'] = { reports: [], errors: [{ file: 'text', message: 'unknown format' }], requests: [] };
    expect(await store.importText('hola')).toBe(false);
    expect(store.toasts.at(-1)).toMatchObject({ kind: 'err' });
    expect(await store.importText('   ')).toBe(false);
  });
});

describe('OAuth 2.0 (fase 4)', () => {
  it('pide el estado, el token y espera el inicio de sesión sin ver nunca el token', async () => {
    store.settings = { active_environment: 'env_1' } as Settings;
    const token = { token_type: 'Bearer', expires_at: 1, scope: 'openid', refresh: true };
    responses['POST /oauth2'] = { flow: 'f1', authorization_url: 'https://id.example.test/authorize?state=s' };
    responses['GET /oauth2/flows/f1'] = { token };
    const result = await store.oauth('get', request(), 'req_1', 'col_1');
    expect(result).toEqual({ flow: 'f1', authorization_url: 'https://id.example.test/authorize?state=s' });
    const sent = calls.find((c) => c.path === '/oauth2');
    expect(sent?.body).toMatchObject({ action: 'get', env_id: 'env_1', request: { id: 'req_1' } });
    expect(await store.oauthWait('f1')).toEqual(token);
    expect(calls.at(-1)).toMatchObject({ method: 'GET', path: '/oauth2/flows/f1' });
  });
});

describe('cookies (fase 4)', () => {
  it('lista, guarda y borra con la API del tarro', async () => {
    const cookie = { domain: 'api.acme.test', host_only: true, path: '/', name: 'sid', value: 'x', expires: null, secure: false, http_only: true, raw: 'sid=x; Path=/' };
    responses['GET /cookies'] = { cookies: [cookie] };
    responses['PUT /cookies'] = { cookies: [cookie] };
    expect(await store.listCookies()).toEqual([cookie]);
    await store.saveCookie('api.acme.test', 'sid=x; Path=/');
    expect(calls.at(-1)).toMatchObject({ method: 'PUT', path: '/cookies', body: { domain: 'api.acme.test', cookie: 'sid=x; Path=/' } });
    responses['DELETE /cookies?domain=api.acme.test&path=%2F&name=sid'] = { cookies: [] };
    expect(await store.deleteCookies('api.acme.test', '/', 'sid')).toEqual([]);
    expect(calls.at(-1)).toMatchObject({ method: 'DELETE', path: '/cookies?domain=api.acme.test&path=%2F&name=sid' });
    responses['DELETE /cookies'] = { cookies: [] };
    await store.deleteCookies();
    expect(calls.at(-1)).toMatchObject({ method: 'DELETE', path: '/cookies' });
  });
});

describe('papelera (fase 4)', () => {
  const entry = { id: 'trash_1', kind: 'request' as const, name: 'ping', location: 'C', deleted_at: 1, requests: 1 };

  it('borrar deja un aviso con «Deshacer» que restaura desde la papelera', async () => {
    collections = [collection(request())];
    store.collections = collections;
    responses['DELETE /collections/col_1/items/req_1'] = { trash: entry };
    const deleting = store.deleteItem('col_1', { type: 'request', ...request() });
    (await dialogOf('confirm')).resolve(true);
    await deleting;
    const toast = store.toasts.at(-1);
    expect(toast).toMatchObject({ kind: 'ok' });
    expect(toast?.text).toContain('ping');
    toast?.action?.run();
    await vi.waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path === '/trash/trash_1/restore')).toBe(true));
  });

  it('lista, borra una entrada para siempre y vacía la papelera', async () => {
    responses['GET /trash'] = { items: [entry] };
    expect(await store.listTrash()).toEqual([entry]);
    await store.purgeTrash('trash_1');
    expect(calls.at(-1)).toMatchObject({ method: 'DELETE', path: '/trash/trash_1' });
    await store.purgeTrash();
    expect(calls.at(-1)).toMatchObject({ method: 'DELETE', path: '/trash' });
  });
});

describe('ejemplos y buscar y reemplazar (fase 4)', () => {
  it('guarda como ejemplo la respuesta de una ejecución del historial', async () => {
    const example = { id: 'exm_1', name: '200 OK', request: { method: 'GET', url: 'http://h/ping' }, status: 200, status_text: 'OK', body: '{}' };
    responses['POST /collections/col_1/items/req_1/examples'] = { collection_id: 'col_1', example };
    expect(await store.saveExample('col_1', 'req_1', 'hist_1')).toEqual(example);
    expect(calls.find((c) => c.path.endsWith('/examples'))?.body).toEqual({ history_id: 'hist_1', name: null });
    expect(store.toasts.at(-1)).toMatchObject({ kind: 'ok' });
  });

  it('busca con las opciones y reemplaza solo en los campos elegidos', async () => {
    const options = { case_sensitive: true, whole_word: false, regex: false };
    responses['POST /collections/col_1/find'] = { matches: [{ item_id: 'req_1', kind: 'request', path: 'C/F/ping', field: 'url', count: 1, preview: 'acme' }] };
    expect(await store.findInCollection('col_1', 'acme', options)).toHaveLength(1);
    expect(calls.at(-1)?.body).toEqual({ query: 'acme', ...options });
    responses['POST /collections/col_1/replace'] = { replaced: 2, collection: {} };
    const targets = [{ item_id: 'req_1', field: 'url' }];
    expect(await store.replaceInCollection('col_1', 'acme', 'demo', options, targets)).toBe(2);
    expect(calls.find((c) => c.path === '/collections/col_1/replace')?.body).toEqual({ query: 'acme', replacement: 'demo', ...options, targets });
  });
});
