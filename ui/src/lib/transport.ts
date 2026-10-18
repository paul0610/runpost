// Única pieza que distingue Tauri de navegador (sección 7). El resto de la UI usa `api` y `ws`.
import { invoke } from '@tauri-apps/api/core';
import type { WsEvent } from './types';

export interface BackendInfo {
  port: number;
  token: string;
  mode: 'tauri' | 'browser';
  workspace: string;
}

declare global {
  interface Window {
    __RUNPOST_BACKEND__?: BackendInfo;
    __TAURI_INTERNALS__?: unknown;
  }
}

export const isTauri = (): boolean => '__TAURI_INTERNALS__' in window;

let backend: Promise<BackendInfo> | undefined;

export function backendInfo(): Promise<BackendInfo> {
  backend ??= isTauri()
    ? invoke<BackendInfo>('backend_info')
    : window.__RUNPOST_BACKEND__
      ? Promise.resolve(window.__RUNPOST_BACKEND__)
      : Promise.reject(new Error('Backend no disponible. Usa scripts/dev o scripts/serve-browser.'));
  return backend;
}

/** Solo para tests: olvida el backend en caché. */
export function resetBackend(): void {
  backend = undefined;
}

/** Error de la API: `{"error":{"code","message",...}}` (sección 7). */
export class ApiError extends Error {
  constructor(
    public status: number,
    public code: string,
    message: string,
    public details: Record<string, unknown> = {},
  ) {
    super(message);
  }
}

async function request<T>(method: string, path: string, body?: unknown, raw = false): Promise<T> {
  const info = await backendInfo();
  const headers: Record<string, string> = { 'X-RunPost-Token': info.token };
  let payload: BodyInit | undefined;
  if (body instanceof FormData) {
    payload = body;
  } else if (body !== undefined) {
    headers['Content-Type'] = 'application/json';
    payload = JSON.stringify(body);
  }
  const response = await fetch(`http://127.0.0.1:${info.port}/api${path}`, { method, headers, body: payload });
  if (response.status === 204) return undefined as T;
  const text = await response.text();
  if (raw && response.ok) return text as T;
  const data = text ? JSON.parse(text) : undefined;
  if (!response.ok) {
    const error = data?.error ?? {};
    throw new ApiError(response.status, error.code ?? 'http_error', error.message ?? `HTTP ${response.status}`, error);
  }
  return data as T;
}

export const api = {
  get: <T>(path: string) => request<T>('GET', path),
  /** Texto tal cual (p. ej. el informe HTML del runner); los errores siguen siendo JSON. */
  getText: (path: string) => request<string>('GET', path, undefined, true),
  post: <T>(path: string, body?: unknown) => request<T>('POST', path, body ?? {}),
  put: <T>(path: string, body: unknown) => request<T>('PUT', path, body),
  patch: <T>(path: string, body: unknown) => request<T>('PATCH', path, body),
  del: <T = void>(path: string) => request<T>('DELETE', path),
};

export type WsListener = (event: WsEvent) => void;

/**
 * WebSocket `/ws` con reconexión. El navegador no admite cabeceras en el handshake:
 * el token va como subprotocolo junto a `runpost` (el servidor responde `runpost`).
 */
export function connectWs(listener: WsListener, onStatus?: (connected: boolean) => void): () => void {
  let socket: WebSocket | undefined;
  let stopped = false;
  let retry = 500;
  let timer: ReturnType<typeof setTimeout> | undefined;

  const open = async () => {
    const info = await backendInfo();
    if (stopped) return;
    socket = new WebSocket(`ws://127.0.0.1:${info.port}/ws`, ['runpost', info.token]);
    socket.onopen = () => {
      retry = 500;
      onStatus?.(true);
    };
    socket.onmessage = (message) => {
      try {
        listener(JSON.parse(String(message.data)) as WsEvent);
      } catch {
        /* mensaje no JSON: ignorar */
      }
    };
    socket.onclose = () => {
      onStatus?.(false);
      if (stopped) return;
      timer = setTimeout(() => void open(), retry);
      retry = Math.min(retry * 2, 10_000);
    };
  };
  void open();
  return () => {
    stopped = true;
    if (timer) clearTimeout(timer);
    socket?.close();
  };
}
