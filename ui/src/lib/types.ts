// Espejo del formato nativo (ARCHITECTURE.md sección 5) y de las respuestas de la API (sección 7).

export interface KeyValue {
  key: string;
  value: string;
  /** Ausente = true (el servidor omite los `true`). */
  enabled?: boolean;
}

export interface Variable {
  key: string;
  value?: string;
  secret?: boolean;
  store?: 'file' | 'keyring';
  enabled?: boolean;
}

export interface Scripts {
  pre: string;
  test: string;
}

export type Auth =
  | { type: 'none' }
  | { type: 'inherit' }
  | { type: 'bearer'; token: string }
  | { type: 'basic'; username: string; password: string }
  | { type: 'apikey'; key: string; value: string; in: 'header' | 'query' };

export type RawLang = 'json' | 'text' | 'xml' | 'html' | 'javascript';

export interface MultipartField {
  key: string;
  value: string;
  kind: 'text' | 'file';
  path?: string;
  enabled?: boolean;
}

export type Body =
  | { mode: 'none' }
  | { mode: 'raw'; lang: RawLang; text: string }
  | { mode: 'urlencoded'; fields: KeyValue[] }
  | { mode: 'multipart'; fields: MultipartField[] }
  | { mode: 'binary'; path: string }
  | { mode: 'graphql'; query: string; variables: string };

export interface RequestSettings {
  follow_redirects: boolean | null;
  verify_tls: boolean | null;
  timeout_ms: number | null;
}

export interface Request {
  id: string;
  name: string;
  method: string;
  url: string;
  params?: KeyValue[];
  headers?: KeyValue[];
  auth: Auth;
  body: Body;
  scripts: Scripts;
  settings: RequestSettings;
  description: string;
}

export interface Folder {
  id: string;
  name: string;
  description: string;
  auth: Auth | null;
  scripts: Scripts;
  items: Item[];
}

export type Item = ({ type: 'folder' } & Folder) | ({ type: 'request' } & Request);

export interface AgentPolicy {
  allow_methods?: string[];
  require_approval?: string[];
  max_calls_per_minute?: number;
}

export interface Collection {
  schema: number;
  id: string;
  name: string;
  description: string;
  auth: Auth | null;
  agent_policy: AgentPolicy | null;
  variables: Variable[];
  scripts: Scripts;
  items: Item[];
}

export interface CollectionSummary {
  id: string;
  name: string;
  slug: string;
  folders: number;
  requests: number;
}

export interface Environment {
  schema: number;
  id: string;
  name: string;
  variables: Variable[];
}

export interface Globals {
  schema: number;
  /** Aparece al guardar el primer secreto en el llavero (es su cuenta). */
  id?: string;
  variables: Variable[];
}

export type ThemeMode = 'light' | 'dark' | 'system';

export interface Settings {
  schema: number;
  theme: ThemeMode;
  active_environment: string | null;
  http: {
    timeout_ms: number;
    verify_tls: boolean;
    follow_redirects: boolean;
    max_redirects: number;
    proxy: string | null;
    user_agent: string;
  };
  mcp: { body_max_chars: number; headers_default: boolean; mask_secrets: boolean; cache_responses: number };
  ui: { font_size: number; wrap_response: boolean; lang: string };
}

export type Headers = [string, string][];

export interface ResponseCookie {
  name: string;
  value: string;
  domain: string | null;
  path: string | null;
  secure: boolean;
  http_only: boolean;
}

export interface HttpResponse {
  status: number;
  status_text: string;
  headers: Headers;
  body: string;
  body_encoding: 'utf8' | 'base64';
  ms: number;
  size: number;
  cookies: ResponseCookie[];
}

export interface ResolvedRequest {
  method: string;
  url: string;
  headers: Headers;
  body: string | null;
  timeout_ms: number;
  follow_redirects: boolean;
  verify_tls: boolean;
  unresolved: string[];
}

export interface TestResult {
  name: string;
  passed: boolean;
  msg: string | null;
  /** `pm.test.skip` o `pm.test` sin función: no cuenta como pasado ni como fallido. */
  skipped?: boolean;
}

export interface RunResult {
  response: HttpResponse;
  resolved: ResolvedRequest;
  tests: TestResult[];
  vars_changed: { scope: string; key: string }[];
  history_id: string;
  console: string[];
  /** Un script `pre` llamó a `pm.execution.skipRequest()`: no se envió nada. */
  skipped?: boolean;
}

