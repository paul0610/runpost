// Panel Agente (P5 de docs/UX.md y sección 19): medidor de tokens, estado de conexión y la
// exportación del medidor en Markdown para docs/BENCH.md. Funciones puras, sin el store.
import type { AgentCall, AgentClient, AgentMeter, McpStatus } from './types';

/** Un cliente cuenta como conectado si habló en los últimos 10 minutos. */
export const ACTIVE_MS = 10 * 60 * 1000;

/** Tokens estimados (sección 19.6): caracteres / 4. */
export const tokens = (bytes: number): number => Math.round(bytes / 4);

/** Tokens que el modelo no tuvo que leer gracias al presupuesto (nunca negativo). */
export function savedTokens(meter: Pick<AgentMeter, 'raw' | 'sent'>): number {
  return Math.max(0, tokens(meter.raw - meter.sent));
}

/** `38`, `1,1k`, `41k`, `1,2M` (columna "t" estrecha del mockup). */
export function compact(value: number, lang = 'es'): string {
  const format = (n: number, digits: number) =>
    n.toLocaleString(lang === 'en' ? 'en-US' : 'es-ES', { maximumFractionDigits: digits, minimumFractionDigits: 0 });
  if (value < 1000) return String(Math.round(value));
  if (value < 1_000_000) return `${format(value / 1000, value < 10_000 ? 1 : 0)}k`;
  return `${format(value / 1_000_000, 1)}M`;
}

export type AgentStatus =
  | { kind: 'connected'; client: string }
  | { kind: 'idle'; client: string; since: number }
  | { kind: 'none' }
  | { kind: 'off' }
  | { kind: 'error'; error: string };

/** Línea de estado del panel: el cliente más reciente, o por qué no hay MCP por HTTP. */
export function agentStatus(clients: AgentClient[], mcp: McpStatus, now = Date.now()): AgentStatus {
  if (mcp.error) return { kind: 'error', error: mcp.error };
  const latest = [...clients].sort((a, b) => b.last_seen - a.last_seen)[0];
  if (latest) {
    return now - latest.last_seen <= ACTIVE_MS
      ? { kind: 'connected', client: latest.client }
      : { kind: 'idle', client: latest.client, since: latest.last_seen };
  }
  return mcp.url ? { kind: 'none' } : { kind: 'off' };
}

/** Color del estado: error de la tool o HTTP ≥ 400 en rojo; tests con fallos (`3/4`) en ámbar. */
export function callTone(call: Pick<AgentCall, 'ok' | 'status'>): 'ok' | 'warn' | 'err' {
  if (!call.ok) return 'err';
  const code = Number(call.status);
  if (Number.isInteger(code) && code >= 400) return 'err';
  const tests = /^(\d+)\/(\d+)$/.exec(call.status);
  if (tests) return Number(tests[1]) < Number(tests[2]) ? 'warn' : 'ok';
  return 'ok';
}

/** En orden cronológico, como el mockup: lo más reciente abajo. */
export function filterCalls(calls: AgentCall[], tool: string, session: string): AgentCall[] {
  return calls.filter((call) => (!tool || call.tool === tool) && (!session || call.session === session));
}

/** `{{a}}/x/{{b}}` → trozos para partir la URL solo detrás de una `/`. */
export function urlParts(url: string): string[] {
  return url.split(/(?<=\/)/);
}

const cell = (text: string) => text.replace(/\|/g, '\\|').replace(/\n/g, ' ');

/** Tabla Markdown del medidor (por llamada y total) para pegar en docs/BENCH.md. */
export function meterMarkdown(calls: AgentCall[], meter: AgentMeter, when: Date): string {
  const time = (ts: number) => new Date(ts).toISOString().slice(11, 19);
  const lines = [
    `### Medidor de tokens del panel Agente (${when.toISOString().slice(0, 16).replace('T', ' ')} UTC)`,
    '',
    '| Hora (UTC) | Cliente | Tool | Petición | Estado | ms | Crudo (B) | Enviado (B) | Tokens ahorrados (≈) |',
    '|---|---|---|---|---|---:|---:|---:|---:|',
    ...calls.map(
      (call) =>
        `| ${time(call.ts)} | ${cell(call.client)} | ${call.tool} | ${cell(call.target || '—')} | ${cell(call.status)} | ${call.ms} | ${call.raw} | ${call.sent} | ${savedTokens(call)} |`,
    ),
    `| **Total** | | **${meter.calls} llamadas** | | | | **${meter.raw}** | **${meter.sent}** | **${savedTokens(meter)}** |`,
    '',
    'Tokens estimados como bytes / 4. "Crudo" es la respuesta completa; "enviado", lo que recibió el modelo.',
    '',
  ];
  return lines.join('\n');
}
