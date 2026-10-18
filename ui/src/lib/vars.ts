// Resaltado de {{variables}} (sección 9.1): verde si resuelve, rojo si no existe; hover con valor o ***.
// Mismo orden que el servidor (5.7) para lo que la UI conoce: entorno activo > colección > globales > dinámicas.
import type { Variable } from './types';

export const DYNAMIC = ['$guid', '$timestamp', '$isoTimestamp', '$randomInt'] as const;
export const MASK = '***';

export interface VarInfo {
  value: string;
  secret: boolean;
  /** 'environment' | 'collection' | 'globals' | 'dynamic' */
  scope: string;
}

export type VarLookup = (name: string) => VarInfo | null;

export interface Segment {
  text: string;
  /** Nombre de la variable si el segmento es `{{nombre}}`. */
  name?: string;
  from: number;
  to: number;
}

const PATTERN = /\{\{\s*([^{}]+?)\s*\}\}/g;

/** Trozos de texto y de `{{variable}}` con sus posiciones. */
export function segments(text: string): Segment[] {
  const out: Segment[] = [];
  let last = 0;
  for (const match of text.matchAll(PATTERN)) {
    const at = match.index ?? 0;
    if (at > last) out.push({ text: text.slice(last, at), from: last, to: at });
    out.push({ text: match[0], name: match[1], from: at, to: at + match[0].length });
    last = at + match[0].length;
  }
  if (last < text.length) out.push({ text: text.slice(last), from: last, to: text.length });
  return out;
}

export function variableNames(text: string): string[] {
  return [...new Set(segments(text).flatMap((s) => (s.name ? [s.name] : [])))];
}

/** Construye la búsqueda a partir de las capas que conoce la UI (las deshabilitadas no cuentan). */
export function lookupFrom(layers: { scope: string; vars: Variable[] | undefined }[]): VarLookup {
  return (name) => {
    for (const layer of layers) {
      const found = layer.vars?.find((v) => v.key === name && v.enabled !== false && v.value !== undefined);
      if (found) return { value: found.value ?? '', secret: found.secret === true, scope: layer.scope };
    }
    if ((DYNAMIC as readonly string[]).includes(name)) return { value: '', secret: false, scope: 'dynamic' };
    return null;
  };
}