/** Una petición ejecutada (u omitida) por el runner. */
export interface RunnerItem {
  iteration: number;
  id: string;
  name: string;
  path: string;
  method: string;
  url: string;
  status: number | null;
  ms: number;
  size: number;
  tests: TestResult[];
  error: string | null;
  skipped: boolean;
  console: string[];
}

export interface RunnerTotals {
  requests: number;
  skipped: number;
  errors: number;
  tests: number;
  passed: number;
  failed: number;
}

export interface RunnerReport {
  collection: { id: string; name: string };
  folder: { id: string; name: string } | null;
  environment: string | null;
  iterations: number;
  data: { name: string | null; rows: number } | null;
  started_at: string;
  ms: number;
  cancelled: boolean;
  totals: RunnerTotals;
  items: RunnerItem[];
}

export interface HistorySummary {
  id: string;
  ts: number;
  request_id: string | null;
  source: 'ui' | 'mcp' | 'cli';
  method: string;
  url: string;
  status: number | null;
  duration_ms: number;
  size_bytes: number;
  error: string | null;
}

/** `GET /history/{id}`: lo enviado y lo recibido, con los secretos ya como `***`. */
export interface HistoryEntry extends HistorySummary {
  req_headers: Headers;
  req_body: { text: string; encoding: string } | null;
  res_headers: Headers;
  res_body: { text: string; encoding: string } | null;
}

export interface ImportReport {
  kind: 'collection' | 'environment';
  id: string;
  name: string;
  folders: number;
  requests: number;
  variables: number;
  file: string;
  replaced: boolean;
  warnings: string[];
}

export interface ImportResult {
  reports: ImportReport[];
  errors: { file: string; message: string }[];
}

export interface AgentConfig {
  client: string;
  text: string;
  target: string;
  /** `http`: sus llamadas se ven en el panel Agente; `stdio`: no (Claude Desktop). */
  transport: 'http' | 'stdio';
  exe: string;
  exists: boolean;
}

/** Una llamada MCP por HTTP (fila del panel Agente, sección 19.2). */
export interface AgentCall {
  id: number;
  ts: number;
  tool: string;
  target: string;
  request_id?: string;
  /** `200`, `3/4`, `ok` o el código de error (`policy`, `connect`…). */
  status: string;
  ok: boolean;
  ms: number;
  /** Bytes de la respuesta completa frente a los entregados al modelo (19.6). */
  raw: number;
  sent: number;
  client: string;
  session: string;
}

export interface AgentMeter {
  calls: number;
  raw: number;
  sent: number;
}

export interface AgentClient {
  client: string;
  version: string;
  session: string;
  first_seen: number;
  last_seen: number;
  calls: number;
}

/** Petición protegida esperando al usuario (aviso de la esquina superior derecha). */
export interface Approval {
  id: string;
  ts: number;
  expires: number;
  client: string;
  tool: string;
  method: string;
  /** URL sin resolver (`{{apiUrl}}/…`). */
  url: string;
  request: string;
  request_id?: string;
  collection: string;
  environment?: string;
  rule: string;
  count: number;
}

export interface McpStatus {
  port: number | null;
  url: string | null;
  error: string | null;
}

export interface AgentState {
  calls: AgentCall[];
  meter: AgentMeter;
  clients: AgentClient[];
  pending: Approval[];
  mcp: McpStatus;
}

export type ApprovalDecision = 'approved' | 'denied' | 'expired' | 'cancelled' | 'unavailable';

export type WsEvent =
  | { type: 'hello' }
  | { type: 'workspace_changed'; kind: 'collection' | 'environment'; slug: string }
  | { type: 'workspace_changed'; kind: 'settings' | 'globals' | 'all' }
  | { type: 'runner_progress'; run_id: string; event: 'started'; total: number; iterations: number }
  | { type: 'runner_progress'; run_id: string; event: 'item'; done: number; total: number; item: RunnerItem }
  | { type: 'runner_done'; run_id: string; totals?: RunnerTotals; cancelled?: boolean; ms?: number; error?: string }
  | { type: 'agent_call'; call: AgentCall }
  | { type: 'agent_client'; client: AgentClient }
  | { type: 'agent_approval'; approval: Approval }
  | { type: 'agent_approval_done'; id: string; decision: ApprovalDecision };
