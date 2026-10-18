// `agent_policy` de una colección (secciones 5.1 y 19.3) ↔ los tres controles de su pestaña Agente
// (docs/UX.md R4.4): métodos permitidos, qué requiere aprobación y límite por minuto.
import type { AgentPolicy } from './types';

export const HTTP_METHODS = ['GET', 'POST', 'PUT', 'PATCH', 'DELETE', 'HEAD', 'OPTIONS'] as const;

/** Límite que se propone al crear una política (UX: "por defecto 30, editable ahí mismo"). */
export const DEFAULT_LIMIT = 30;

export interface PolicyDraft {
  /** Métodos permitidos; los siete = sin restricción. */
  allow: string[];
  /** Métodos (`DELETE`) o ids de petición (`req_…`) que piden aprobación en la app. */
  approval: string[];
  /** `null` = sin límite. */
  limit: number | null;
}

const order = (method: string) => {
  const index = (HTTP_METHODS as readonly string[]).indexOf(method);
  return index < 0 ? HTTP_METHODS.length : index;
};

function sortMethods(methods: string[]): string[] {
  return [...new Set(methods.map((m) => m.toUpperCase()))].sort((a, b) => order(a) - order(b) || a.localeCompare(b));
}

export function isMethod(entry: string): boolean {
  return (HTTP_METHODS as readonly string[]).includes(entry.toUpperCase());
}

export function policyDraft(policy: AgentPolicy | null): PolicyDraft {
  return {
    allow: policy?.allow_methods ? sortMethods(policy.allow_methods) : [...HTTP_METHODS],
    approval: (policy?.require_approval ?? []).map((entry) => (isMethod(entry) ? entry.toUpperCase() : entry)),
    limit: policy ? (policy.max_calls_per_minute ?? null) : DEFAULT_LIMIT,
  };
}

/** Forma canónica (para el • de cambios sin guardar). */
export function policySnapshot(draft: PolicyDraft): string {
  const methods = sortMethods(draft.approval.filter(isMethod));
  const requests = draft.approval.filter((entry) => !isMethod(entry));
  return JSON.stringify({ allow: sortMethods(draft.allow), approval: [...methods, ...requests], limit: draft.limit });
}

/**
 * Política a guardar. Sin ninguna regla vuelve `null` (todo permitido); el límite propuesto (30) solo
 * cuenta si la colección ya tenía política o si hay otra regla.
 */
export function toPolicy(draft: PolicyDraft, previous: AgentPolicy | null): AgentPolicy | null {
  const allowAll = HTTP_METHODS.every((method) => draft.allow.map((m) => m.toUpperCase()).includes(method));
  const methods = sortMethods(draft.approval.filter(isMethod));
  const approval = [...methods, ...draft.approval.filter((entry) => !isMethod(entry))];
  const limit = draft.limit !== null && Number.isFinite(draft.limit) && draft.limit > 0 ? Math.round(draft.limit) : null;
  if (allowAll && approval.length === 0 && (limit === null || (previous === null && limit === DEFAULT_LIMIT))) return null;
  const policy: AgentPolicy = {};
  if (!allowAll) policy.allow_methods = sortMethods(draft.allow);
  if (approval.length) policy.require_approval = approval;
  if (limit !== null) policy.max_calls_per_minute = limit;
  return policy;
}
