import { afterEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { ApiError, api, backendInfo, connectWs, isTauri, resetBackend } from './transport';

const BROWSER = { port: 7878, token: 'abc', mode: 'browser' as const, workspace: 'D:\\ws' };

afterEach(() => {
  resetBackend();
  delete window.__RUNPOST_BACKEND__;
  delete window.__TAURI_INTERNALS__;
  invoke.mockReset();
  vi.unstubAllGlobals();
});

describe('backendInfo (sección 7)', () => {
  it('modo navegador: lee lo inyectado en index.html', async () => {
    window.__RUNPOST_BACKEND__ = BROWSER;
    expect(isTauri()).toBe(false);
    await expect(backendInfo()).resolves.toEqual(BROWSER);
    expect(invoke).not.toHaveBeenCalled();
  });

  it('modo Tauri: pide backend_info por IPC una sola vez', async () => {
    window.__TAURI_INTERNALS__ = {};
    invoke.mockResolvedValue({ ...BROWSER, mode: 'tauri' });
    expect(isTauri()).toBe(true);
    await backendInfo();
    await backendInfo();
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith('backend_info');
  });

  it('sin backend: error claro', async () => {
    await expect(backendInfo()).rejects.toThrow(/serve-browser/);
  });
});

describe('api', () => {
  it('manda el token, JSON y traduce los errores de la API', async () => {
    window.__RUNPOST_BACKEND__ = BROWSER;
    const fetchMock = vi.fn(async (url: string, init: RequestInit) => {
      if (url.endsWith('/missing')) {
        return new Response(JSON.stringify({ error: { code: 'collection_not_found', message: 'nope' } }), { status: 404 });
      }
      if (init.method === 'DELETE') return new Response(null, { status: 204 });
      return new Response(JSON.stringify({ ok: true, echo: init.body ?? null }), { status: 200 });
    });
    vi.stubGlobal('fetch', fetchMock);
    const result = await api.post<{ ok: boolean; echo: string }>('/collections', { name: 'x' });
    expect(result.echo).toBe('{"name":"x"}');
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe('http://127.0.0.1:7878/api/collections');
    expect((init.headers as Record<string, string>)['X-RunPost-Token']).toBe('abc');
    expect((init.headers as Record<string, string>)['Content-Type']).toBe('application/json');
    await expect(api.del('/collections/x')).resolves.toBeUndefined();
    const error = await api.get('/missing').catch((e: unknown) => e);
    expect(error).toBeInstanceOf(ApiError);
    expect((error as ApiError).code).toBe('collection_not_found');
    expect((error as ApiError).status).toBe(404);
  });

  it('FormData no lleva Content-Type JSON (multipart para importar)', async () => {
    window.__RUNPOST_BACKEND__ = BROWSER;
    const fetchMock = vi.fn(async () => new Response('{}', { status: 200 }));
    vi.stubGlobal('fetch', fetchMock);
    await api.post('/import', new FormData());
    const init = (fetchMock.mock.calls[0] as unknown as [string, RequestInit])[1];
    expect((init.headers as Record<string, string>)['Content-Type']).toBeUndefined();
  });
});

describe('connectWs', () => {
  it('pasa el token como subprotocolo y reenvía los eventos', async () => {
    window.__RUNPOST_BACKEND__ = BROWSER;
    const sockets: FakeSocket[] = [];
    class FakeSocket {
      onopen?: () => void;
      onmessage?: (m: { data: string }) => void;
      onclose?: () => void;
      constructor(
        public url: string,
        public protocols: string[],
      ) {
        sockets.push(this);
      }
      close() {
        this.onclose?.();
      }
    }
    vi.stubGlobal('WebSocket', FakeSocket);
    const events: unknown[] = [];
    const status: boolean[] = [];
    const stop = connectWs((e) => events.push(e), (s) => status.push(s));
    await vi.waitFor(() => expect(sockets).toHaveLength(1));
    expect(sockets[0].url).toBe('ws://127.0.0.1:7878/ws');
    expect(sockets[0].protocols).toEqual(['runpost', 'abc']);
    sockets[0].onopen?.();
    sockets[0].onmessage?.({ data: '{"type":"workspace_changed","kind":"collection","slug":"x"}' });
    sockets[0].onmessage?.({ data: 'not json' });
    expect(events).toEqual([{ type: 'workspace_changed', kind: 'collection', slug: 'x' }]);
    stop();
    expect(status).toEqual([true, false]);
  });
});
