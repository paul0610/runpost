// "Bulk edit" de tablas clave/valor (sección 18): "Clave: Valor" por línea; "//" delante = desactivada.
import type { KeyValue } from './types';

export function toBulk(rows: KeyValue[]): string {
  return rows.map((r) => `${r.enabled === false ? '//' : ''}${r.key}: ${r.value}`).join('\n');
}

export function fromBulk(text: string): KeyValue[] {
  return text
    .split(/\r?\n/)
    .filter((line) => line.trim() !== '')
    .map((line) => {
      const disabled = line.trimStart().startsWith('//');
      const body = disabled ? line.trimStart().slice(2) : line;
      const at = body.indexOf(':');
      const key = (at < 0 ? body : body.slice(0, at)).trim();
      const value = at < 0 ? '' : body.slice(at + 1).trim();
      return { key, value, enabled: !disabled };
    });
}

/** Cabeceras habituales para el autocompletado (sección 18). */
export const COMMON_HEADERS = [
  'Accept',
  'Accept-Encoding',
  'Accept-Language',
  'Authorization',
  'Cache-Control',
  'Connection',
  'Content-Type',
  'Cookie',
  'If-Match',
  'If-None-Match',
  'Origin',
  'Referer',
  'User-Agent',
  'X-Api-Key',
  'X-Request-Id',
  'X-Requested-With',
];

export const MEDIA_TYPES = [
  'application/json',
  'application/xml',
  'application/x-www-form-urlencoded',
  'multipart/form-data',
  'text/plain',
  'text/html',
  '*/*',
];
