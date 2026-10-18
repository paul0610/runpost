import { describe, expect, it } from 'vitest';
import { ACTIVE_MS, agentStatus, callTone, compact, filterCalls, meterMarkdown, savedTokens, urlParts } from './agent';
import { DEFAULT_LIMIT, HTTP_METHODS, policyDraft, policySnapshot, toPolicy } from './policy';
import type { AgentCall, AgentClient } from './types';

const call = (overrides: Partial<AgentCall> = {}): AgentCall => ({
  id: 1,
  ts: Date.UTC(2026, 9, 18, 11, 41, 7),
  tool: 'run_request',
  target: '00.2 Gateway - ping',
  status: '200',
  ok: true,
  ms: 98,
  raw: 1100,
  sent: 112,
  client: 'Claude Code',
  session: 'abcdef01',
  ...overrides,
});

describe('medidor del panel Agente', () => {
  it('tokens ahorrados = (crudo - enviado) / 4, nunca negativos', () => {
    expect(savedTokens({ raw: 84_000, sent: 3_100 })).toBe(20_225);
    expect(savedTokens({ raw: 20, sent: 150 })).toBe(0);
  });

  it('números cortos como el mockup', () => {
    expect(compact(38)).toBe('38');
    expect(compact(1100)).toBe('1,1k');
    expect(compact(1100, 'en')).toBe('1.1k');
    expect(compact(41_000)).toBe('41k');
    expect(compact(1_250_000)).toBe('1,3M');
  });

  it('color del estado: error, HTTP ≥ 400 y tests con fallos', () => {
    expect(callTone({ ok: true, status: '200' })).toBe('ok');
    expect(callTone({ ok: true, status: '500' })).toBe('err');
    expect(callTone({ ok: false, status: 'policy' })).toBe('err');
    expect(callTone({ ok: true, status: '3/4' })).toBe('warn');
    expect(callTone({ ok: true, status: '4/4' })).toBe('ok');
    expect(callTone({ ok: true, status: 'ok' })).toBe('ok');
  });

  it('filtra por tool y sesión, en orden cronológico como el mockup', () => {
    const calls = [call({ id: 1 }), call({ id: 2, tool: 'env_set' }), call({ id: 3, session: 'zz' })];
    expect(filterCalls(calls, '', '').map((c) => c.id)).toEqual([1, 2, 3]);
    expect(filterCalls(calls, 'run_request', '').map((c) => c.id)).toEqual([1, 3]);
    expect(filterCalls(calls, 'run_request', 'abcdef01').map((c) => c.id)).toEqual([1]);
  });

  it('la URL del aviso se parte solo detrás de una barra', () => {
    expect(urlParts('{{apiUrl}}/v1/{{currency}}/accounts/{{accountId}}')).toEqual([
      '{{apiUrl}}/',
      'v1/',
      '{{currency}}/',
      'accounts/',
      '{{accountId}}',
    ]);
  });

  it('estado: conectado, inactivo, sin agente, apagado o con error', () => {
    const now = Date.now();
    const client = (last_seen: number): AgentClient => ({ client: 'Claude Code', version: '2.1', session: 's', first_seen: 0, last_seen, calls: 1 });
    const mcp = { port: 7879, url: 'http://127.0.0.1:7879/mcp', error: null };
    expect(agentStatus([client(now - 1000)], mcp, now)).toEqual({ kind: 'connected', client: 'Claude Code' });
    expect(agentStatus([client(now - ACTIVE_MS - 1)], mcp, now).kind).toBe('idle');
    expect(agentStatus([], mcp, now)).toEqual({ kind: 'none' });
    expect(agentStatus([], { port: null, url: null, error: null }, now)).toEqual({ kind: 'off' });
    expect(agentStatus([], { port: null, url: null, error: '127.0.0.1:7879: en uso' }, now)).toEqual({
      kind: 'error',
      error: '127.0.0.1:7879: en uso',
    });
  });

  it('exporta una tabla Markdown por llamada con el total para docs/BENCH.md', () => {
    const calls = [call(), call({ id: 2, target: 'a|b', status: 'policy', ok: false, raw: 80, sent: 80 })];
    const text = meterMarkdown(calls, { calls: 2, raw: 1180, sent: 192 }, new Date(Date.UTC(2026, 9, 18, 11, 45)));
    expect(text).toContain('### Medidor de tokens del panel Agente (2026-10-18 11:45 UTC)');
    expect(text).toContain('| 11:41:07 | Claude Code | run_request | 00.2 Gateway - ping | 200 | 98 | 1100 | 112 | 247 |');
    expect(text).toContain('| a\\|b | policy |');
    expect(text).toContain('| **Total** | | **2 llamadas** | | | | **1180** | **192** | **247** |');
  });
});

describe('política del agente (pestaña Agente de la colección)', () => {
  it('sin política: todos los métodos, nada que aprobar y el límite propuesto', () => {
    expect(policyDraft(null)).toEqual({ allow: [...HTTP_METHODS], approval: [], limit: DEFAULT_LIMIT });
    // Guardar sin tocar nada deja la colección sin política.
    expect(toPolicy(policyDraft(null), null)).toBeNull();
  });

  it('los tres controles se convierten en agent_policy', () => {
    const draft = policyDraft(null);
    draft.allow = ['GET', 'POST', 'DELETE'];
    draft.approval = ['delete', 'req_7'];
    expect(toPolicy(draft, null)).toEqual({
      allow_methods: ['GET', 'POST', 'DELETE'],
      require_approval: ['DELETE', 'req_7'],
      max_calls_per_minute: 30,
    });
    draft.limit = null;
    expect(toPolicy(draft, null)).toEqual({ allow_methods: ['GET', 'POST', 'DELETE'], require_approval: ['DELETE', 'req_7'] });
  });

  it('una política existente se edita y el orden no ensucia la pestaña', () => {
    const policy = { allow_methods: ['post', 'GET'], require_approval: ['DELETE'], max_calls_per_minute: 10 };
    const draft = policyDraft(policy);
    expect(draft).toEqual({ allow: ['GET', 'POST'], approval: ['DELETE'], limit: 10 });
    expect(policySnapshot({ ...draft, allow: ['POST', 'GET'] })).toBe(policySnapshot(draft));
    // Solo el límite: se conserva como política.
    expect(toPolicy({ allow: [...HTTP_METHODS], approval: [], limit: 30 }, { max_calls_per_minute: 30 })).toEqual({ max_calls_per_minute: 30 });
    expect(toPolicy({ allow: [...HTTP_METHODS], approval: [], limit: null }, policy)).toBeNull();
  });
});
