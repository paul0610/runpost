// Store único (sección 9.2): workspace, pestañas, entorno activo, ajustes y tema.
// Los datos vienen siempre de la API; el WebSocket avisa de cambios hechos fuera (p. ej. por MCP).
import { ApiError, api, backendInfo, connectWs, isTauri, type BackendInfo } from '../transport';
import type {
  AgentCall,
  AgentClient,
  AgentConfig,
  AgentMeter,
  AgentState,
  Approval,
  Collection,
  CollectionSummary,
  Environment,
  Globals,
  HistoryEntry,
  HistorySummary,
  ImportResult,
  Item,
  KeyValue,
  McpStatus,
  OAuthResult,
  OAuthToken,
  Request,
  RunResult,
  RunnerItem,
  RunnerReport,
  Settings,
  SnippetLang,
  ThemeMode,
  Variable,
  WsEvent,
} from '../types';
import { LANG_FLAG, detectLang, isLang, locale, t } from '../i18n.svelte';
import { applyTheme, nextTheme, watchSystemTheme } from '../theme';
import { copyText, downloadText, pickFiles, pickSavePath, type PickedFiles } from '../native';
import { applyUrlText, composeUrl } from '../url';
import { lookupFrom, type VarLookup } from '../vars';
import type { VarDescriber } from '../cm';
import { draftPayload, emptyRequest, findRequest, locate, normalizeRequest, snapshot, type Draft } from './requests';
import { policyDraft, policySnapshot, toPolicy, type PolicyDraft } from '../policy';
import { meterMarkdown } from '../agent';

export type Pane = 'collections' | 'environments' | 'history' | 'agent';
export type SubTab = 'params' | 'auth' | 'headers' | 'body' | 'scripts' | 'settings' | 'docs';
export type RespTab = 'body' | 'headers' | 'cookies' | 'tests';
export type BodyView = 'pretty' | 'raw' | 'preview';
export type DrawerTab = 'console' | 'agent' | 'runner';

/** `envId` especial para la pestaña de variables globales. */
export const GLOBALS = 'globals';

export interface SendError {
  code: string;
  message: string;
  url?: string;
  historyId?: string;
}

export interface RequestTab {
  kind: 'request';
  id: string;
  collectionId: string | null;
  requestId: string | null;
  draft: Draft;
  /** Huella de la última versión guardada (para el punto •). */
  saved: string;
  /** Texto de la barra de URL (base + query activa). */
  urlText: string;
  subtab: SubTab;
  respTab: RespTab;
  bodyView: BodyView;
  result: RunResult | null;
  error: SendError | null;
  sending: boolean;
}

export interface EnvDraft {
  name: string;
  variables: Variable[];
}

/** Gestor de un entorno o de las globales (P3). */
export interface EnvTab {
  kind: 'env';
  id: string;
  envId: string;
  draft: EnvDraft;
  saved: string;
  /** Variables secretas que el usuario ha destapado con el ojo (por índice de fila). */
  revealed: number[];
}

/** Documentación markdown de una colección o carpeta (S4). */
export interface DocsTab {
  kind: 'docs';
  id: string;
  collectionId: string;
  itemId: string | null;
  draft: string;
  saved: string;
  editing: boolean;
}

/** Pestaña Agente de una colección (R4.4): métodos permitidos, aprobación y límite por minuto. */
export interface PolicyTab {
  kind: 'policy';
  id: string;
  collectionId: string;
  draft: PolicyDraft;
  saved: string;
}

export type Tab = RequestTab | EnvTab | DocsTab | PolicyTab;

/** Una línea de la Consola (formato del mockup): petición resuelta y respuesta o error. */
export interface ConsoleEntry {
  id: number;
  ts: number;
  method: string;
  url: string;
  headers: [string, string][];
  status: number | null;
  ms: number | null;
  size: number | null;
  contentType: string;
  error: string | null;
  unresolved: string[];
  console: string[];
  /** `pm.execution.skipRequest()`: no se envió nada. */
  skipped?: boolean;
  /** Tests fallidos de los scripts (como la consola del mockup). */
  failures?: { name: string; msg: string | null }[];
}

export type RunnerStatus = 'idle' | 'running' | 'done' | 'error';

/** Archivo de datos del runner: el texto (navegador) o la ruta (diálogo nativo). */
export interface RunnerData {
  name: string;
  text?: string;
  path?: string;
}

type RunnerEvent = Extract<WsEvent, { type: 'runner_progress' | 'runner_done' }>;
type AgentEvent = Extract<WsEvent, { type: 'agent_call' | 'agent_client' | 'agent_approval' | 'agent_approval_done' }>;

export interface Toast {
  id: number;
  kind: 'info' | 'ok' | 'err';
  text: string;
}

export interface MenuItem {
  label: string;
  action?: () => void | Promise<void>;
  disabled?: boolean;
  danger?: boolean;
  hint?: string;
  separator?: boolean;
}

export type Dialog =
  | { kind: 'prompt'; title: string; value: string; confirm: string; resolve: (value: string | null) => void }
  | { kind: 'confirm'; title: string; body: string; confirm: string; danger: boolean; resolve: (ok: boolean) => void }
  | { kind: 'unsaved'; name: string; resolve: (choice: 'save' | 'discard' | 'cancel') => void }
  | { kind: 'text'; title: string; body: string; text: string; resolve: () => void }
  | { kind: 'export'; title: string; strip: boolean; resolve: (strip: boolean | null) => void }
  | {
      kind: 'saveAs';
      name: string;
      collectionId: string | null;
      resolve: (choice: { collectionId: string; parentId: string | null; name: string } | null) => void;
    };

interface UiPersisted {
  pane?: Pane;
  sideOpen?: boolean;
  splitPx?: number;
  code?: boolean;
  codeLang?: SnippetLang;
}

/** Lo que reciben /send y /codegen: la petición guardada (con sus cambios) o un borrador. */
function requestPayload(request: Request, requestId: string | null, collectionId: string | null) {
  return requestId ? { ...request, id: requestId } : draftPayload(request, collectionId);
}

const TABS_KEY = 'runpost.tabs';
const COLLAPSED_KEY = 'runpost.collapsed';
const UI_KEY = 'runpost.ui';
const MAX_CONSOLE = 200;
/** Llamadas del agente que guarda el panel (como el servidor). */
const MAX_AGENT_CALLS = 500;

function load<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    return raw ? (JSON.parse(raw) as T) : fallback;
  } catch {
    return fallback;
  }
}

function save(key: string, value: unknown): void {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* almacenamiento lleno o bloqueado: la persistencia de UI es opcional */
  }
}

const newId = (): string =>
  typeof crypto !== 'undefined' && 'randomUUID' in crypto ? crypto.randomUUID() : `t${Date.now()}${Math.random()}`;

export function makeTab(request: Request, collectionId: string | null, requestId: string | null): RequestTab {
  const draft = normalizeRequest(request);
  return {
    kind: 'request',
    id: newId(),
    collectionId,
    requestId,
    draft,
    saved: snapshot(draft),
    urlText: composeUrl(draft.url, draft.params),
    subtab: 'params',
    respTab: 'body',
    bodyView: 'pretty',
    result: null,
    error: null,
    sending: false,
  };
}

/** Forma canónica de las variables de un entorno (para el •). */
export function normalizeEnv(draft: EnvDraft): EnvDraft {
  return {
    name: draft.name,
    variables: draft.variables.map((v) => ({
      key: v.key,
      value: v.value ?? '',
      secret: v.secret === true,
      ...(v.store ? { store: v.store } : {}),
      enabled: v.enabled !== false,
    })),
  };
}

export const envSnapshot = (draft: EnvDraft): string => JSON.stringify(normalizeEnv(JSON.parse(JSON.stringify(draft)) as EnvDraft));

export function makeEnvTab(envId: string, name: string, variables: Variable[]): EnvTab {
  const draft = normalizeEnv({ name, variables: JSON.parse(JSON.stringify(variables)) as Variable[] });
  return { kind: 'env', id: newId(), envId, draft, saved: envSnapshot(draft), revealed: [] };
}

class AppStore {
  status = $state<'loading' | 'ready' | 'error'>('loading');
  backend = $state<BackendInfo | null>(null);
  settings = $state<Settings | null>(null);
  summaries = $state<CollectionSummary[]>([]);
  collections = $state<Collection[]>([]);
  environments = $state<Environment[]>([]);
  globals = $state<Globals>({ schema: 1, variables: [] });
  history = $state<HistorySummary[]>([]);
  historyFilter = $state('');
  tabs = $state<Tab[]>([]);
  activeTabId = $state<string | null>(null);
  consoleEntries = $state<ConsoleEntry[]>([]);
  wsConnected = $state(false);
  /** Runner del panel inferior (P4). `iterations: null` = una por fila de datos. */
  runner = $state({
    collectionId: null as string | null,
    folderId: null as string | null,
    /** `null` = el entorno activo. */
    envId: null as string | null,
    iterations: 1 as number | null,
    delayMs: 0,
    data: null as RunnerData | null,
    onlyFailures: false,
    runId: null as string | null,
    status: 'idle' as RunnerStatus,
    total: 0,
    items: [] as RunnerItem[],
    report: null as RunnerReport | null,
    error: null as string | null,
    expanded: [] as number[],
  });

  /** Panel Agente (P5): llamadas MCP por HTTP, medidor, clientes y aprobaciones pendientes. */
  agent = $state({
    calls: [] as AgentCall[],
    meter: { calls: 0, raw: 0, sent: 0 } as AgentMeter,
    clients: [] as AgentClient[],
    pending: [] as Approval[],
    mcp: { port: null, url: null, error: null } as McpStatus,
    /** Llamadas nuevas desde que se miró el panel (insignia de la pestaña Agente). */
    unseen: 0,
    /** Filtros de la lista: tool y sesión (vacío = todas). */
    tool: '',
    session: '',
    /** Aprobaciones con la respuesta en camino (botones deshabilitados). */
    deciding: [] as string[],
  });

  ui = $state({
    pane: 'collections' as Pane,
    sideOpen: true,
    drawer: null as DrawerTab | null,
    splitPx: 300,
    palette: false,
    settings: false,
    about: false,
    /** Diálogo Importar (fase 4): pegar texto o elegir archivos. */
    importOpen: false,
    /** Panel "Código" (snippets) a la derecha del editor, como en Postman. */
    code: false,
    codeLang: 'curl' as SnippetLang,
    envMenu: false,
    quicklook: false,
  });
  collapsed = $state<string[]>([]);
  toasts = $state<Toast[]>([]);
  menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);
  dialog = $state<Dialog | null>(null);

  #toastId = 0;
  #consoleId = 0;
  /** Eventos de un run que llegan por el WebSocket antes que la respuesta de `POST /runner`. */
  #pendingRunner: Record<string, RunnerEvent[]> = {};
  #stopWs: (() => void) | null = null;
  #stopTheme: (() => void) | null = null;

  get activeTab(): Tab | null {
    return this.tabs.find((tab) => tab.id === this.activeTabId) ?? null;
  }

  get activeRequestTab(): RequestTab | null {
    const tab = this.activeTab;
    return tab?.kind === 'request' ? tab : null;
  }

  get activeEnvironment(): Environment | null {
    const active = this.settings?.active_environment;
    if (!active) return null;
    return this.environments.find((e) => e.id === active || e.name === active) ?? null;
  }

  collection(id: string | null): Collection | null {
    return id ? (this.collections.find((c) => c.id === id) ?? null) : null;
  }

  requestTabs(): RequestTab[] {
    return this.tabs.filter((tab): tab is RequestTab => tab.kind === 'request');
  }

  /** Variables que la UI conoce para una pestaña: entorno activo > colección > globales > dinámicas. */
  lookupFor(tab: Tab | null): VarLookup {
    const collectionId = tab?.kind === 'request' || tab?.kind === 'docs' ? tab.collectionId : null;
    return lookupFrom([
      { scope: 'environment', vars: this.activeEnvironment?.variables },
      { scope: 'collection', vars: this.collection(collectionId)?.variables },
      { scope: 'globals', vars: this.globals.variables },
    ]);
  }

  /** Para el resaltado de {{variables}}: si resuelve y qué mostrar en el hover. */
  describerFor(tab: Tab | null): VarDescriber {
    const lookup = this.lookupFor(tab);
    return (name) => {
      const info = lookup(name);
      if (!info) return { ok: false, text: t('req.varMissing', { name }) };
      if (info.secret) return { ok: true, text: t('req.varSecret', { name }) };
      if (info.scope === 'dynamic') return { ok: true, text: t('req.varDynamic', { name }) };
      return { ok: true, text: t('req.varOk', { name, value: info.value }) };
    };
  }

  isDirty(tab: Tab): boolean {
    if (tab.kind === 'request') return snapshot(tab.draft) !== tab.saved;
    if (tab.kind === 'env') return envSnapshot(tab.draft) !== tab.saved;
    if (tab.kind === 'policy') return policySnapshot(tab.draft) !== tab.saved;
    return tab.draft !== tab.saved;
  }

  tabTitle(tab: Tab): string {
    if (tab.kind === 'request') return tab.draft.name || t('tabs.untitled');
    if (tab.kind === 'env') return tab.envId === GLOBALS ? t('side.globals') : tab.draft.name || t('dialog.newEnvironment');
    if (tab.kind === 'policy') return `${t('policy.tab')} · ${this.collection(tab.collectionId)?.name ?? ''}`;
    const collection = this.collection(tab.collectionId);
    const item = tab.itemId && collection ? locate(collection.items, tab.itemId)?.item : null;
    return `${t('docs.tab')} · ${item?.name ?? collection?.name ?? ''}`;
  }

  // ---------- arranque y recarga ----------

  async init(): Promise<void> {
    this.status = 'loading';
    try {
      this.backend = await backendInfo();
      const ui = load<UiPersisted | null>(UI_KEY, null);
      if (ui) {
        this.ui.pane = ui.pane ?? this.ui.pane;
        this.ui.sideOpen = ui.sideOpen ?? true;
        this.ui.splitPx = ui.splitPx ?? this.ui.splitPx;
        this.ui.code = ui.code ?? false;
        this.ui.codeLang = ui.codeLang ?? this.ui.codeLang;
      }
      this.collapsed = load(COLLAPSED_KEY, [] as string[]);
      await Promise.all([this.reloadSettings(), this.reloadCollections(), this.reloadEnvironments(), this.reloadGlobals()]);
      void this.reloadHistory();
      await this.initLanguage();
      this.restoreTabs();
      this.#stopTheme = watchSystemTheme(
        () => this.settings?.theme ?? 'system',
        () => undefined,
      );
      this.#stopWs = connectWs(
        (event) => void this.onEvent(event),
        (connected) => {
          // Al (re)conectar se pudieron perder llamadas o avisos del agente: se piden enteros.
          if (connected && !this.wsConnected) void this.reloadAgent();
          this.wsConnected = connected;
        },
      );
      this.status = 'ready';
    } catch (error) {
      console.error(error);
      this.status = 'error';
    }
  }

  destroy(): void {
    this.#stopWs?.();
    this.#stopTheme?.();
  }

  async onEvent(event: WsEvent): Promise<void> {
    if (event.type === 'runner_progress' || event.type === 'runner_done') return this.onRunnerEvent(event);
    if (event.type === 'agent_call' || event.type === 'agent_client' || event.type === 'agent_approval' || event.type === 'agent_approval_done') {
      return this.onAgentEvent(event);
    }
    if (event.type !== 'workspace_changed') return;
    if (event.kind === 'collection' || event.kind === 'all') await this.reloadCollections();
    if (event.kind === 'environment' || event.kind === 'all') await this.reloadEnvironments();
    if (event.kind === 'settings' || event.kind === 'all') await this.reloadSettings();
    if (event.kind === 'globals' || event.kind === 'all') await this.reloadGlobals();
  }

  async reloadSettings(): Promise<void> {
    this.settings = await api.get<Settings>('/settings');
    this.applySettings();
  }

  applySettings(): void {
    if (!this.settings) return;
    applyTheme(this.settings.theme);
    const size = this.settings.ui.font_size;
    document.documentElement.style.setProperty('--font-size', `${size}px`);
    try {
      localStorage.setItem('runpost.fontSize', String(size));
    } catch {
      /* opcional */
    }
    if (isLang(this.settings.ui.lang)) locale.lang = this.settings.ui.lang;
  }

  /** Primera vez: el idioma sigue al sistema y se guarda en settings (UX decisión 4). */
  async initLanguage(): Promise<void> {
    if (!this.settings) return;
    let initialized = false;
    try {
      initialized = localStorage.getItem(LANG_FLAG) === '1';
      localStorage.setItem(LANG_FLAG, '1');
    } catch {
      initialized = true;
    }
    if (initialized) return;
    const lang = detectLang();
    if (lang !== this.settings.ui.lang) await this.updateSettings((s) => (s.ui.lang = lang));
  }

  async reloadCollections(): Promise<void> {
    const summaries = await api.get<CollectionSummary[]>('/collections');
    const collections = await Promise.all(summaries.map((s) => api.get<Collection>(`/collections/${s.id}`)));
    this.summaries = summaries;
    this.collections = collections;
    this.syncRequestTabs();
    this.syncDocsTabs();
    this.syncPolicyTabs();
  }

  async reloadEnvironments(): Promise<void> {
    this.environments = await api.get<Environment[]>('/environments');
    this.syncEnvTabs();
  }

  async reloadGlobals(): Promise<void> {
    this.globals = await api.get<Globals>('/globals');
    this.syncEnvTabs();
  }

  async reloadHistory(): Promise<void> {
    const q = this.historyFilter.trim();
    this.history = await api.get<HistorySummary[]>(`/history?limit=100${q ? `&q=${encodeURIComponent(q)}` : ''}`);
  }

  /** Tras recargar colecciones: refresca las pestañas sin cambios; las borradas pasan a "sin guardar". */
  syncRequestTabs(): void {
    for (const tab of this.requestTabs()) {
      if (!tab.requestId) continue;
      const collection = this.collection(tab.collectionId);
      const request = collection ? findRequest(collection, tab.requestId) : null;
      if (!request) {
        tab.requestId = null;
        tab.collectionId = null;
        tab.saved = '';
        tab.draft.id = '';
        continue;
      }
      const fresh = normalizeRequest(request);
      const freshSnapshot = snapshot(fresh);
      if (freshSnapshot === tab.saved) continue;
      if (!this.isDirty(tab)) {
        tab.draft = fresh;
        tab.urlText = composeUrl(fresh.url, fresh.params);
      }
      tab.saved = freshSnapshot;
    }
  }

  /** Igual para los gestores de entornos: los borrados quedan como "sin guardar" (al guardar se crean). */
  syncEnvTabs(): void {
    for (const tab of this.tabs) {
      if (tab.kind !== 'env') continue;
      const source =
        tab.envId === GLOBALS
          ? { name: '', variables: this.globals.variables }
          : this.environments.find((e) => e.id === tab.envId);
      if (!source) {
        tab.saved = '';
        continue;
      }
      const fresh = normalizeEnv({ name: source.name, variables: JSON.parse(JSON.stringify(source.variables)) as Variable[] });
      const freshSnapshot = envSnapshot(fresh);
      if (freshSnapshot === tab.saved) continue;
      if (!this.isDirty(tab)) {
        tab.draft = fresh;
        tab.revealed = [];
      }
      tab.saved = freshSnapshot;
    }
  }

  syncDocsTabs(): void {
    for (const tab of this.tabs) {
      if (tab.kind !== 'docs') continue;
      const fresh = this.descriptionOf(tab.collectionId, tab.itemId);
      if (fresh === null) {
        tab.saved = '\u0000';
        continue;
      }
      if (fresh === tab.saved) continue;
      if (!this.isDirty(tab)) tab.draft = fresh;
      tab.saved = fresh;
    }
  }

  syncPolicyTabs(): void {
    for (const tab of this.tabs) {
      if (tab.kind !== 'policy') continue;
      const collection = this.collection(tab.collectionId);
      if (!collection) {
        tab.saved = '\u0000';
        continue;
      }
      const fresh = policyDraft(collection.agent_policy);
      const freshSnapshot = policySnapshot(fresh);
      if (freshSnapshot === tab.saved) continue;
      if (!this.isDirty(tab)) tab.draft = fresh;
      tab.saved = freshSnapshot;
    }
  }

  descriptionOf(collectionId: string, itemId: string | null): string | null {
    const collection = this.collection(collectionId);
    if (!collection) return null;
    if (!itemId) return collection.description;
    const item = locate(collection.items, itemId)?.item;
    return item ? item.description : null;
  }

  // ---------- pestañas ----------

  restoreTabs(): void {
    const stored = load(TABS_KEY, null as null | { tabs: Partial<Tab>[]; active: string | null });
    if (!stored) return;
    this.tabs = stored.tabs.flatMap((saved): Tab[] => {
      if (saved.kind === 'env') {
        if (!saved.envId) return [];
        const source = saved.envId === GLOBALS ? { name: '', variables: this.globals.variables } : this.environments.find((e) => e.id === saved.envId);
        const current = source ? makeEnvTab(saved.envId, source.name, source.variables) : null;
        if (!saved.draft) return current ? [{ ...current, id: saved.id ?? current.id }] : [];
        // Con cambios sin guardar: vuelve lo editado, salvo los valores secretos (nunca se guardan en el
        // navegador), que salen del entorno.
        const variables = saved.draft.variables.map((v) =>
          v.secret ? { ...v, value: source?.variables.find((s) => s.key === v.key)?.value ?? '' } : v,
        );
        const tab = makeEnvTab(saved.envId, saved.draft.name, variables);
        return [{ ...tab, id: saved.id ?? tab.id, saved: current?.saved ?? '' }];
      }
      if (saved.kind === 'policy') {
        const collection = this.collection(saved.collectionId ?? null);
        if (!collection) return [];
        const fresh = policyDraft(collection.agent_policy);
        const draft = (saved.draft as PolicyDraft | undefined) ?? fresh;
        return [{ kind: 'policy', id: saved.id ?? newId(), collectionId: collection.id, draft, saved: policySnapshot(fresh) }];
      }
      if (saved.kind === 'docs') {
        if (!saved.collectionId || typeof saved.draft !== 'string') return [];
        return [
          {
            kind: 'docs',
            id: saved.id ?? newId(),
            collectionId: saved.collectionId,
            itemId: saved.itemId ?? null,
            draft: saved.draft,
            saved: saved.saved ?? saved.draft,
            editing: false,
          },
        ];
      }
      // Pestañas de petición (también las guardadas antes del S4, sin `kind`).
      const request = saved as Partial<RequestTab>;
      if (!request.draft) return [];
      const tab = makeTab(request.draft, request.collectionId ?? null, request.requestId ?? null);
      tab.id = request.id ?? tab.id;
      tab.saved = request.saved ?? tab.saved;
      tab.subtab = request.subtab ?? 'params';
      return [tab];
    });
    this.activeTabId = this.tabs.some((t) => t.id === stored.active) ? stored.active : (this.tabs[0]?.id ?? null);
    this.syncRequestTabs();
    this.syncEnvTabs();
    this.syncDocsTabs();
  }

  persist(): void {
    save(TABS_KEY, {
      tabs: this.tabs.map((tab) => {
        if (tab.kind === 'request') {
          const { kind, id, collectionId, requestId, draft, saved, subtab } = tab;
          return { kind, id, collectionId, requestId, draft: $state.snapshot(draft), saved, subtab };
        }
        if (tab.kind === 'env') {
          // Sin valores secretos: el localStorage del navegador no es sitio para ellos. Sin cambios, ni
          // siquiera el borrador (se relee del entorno al abrir).
          const { kind, id, envId } = tab;
          if (!this.isDirty(tab)) return { kind, id, envId };
          const draft = $state.snapshot(tab.draft) as EnvDraft;
          return { kind, id, envId, draft: { ...draft, variables: draft.variables.map((v) => (v.secret ? { ...v, value: '' } : v)) } };
        }
        if (tab.kind === 'policy') {
          const { kind, id, collectionId } = tab;
          return this.isDirty(tab) ? { kind, id, collectionId, draft: $state.snapshot(tab.draft) } : { kind, id, collectionId };
        }
        const { kind, id, collectionId, itemId, draft, saved } = tab;
        return { kind, id, collectionId, itemId, draft, saved };
      }),
      active: this.activeTabId,
    });
    save(UI_KEY, {
      pane: this.ui.pane,
      sideOpen: this.ui.sideOpen,
      splitPx: this.ui.splitPx,
      code: this.ui.code,
      codeLang: this.ui.codeLang,
    });
    save(COLLAPSED_KEY, $state.snapshot(this.collapsed));
  }

  newTab(request: Request = emptyRequest(), collectionId: string | null = null): RequestTab {
    const tab = makeTab(request, collectionId, request.id || null);
    this.tabs.push(tab);
    this.activeTabId = tab.id;
    return tab;
  }

  openRequest(collectionId: string, requestId: string): void {
    const existing = this.requestTabs().find((tab) => tab.requestId === requestId);
    if (existing) {
      this.activeTabId = existing.id;
      return;
    }
    const collection = this.collection(collectionId);
    const request = collection ? findRequest(collection, requestId) : null;
    if (request) this.newTab(request, collectionId);
  }

  /** Abre el gestor de un entorno (o de las globales con `GLOBALS`). */
  openEnvironment(envId: string): void {
    const existing = this.tabs.find((tab) => tab.kind === 'env' && tab.envId === envId);
    if (existing) {
      this.activeTabId = existing.id;
      return;
    }
    const source = envId === GLOBALS ? { name: '', variables: this.globals.variables } : this.environments.find((e) => e.id === envId);
    if (!source) return;
    const tab = makeEnvTab(envId, source.name, source.variables);
    this.tabs.push(tab);
    this.activeTabId = tab.id;
  }

  openDocs(collectionId: string, itemId: string | null): void {
    const existing = this.tabs.find((tab) => tab.kind === 'docs' && tab.collectionId === collectionId && tab.itemId === itemId);
    if (existing) {
      this.activeTabId = existing.id;
      return;
    }
    const description = this.descriptionOf(collectionId, itemId);
    if (description === null) return;
    const tab: DocsTab = {
      kind: 'docs',
      id: newId(),
      collectionId,
      itemId,
      draft: description,
      saved: description,
      editing: description.trim() === '',
    };
    this.tabs.push(tab);
    this.activeTabId = tab.id;
  }

  /** Pestaña Agente de una colección (R4.4). */
  openPolicy(collectionId: string): void {
    const existing = this.tabs.find((tab) => tab.kind === 'policy' && tab.collectionId === collectionId);
    if (existing) {
      this.activeTabId = existing.id;
      return;
    }
    const collection = this.collection(collectionId);
    if (!collection) return;
    const draft = policyDraft(collection.agent_policy);
    const tab: PolicyTab = { kind: 'policy', id: newId(), collectionId, draft, saved: policySnapshot(draft) };
    this.tabs.push(tab);
    this.activeTabId = tab.id;
  }

  async closeTab(id: string): Promise<void> {
    const tab = this.tabs.find((t) => t.id === id);
    if (!tab) return;
    if (this.isDirty(tab)) {
      const choice = await this.askUnsaved(this.tabTitle(tab));
      if (choice === 'cancel') return;
      if (choice === 'save' && !(await this.saveTab(tab))) return;
    }
    const index = this.tabs.findIndex((t) => t.id === id);
    this.tabs.splice(index, 1);
    if (this.activeTabId === id) this.activeTabId = this.tabs[Math.min(index, this.tabs.length - 1)]?.id ?? null;
  }

  /** El usuario escribió en la barra de URL: base y query activa salen del texto. */
  setUrlText(tab: RequestTab, text: string): void {
    tab.urlText = text;
    const { url, params } = applyUrlText(text, tab.draft.params);
    tab.draft.url = url;
    tab.draft.params = params.map((p) => ({ ...p, enabled: p.enabled !== false }));
  }

  /** Se editó la tabla Params: la barra de URL se recompone. */
  paramsChanged(tab: RequestTab): void {
    tab.urlText = composeUrl(tab.draft.url, tab.draft.params);
  }

  // ---------- enviar ----------

  async send(tab: RequestTab | null = this.activeRequestTab): Promise<void> {
    if (!tab || tab.sending) return;
    if (!tab.draft.url.trim()) return;
    tab.sending = true;
    tab.error = null;
    const request = requestPayload($state.snapshot(tab.draft), tab.requestId, tab.collectionId);
    const started = Date.now();
    try {
      const result = await api.post<RunResult>('/send', {
        request,
        env_id: this.settings?.active_environment ?? null,
      });
      tab.result = result;
      // Como Postman se queda en la pestaña elegida; si un script saltó la petición, solo hay tests.
      if (result.skipped) tab.respTab = 'tests';
      this.log({
        ts: started,
        method: result.resolved.method,
        url: result.resolved.url,
        headers: result.resolved.headers,
        status: result.skipped ? null : result.response.status,
        ms: result.skipped ? null : result.response.ms,
        size: result.skipped ? null : result.response.size,
        contentType: result.response.headers.find(([k]) => k.toLowerCase() === 'content-type')?.[1] ?? '',
        error: null,
        unresolved: result.resolved.unresolved,
        console: result.console,
        skipped: result.skipped === true,
        failures: result.tests.filter((x) => !x.passed && !x.skipped).map(({ name, msg }) => ({ name, msg })),
      });
    } catch (error) {
      tab.result = null;
      tab.error =
        error instanceof ApiError
          ? {
              code: error.code,
              message: error.message,
              url: error.details.url as string | undefined,
              historyId: error.details.history_id as string | undefined,
            }
          : { code: 'request', message: String(error) };
      // Un error de red también quedó en el historial: de ahí salen las cabeceras reales (ya enmascaradas).
      const sent = tab.error.historyId ? await api.get<HistoryEntry>(`/history/${tab.error.historyId}`).catch(() => null) : null;
      this.log({
        ts: started,
        method: sent?.method ?? tab.draft.method,
        url: tab.error.url ?? tab.urlText,
        headers: sent?.req_headers ?? [],
        status: null,
        ms: null,
        size: null,
        contentType: '',
        error: `${tab.error.code}: ${tab.error.message}`,
        unresolved: [],
        console: [],
      });
    } finally {
      tab.sending = false;
      void this.reloadHistory();
    }
  }

  log(entry: Omit<ConsoleEntry, 'id'>): void {
    this.consoleEntries.push({ id: ++this.#consoleId, ...entry });
    if (this.consoleEntries.length > MAX_CONSOLE) this.consoleEntries.splice(0, this.consoleEntries.length - MAX_CONSOLE);
  }

  clearConsole(): void {
    this.consoleEntries = [];
  }

  // ---------- runner (P4) ----------

  /** "Ejecutar" del menú del árbol: abre el Runner con esa colección o carpeta. */
  openRunner(collectionId: string, folderId: string | null = null): void {
    if (this.runner.status !== 'running') {
      this.runner.collectionId = collectionId;
      this.runner.folderId = folderId;
    }
    this.ui.drawer = 'runner';
  }

  async pickRunnerData(): Promise<void> {
    const picked = await pickFiles([{ name: 'CSV / JSON', extensions: ['csv', 'json'] }], false);
    if (!picked) return;
    if (picked.kind === 'paths') {
      const path = picked.paths[0];
      this.runner.data = { name: path.split(/[\\/]/).pop() || path, path };
    } else {
      const file = picked.files[0];
      this.runner.data = { name: file.name, text: await file.text() };
    }
    // Como Postman: con datos, una iteración por fila salvo que se indique otra cosa.
    this.runner.iterations = null;
  }

  clearRunnerData(): void {
    this.runner.data = null;
    if (this.runner.iterations === null) this.runner.iterations = 1;
  }

  async startRun(): Promise<void> {
    const runner = this.runner;
    if (runner.status === 'running') return;
    const collectionId = runner.collectionId ?? this.collections[0]?.id ?? null;
    if (!collectionId) return;
    runner.collectionId = collectionId;
    runner.runId = null;
    runner.status = 'running';
    runner.total = 0;
    runner.items = [];
    runner.report = null;
    runner.error = null;
    runner.expanded = [];
    this.#pendingRunner = {};
    const iterations = runner.iterations === null ? null : Math.max(1, Math.floor(Number(runner.iterations) || 1));
    try {
      const { run_id } = await api.post<{ run_id: string }>('/runner', {
        collection_id: collectionId,
        folder_id: runner.folderId,
        env_id: runner.envId,
        iterations,
        delay_ms: Math.max(0, Math.floor(Number(runner.delayMs) || 0)),
        data: runner.data ? $state.snapshot(runner.data) : null,
      });
      runner.runId = run_id;
      const pending = this.#pendingRunner[run_id] ?? [];
      this.#pendingRunner = {};
      for (const event of pending) await this.onRunnerEvent(event);
      void this.#watchRun(run_id);
    } catch (error) {
      runner.status = 'error';
      runner.error = error instanceof ApiError ? `${error.code}: ${error.message}` : String(error);
      this.toastError(error);
    }
  }

  async cancelRun(): Promise<void> {
    if (this.runner.status !== 'running' || !this.runner.runId) return;
    try {
      await api.del(`/runner/${this.runner.runId}`);
    } catch (error) {
      this.toastError(error);
    }
  }

  async onRunnerEvent(event: RunnerEvent): Promise<void> {
    const runner = this.runner;
    if (runner.status !== 'running') return;
    if (runner.runId === null) {
      (this.#pendingRunner[event.run_id] ??= []).push(event);
      return;
    }
    if (event.run_id !== runner.runId) return;
    if (event.type === 'runner_progress') {
      if (event.event === 'started') {
        runner.total = event.total;
      } else {
        runner.total = event.total;
        runner.items.push(event.item);
      }
      return;
    }
    await this.#finishRun(event.run_id, event.error);
  }

  async #finishRun(runId: string, error?: string): Promise<void> {
    const runner = this.runner;
    if (runner.runId !== runId || runner.status !== 'running') return;
    if (error) {
      runner.status = 'error';
      runner.error = error;
      return;
    }
    try {
      const state = await api.get<{ report: RunnerReport | null }>(`/runner/${runId}`);
      if (state.report) {
        runner.report = state.report;
        runner.items = state.report.items;
      }
    } catch (failure) {
      this.toastError(failure);
    }
    runner.status = 'done';
  }

  /** Red de seguridad sin WebSocket: consulta el estado mientras el run siga en marcha. */
  async #watchRun(runId: string): Promise<void> {
    while (this.runner.runId === runId && this.runner.status === 'running') {
      await new Promise((resolve) => setTimeout(resolve, 1500));
      if (this.wsConnected || this.runner.runId !== runId || this.runner.status !== 'running') continue;
      try {
        const state = await api.get<{ status: string; total: number; items?: RunnerItem[]; error: string | null }>(`/runner/${runId}`);
        if (state.items) this.runner.items = state.items;
        this.runner.total = state.total;
        if (state.status !== 'running') await this.#finishRun(runId, state.error ?? undefined);
      } catch {
        /* sin conexión: se reintenta */
      }
    }
  }

  async exportRunReport(format: 'json' | 'html'): Promise<void> {
    const { runId, report } = this.runner;
    if (!runId || !report) return;
    const file = `runner-${slug(report.collection.name)}.${format}`;
    try {
      if (isTauri()) {
        const path = await pickSavePath(file, [{ name: format.toUpperCase(), extensions: [format] }]);
        if (!path) return;
        const saved = await api.post<{ path: string }>('/files/save', { path, runner_report: { run_id: runId, format } });
        this.toast('ok', t('runner.saved', { path: saved.path }));
      } else {
        const text =
          format === 'json'
            ? `${JSON.stringify($state.snapshot(report), null, 2)}\n`
            : await api.getText(`/runner/${runId}/report?format=html`);
        downloadText(file, text, format === 'json' ? 'application/json' : 'text/html');
        this.toast('ok', t('runner.saved', { path: file }));
      }
    } catch (error) {
      this.toastError(error);
    }
  }

  runnerExportMenu(x: number, y: number): void {
    this.openMenu(x, y, [
      { label: t('runner.exportJson'), action: () => this.exportRunReport('json') },
      { label: t('runner.exportHtml'), action: () => this.exportRunReport('html') },
    ]);
  }

  // ---------- guardar ----------

  /** Ctrl+S según el tipo de pestaña. Devuelve false si se canceló o falló. */
  async saveTab(tab: Tab | null = this.activeTab): Promise<boolean> {
    if (!tab) return false;
    try {
      if (tab.kind === 'request') return await this.saveRequestTab(tab);
      if (tab.kind === 'env') return await this.saveEnvTab(tab);
      if (tab.kind === 'policy') return await this.savePolicyTab(tab);
      return await this.saveDocsTab(tab);
    } catch (error) {
      this.toastError(error);
      return false;
    }
  }

  async saveRequestTab(tab: RequestTab): Promise<boolean> {
    const draft = normalizeRequest($state.snapshot(tab.draft));
    if (tab.requestId && tab.collectionId && this.collection(tab.collectionId)) {
      const { id: _id, ...fields } = draft;
      await api.patch(`/collections/${tab.collectionId}/items/${tab.requestId}`, fields);
    } else {
      const target = await this.askSaveAs(draft.name || t('tabs.untitled'), tab.collectionId);
      if (!target) return false;
      draft.name = target.name;
      const created = await api.post<{ id: string }>(`/collections/${target.collectionId}/items`, {
        parent_id: target.parentId,
        item: { ...draft, type: 'request', id: '' },
      });
      tab.requestId = created.id;
      tab.collectionId = target.collectionId;
      draft.id = created.id;
      tab.draft.id = created.id;
      tab.draft.name = target.name;
    }
    tab.saved = snapshot(draft);
    await this.reloadCollections();
    return true;
  }

  async saveEnvTab(tab: EnvTab): Promise<boolean> {
    const clean = normalizeEnv($state.snapshot(tab.draft) as EnvDraft);
    // Las filas sin nombre no se guardan (la fila nueva vacía, como en Postman).
    clean.variables = clean.variables.filter((v) => v.key.trim() !== '');
    if (tab.envId === GLOBALS) {
      await api.put('/globals', { schema: 1, variables: clean.variables });
      tab.draft.variables = clean.variables;
      tab.saved = envSnapshot(clean);
      await this.reloadGlobals();
      return true;
    }
    const name = clean.name.trim() || t('dialog.newEnvironment');
    const exists = this.environments.some((e) => e.id === tab.envId);
    if (exists) {
      await api.put(`/environments/${tab.envId}`, { schema: 1, id: tab.envId, name, variables: clean.variables });
    } else {
      const created = await api.post<Environment>('/environments', {
        environment: { schema: 1, id: '', name, variables: clean.variables },
      });
      tab.envId = created.id;
    }
    tab.draft = { name, variables: clean.variables };
    tab.saved = envSnapshot(tab.draft);
    await this.reloadEnvironments();
    return true;
  }

  async saveDocsTab(tab: DocsTab): Promise<boolean> {
    const collection = this.collection(tab.collectionId);
    if (!collection) throw new Error(t('docs.missing'));
    if (tab.itemId) {
      await api.patch(`/collections/${tab.collectionId}/items/${tab.itemId}`, { description: tab.draft });
    } else {
      await api.put(`/collections/${tab.collectionId}`, { ...$state.snapshot(collection), description: tab.draft });
    }
    tab.saved = tab.draft;
    tab.editing = false;
    await this.reloadCollections();
    return true;
  }

  async savePolicyTab(tab: PolicyTab): Promise<boolean> {
    const collection = this.collection(tab.collectionId);
    if (!collection) throw new Error(t('docs.missing'));
    const agentPolicy = toPolicy($state.snapshot(tab.draft) as PolicyDraft, collection.agent_policy);
    await api.put(`/collections/${tab.collectionId}`, { ...$state.snapshot(collection), agent_policy: agentPolicy });
    tab.draft = policyDraft(agentPolicy);
    tab.saved = policySnapshot(tab.draft);
    await this.reloadCollections();
    return true;
  }

  // ---------- colecciones y árbol ----------

  async createCollection(): Promise<Collection | null> {
    const name = await this.askPrompt(t('dialog.newCollection'), '', t('dialog.create'));
    if (!name?.trim()) return null;
    try {
      const collection = await api.post<Collection>('/collections', { name: name.trim() });
      await this.reloadCollections();
      return collection;
    } catch (error) {
      this.toastError(error);
      return null;
    }
  }

  async createRequest(collectionId: string, parentId: string | null): Promise<void> {
    const item = { ...emptyRequest(t('tabs.untitled')), type: 'request' };
    try {
      const created = await api.post<{ id: string }>(`/collections/${collectionId}/items`, { parent_id: parentId, item });
      await this.reloadCollections();
      if (parentId) this.expand(parentId);
      this.expand(collectionId);
      this.openRequest(collectionId, created.id);
    } catch (error) {
      this.toastError(error);
    }
  }

  async createFolder(collectionId: string, parentId: string | null): Promise<void> {
    const name = await this.askPrompt(t('dialog.newFolder'), '', t('dialog.create'));
    if (!name?.trim()) return;
    const item = { type: 'folder', id: '', name: name.trim(), description: '', auth: null, scripts: { pre: '', test: '' }, items: [] };
    try {
      await api.post(`/collections/${collectionId}/items`, { parent_id: parentId, item });
      await this.reloadCollections();
      this.expand(parentId ?? collectionId);
    } catch (error) {
      this.toastError(error);
    }
  }

  async renameCollection(collection: Collection): Promise<void> {
    const name = await this.askPrompt(t('dialog.renameTitle', { name: collection.name }), collection.name, t('dialog.rename'));
    if (!name?.trim() || name === collection.name) return;
    try {
      await api.put(`/collections/${collection.id}`, { ...$state.snapshot(collection), name: name.trim() });
      await this.reloadCollections();
    } catch (error) {
      this.toastError(error);
    }
  }

  async renameItem(collectionId: string, item: Item): Promise<void> {
    const name = await this.askPrompt(t('dialog.renameTitle', { name: item.name }), item.name, t('dialog.rename'));
    if (!name?.trim() || name === item.name) return;
    try {
      await api.patch(`/collections/${collectionId}/items/${item.id}`, { name: name.trim() });
      // Una pestaña con cambios no debe deshacer el renombrado al guardar.
      for (const tab of this.requestTabs().filter((t) => t.requestId === item.id)) tab.draft.name = name.trim();
      await this.reloadCollections();
    } catch (error) {
      this.toastError(error);
    }
  }

  async duplicateItem(collectionId: string, item: Item): Promise<void> {
    try {
      await api.post(`/collections/${collectionId}/items/${item.id}/duplicate`, {
        name: `${item.name} ${t('dialog.copySuffix')}`,
      });
      await this.reloadCollections();
    } catch (error) {
      this.toastError(error);
    }
  }

  async duplicateCollection(collection: Collection): Promise<void> {
    try {
      await api.post('/collections', {
        collection: $state.snapshot(collection),
        name: `${collection.name} ${t('dialog.copySuffix')}`,
      });
      await this.reloadCollections();
    } catch (error) {
      this.toastError(error);
    }
  }

  async deleteItem(collectionId: string, item: Item): Promise<void> {
    const body = item.type === 'folder' ? t('dialog.deleteFolder') : t('dialog.deleteRequest');
    if (!(await this.askConfirm(t('dialog.deleteTitle', { name: item.name }), body, t('dialog.delete'), true))) return;
    try {
      await api.del(`/collections/${collectionId}/items/${item.id}`);
      await this.reloadCollections();
    } catch (error) {
      this.toastError(error);
    }
  }

  async deleteCollection(collection: Collection): Promise<void> {
    if (!(await this.askConfirm(t('dialog.deleteTitle', { name: collection.name }), t('dialog.deleteCollection'), t('dialog.delete'), true)))
      return;
    try {
      await api.del(`/collections/${collection.id}`);
      await this.reloadCollections();
    } catch (error) {
      this.toastError(error);
    }
  }

  /** Arrastrar y soltar: dentro de una carpeta, antes de un hermano o al final de una colección. */
  async moveItem(
    fromCollectionId: string,
    itemId: string,
    target: { collectionId: string; parentId: string | null; before: string | null },
  ): Promise<void> {
    if (target.parentId === itemId || target.before === itemId) return;
    try {
      await api.post(`/collections/${fromCollectionId}/items/${itemId}/move`, {
        collection_id: target.collectionId,
        parent_id: target.parentId,
        before: target.before,
      });
      for (const tab of this.requestTabs().filter((t) => t.requestId === itemId)) tab.collectionId = target.collectionId;
      await this.reloadCollections();
    } catch (error) {
      this.toastError(error);
    }
  }

  isCollapsed(id: string): boolean {
    return this.collapsed.includes(id);
  }

  toggleCollapsed(id: string): void {
    this.collapsed = this.isCollapsed(id) ? this.collapsed.filter((c) => c !== id) : [...this.collapsed, id];
  }

  expand(id: string): void {
    if (this.isCollapsed(id)) this.collapsed = this.collapsed.filter((c) => c !== id);
  }

  collapse(id: string): void {
    if (!this.isCollapsed(id)) this.collapsed = [...this.collapsed, id];
  }

  locateItem(collectionId: string, id: string) {
    const collection = this.collection(collectionId);
    return collection ? locate(collection.items, id) : null;
  }

  // ---------- exportar y curl ----------

  /** Diálogo "Exportar" con "Quitar secretos" marcado por defecto (R5). */
  askExport(title: string): Promise<boolean | null> {
    return new Promise((resolve) => (this.dialog = { kind: 'export', title, strip: true, resolve }));
  }

  async exportCollection(collectionId: string, folderId: string | null = null): Promise<void> {
    const collection = this.collection(collectionId);
    if (!collection) return;
    const name = folderId ? (locate(collection.items, folderId)?.item.name ?? collection.name) : collection.name;
    const strip = await this.askExport(t('export.title', { name }));
    if (strip === null) return;
    const file = `${slug(name)}.postman_collection.json`;
    const query = `?format=postman_v21${strip ? '&strip_secrets=1' : ''}${folderId ? `&folder=${encodeURIComponent(folderId)}` : ''}`;
    await this.exportTo(file, `/export/collections/${collectionId}${query}`, {
      export: { collection_id: collectionId, folder_id: folderId, strip_secrets: strip },
    });
  }

  async exportEnvironment(env: Environment): Promise<void> {
    const strip = await this.askExport(t('export.title', { name: env.name }));
    if (strip === null) return;
    const file = `${slug(env.name)}.postman_environment.json`;
    await this.exportTo(file, `/export/environments/${env.id}${strip ? '?strip_secrets=1' : ''}`, {
      export_environment: { environment_id: env.id, strip_secrets: strip },
    });
  }

  /** App nativa: diálogo de guardar + el servidor escribe. Navegador: descarga. */
  async exportTo(file: string, getPath: string, saveBody: Record<string, unknown>): Promise<void> {
    try {
      if (isTauri()) {
        const path = await pickSavePath(file, [{ name: 'JSON', extensions: ['json'] }]);
        if (!path) return;
        const saved = await api.post<{ path: string }>('/files/save', { path, ...saveBody });
        this.toast('ok', t('export.done', { path: saved.path }));
      } else {
        const data = await api.get<unknown>(getPath);
        downloadText(file, `${JSON.stringify(data, null, 2)}\n`, 'application/json');
        this.toast('ok', t('export.done', { path: file }));
      }
    } catch (error) {
      this.toastError(error);
    }
  }

  /**
   * Snippet de código (fase 4) que genera el servidor: variables normales resueltas, auth heredada
   * y los secretos como `{{nombre}}`. `requestId` vacío = borrador de `collectionId`.
   */
  async snippet(lang: SnippetLang, request: Request, requestId: string | null, collectionId: string | null): Promise<string> {
    const { code } = await api.post<{ code: string }>(`/codegen/${lang}`, {
      request: requestPayload(request, requestId, collectionId),
      env_id: this.settings?.active_environment ?? null,
    });
    return code;
  }

  /**
   * Token OAuth 2.0 de la auth efectiva (propia o heredada) de una petición: `status`, `get`
   * ("Obtener token nuevo") o `clear`. Con authorization code, `get` abre el navegador y devuelve el
   * inicio de sesión en curso, que se espera con `oauthWait`.
   */
  async oauth(action: 'status' | 'get' | 'clear', request: Request, requestId: string | null, collectionId: string | null): Promise<OAuthResult> {
    return api.post<OAuthResult>('/oauth2', {
      request: requestPayload(request, requestId, collectionId),
      env_id: this.settings?.active_environment ?? null,
      action,
    });
  }

  async oauthWait(flow: string): Promise<OAuthToken> {
    const { token } = await api.get<{ token: OAuthToken }>(`/oauth2/flows/${encodeURIComponent(flow)}`);
    return token;
  }

  /** "Copiar como curl" de una petición guardada o de la pestaña actual. */
  async copyCurl(collectionId: string | null, request: Request): Promise<void> {
    let text: string;
    try {
      text = await this.snippet('curl', normalizeRequest(request), request.id || null, collectionId);
    } catch (error) {
      this.toastError(error);
      return;
    }
    try {
      await copyText(text);
      this.toast('ok', t('curl.copied'));
    } catch {
      await new Promise<void>((resolve) => {
        this.dialog = { kind: 'text', title: t('menu.copyCurl'), body: t('curl.copyManually'), text, resolve };
      });
    }
  }

  // ---------- historial ----------

  /** Abre una entrada: la petición guardada si existe; si no, una pestaña nueva con lo enviado. */
  async openHistory(id: string): Promise<RequestTab | null> {
    try {
      const entry = await api.get<HistoryEntry>(`/history/${id}`);
      if (entry.request_id) {
        const owner = this.collections.find((c) => findRequest(c, entry.request_id!));
        if (owner) {
          this.openRequest(owner.id, entry.request_id);
          return this.activeRequestTab;
        }
      }
      const headers: KeyValue[] = entry.req_headers
        .filter(([k]) => k.toLowerCase() !== 'user-agent')
        .map(([key, value]) => ({ key, value, enabled: true }));
      const contentType = entry.req_headers.find(([k]) => k.toLowerCase() === 'content-type')?.[1] ?? '';
      const text = entry.req_body?.encoding === 'utf8' ? entry.req_body.text : '';
      const request: Request = {
        ...emptyRequest(),
        method: entry.method,
        url: entry.url,
        headers,
        auth: { type: 'none' },
        body: text ? { mode: 'raw', lang: contentType.includes('json') ? 'json' : 'text', text } : { mode: 'none' },
      };
      const tab = this.newTab(request, null);
      if (`${entry.url} ${JSON.stringify(entry.req_headers)} ${text}`.includes('***')) this.toast('info', t('history.masked'));
      return tab;
    } catch (error) {
      this.toastError(error);
      return null;
    }
  }

  async saveHistoryToCollection(id: string): Promise<void> {
    const tab = await this.openHistory(id);
    if (tab && !tab.requestId) await this.saveTab(tab);
  }

  async clearHistory(): Promise<void> {
    if (!(await this.askConfirm(t('history.clearTitle'), t('history.clearBody'), t('history.clear'), true))) return;
    try {
      await api.del('/history');
      await this.reloadHistory();
    } catch (error) {
      this.toastError(error);
    }
  }

  // ---------- entornos y ajustes ----------

  async updateSettings(change: (settings: Settings) => void): Promise<void> {
    if (!this.settings) return;
    const next = $state.snapshot(this.settings) as Settings;
    change(next);
    this.settings = next;
    this.applySettings();
    try {
      this.settings = await api.put<Settings>('/settings', next);
    } catch (error) {
      this.toastError(error);
      await this.reloadSettings();
    }
  }

  async setActiveEnvironment(id: string | null): Promise<void> {
    await this.updateSettings((s) => (s.active_environment = id));
  }

  async cycleTheme(): Promise<void> {
    const current: ThemeMode = this.settings?.theme ?? 'system';
    await this.updateSettings((s) => (s.theme = nextTheme(current)));
  }

  async setTheme(mode: ThemeMode): Promise<void> {
    await this.updateSettings((s) => (s.theme = mode));
  }

  async createEnvironment(): Promise<void> {
    const name = await this.askPrompt(t('dialog.newEnvironment'), '', t('dialog.create'));
    if (!name?.trim()) return;
    try {
      const env = await api.post<Environment>('/environments', { name: name.trim() });
      await this.reloadEnvironments();
      this.openEnvironment(env.id);
    } catch (error) {
      this.toastError(error);
    }
  }

  async renameEnvironment(env: Environment): Promise<void> {
    const name = await this.askPrompt(t('dialog.renameTitle', { name: env.name }), env.name, t('dialog.rename'));
    if (!name?.trim() || name === env.name) return;
    try {
      await api.put(`/environments/${env.id}`, { ...$state.snapshot(env), name: name.trim() });
      for (const tab of this.tabs) if (tab.kind === 'env' && tab.envId === env.id) tab.draft.name = name.trim();
      await this.reloadEnvironments();
    } catch (error) {
      this.toastError(error);
    }
  }

  async duplicateEnvironment(env: Environment): Promise<void> {
    try {
      await api.post('/environments', { environment: $state.snapshot(env), name: `${env.name} ${t('dialog.copySuffix')}` });
      await this.reloadEnvironments();
    } catch (error) {
      this.toastError(error);
    }
  }

  async deleteEnvironment(env: Environment): Promise<void> {
    if (!(await this.askConfirm(t('dialog.deleteTitle', { name: env.name }), t('env.deleteBody'), t('dialog.delete'), true))) return;
    try {
      await api.del(`/environments/${env.id}`);
      if (this.settings?.active_environment === env.id || this.settings?.active_environment === env.name) {
        await this.setActiveEnvironment(null);
      }
      // Las pestañas sin cambios de ese entorno se cierran; con cambios, quedan para guardarse de nuevo.
      this.tabs = this.tabs.filter((tab) => !(tab.kind === 'env' && tab.envId === env.id && !this.isDirty(tab)));
      if (!this.tabs.some((tab) => tab.id === this.activeTabId)) this.activeTabId = this.tabs.at(-1)?.id ?? null;
      await this.reloadEnvironments();
    } catch (error) {
      this.toastError(error);
    }
  }

  // ---------- importar y agente ----------

  async importPicked(picked: PickedFiles | null): Promise<void> {
    if (!picked) return;
    try {
      let result: ImportResult;
      if (picked.kind === 'paths') {
        result = await api.post<ImportResult>('/import', { paths: picked.paths });
      } else {
        const form = new FormData();
        for (const file of picked.files) form.append('file', file, file.name);
        result = await api.post<ImportResult>('/import', form);
      }
      await this.applyImport(result);
    } catch (error) {
      this.toastError(error);
    }
  }

  /**
   * Texto pegado en el diálogo Importar (fase 4): un comando cURL o el contenido de una colección
   * Postman, OpenAPI, HAR, Insomnia o Bruno. `true` si se importó o abrió algo.
   */
  async importText(text: string): Promise<boolean> {
    if (!text.trim()) return false;
    try {
      const result = await api.post<ImportResult>('/import', { text });
      await this.applyImport(result);
      return result.reports.length > 0 || (result.requests?.length ?? 0) > 0;
    } catch (error) {
      this.toastError(error);
      return false;
    }
  }

  /** Avisos, recarga y, para un cURL, la petición en una pestaña nueva sin guardar (como Postman). */
  private async applyImport(result: ImportResult): Promise<void> {
    try {
      for (const request of result.requests ?? []) {
        this.newTab({ ...normalizeRequest(request), id: '' });
        this.toast('ok', t('import.curlOpened'));
      }
      for (const report of result.reports) {
        if (report.kind === 'collection') {
          this.toast('ok', t(report.replaced ? 'import.doneReplaced' : 'import.done', { name: report.name, count: report.requests }));
          this.expand(report.id);
        } else {
          this.toast('ok', t('import.doneEnv', { name: report.name, count: report.variables }));
        }
        if (report.warnings.length) {
          this.toast('info', t('import.warnings', { count: report.warnings.length, first: report.warnings[0] }));
        }
      }
      for (const error of result.errors) this.toast('err', t('import.failed', { file: error.file, message: error.message }));
      await Promise.all([this.reloadCollections(), this.reloadEnvironments()]);
      // Si no hay entorno activo y se importó uno, se activa (como Postman tras importar).
      const env = result.reports.find((r) => r.kind === 'environment');
      if (env && !this.settings?.active_environment) await this.setActiveEnvironment(env.id);
      if (result.reports.some((r) => r.kind === 'collection')) {
        this.ui.pane = 'collections';
        this.ui.sideOpen = true;
      }
    } catch (error) {
      this.toastError(error);
    }
  }

  async copyAgentConfig(client: 'claude-code' | 'codex' | 'claude-desktop'): Promise<void> {
    const names = { 'claude-code': 'claudeCode', codex: 'codex', 'claude-desktop': 'claudeDesktop' } as const;
    let config: AgentConfig;
    try {
      config = await api.get<AgentConfig>(`/agent/config?client=${client}`);
    } catch (error) {
      this.toastError(error);
      return;
    }
    if (!config.exists) this.toast('info', t('agent.missingExe', { exe: config.exe }));
    try {
      await copyText(config.text);
      this.toast('ok', t('agent.copied', { client: t(`agent.${names[client]}`), target: config.target }));
      if (config.transport === 'stdio') this.toast('info', t('agent.stdioNote'));
    } catch {
      // Sin permiso de portapapeles: se muestra para copiarla a mano.
      await new Promise<void>((resolve) => {
        this.dialog = {
          kind: 'text',
          title: t(`agent.${names[client]}`),
          body: t('agent.copyManually', { target: config.target }),
          text: config.text,
          resolve,
        };
      });
    }
  }

  agentMenu(x: number, y: number): void {
    this.openMenu(x, y, [
      { label: t('agent.claudeCode'), action: () => this.copyAgentConfig('claude-code') },
      { label: t('agent.codex'), action: () => this.copyAgentConfig('codex') },
      { label: t('agent.claudeDesktop'), action: () => this.copyAgentConfig('claude-desktop') },
    ]);
  }

  async copyId(id: string): Promise<void> {
    try {
      await copyText(id);
      this.toast('ok', t('req.copied', { id }));
    } catch {
      this.toast('err', t('common.copyFailed', { text: id }));
    }
  }

  // ---------- agente (P5, sección 19) ----------

  async reloadAgent(): Promise<void> {
    try {
      const state = await api.get<AgentState>('/agent');
      this.agent.calls = state.calls ?? [];
      this.agent.meter = state.meter ?? { calls: 0, raw: 0, sent: 0 };
      this.agent.clients = state.clients ?? [];
      this.agent.pending = state.pending ?? [];
      this.agent.mcp = state.mcp ?? { port: null, url: null, error: null };
    } catch {
      /* sin panel Agente en el servidor: queda vacío */
    }
  }

  /** ¿Está el usuario mirando las llamadas del agente? (para la insignia de nuevas). */
  get agentViewing(): boolean {
    return this.ui.drawer === 'agent' || (this.ui.sideOpen && this.ui.pane === 'agent');
  }

  onAgentEvent(event: AgentEvent): void {
    const agent = this.agent;
    if (event.type === 'agent_call') {
      const call = event.call;
      if (agent.calls.some((c) => c.id === call.id)) return;
      agent.calls.push(call);
      if (agent.calls.length > MAX_AGENT_CALLS) agent.calls.splice(0, agent.calls.length - MAX_AGENT_CALLS);
      agent.meter = { calls: agent.meter.calls + 1, raw: agent.meter.raw + call.raw, sent: agent.meter.sent + call.sent };
      if (!this.agentViewing) agent.unseen += 1;
      // Lo que ejecutó el agente aparece también en el Historial (con 🤖).
      if (call.tool === 'run_request' || call.tool === 'send') void this.reloadHistory().catch(() => undefined);
      return;
    }
    if (event.type === 'agent_client') {
      const index = agent.clients.findIndex((c) => c.client === event.client.client && c.session === event.client.session);
      if (index < 0) agent.clients.push(event.client);
      else agent.clients[index] = event.client;
      return;
    }
    if (event.type === 'agent_approval') {
      if (!agent.pending.some((a) => a.id === event.approval.id)) agent.pending.push(event.approval);
      return;
    }
    const approval = agent.pending.find((a) => a.id === event.id);
    agent.pending = agent.pending.filter((a) => a.id !== event.id);
    if (!approval) return;
    if (event.decision === 'expired') this.toast('info', t('agent.expired', { client: approval.client, request: approval.request }));
    if (event.decision === 'cancelled') this.toast('info', t('agent.cancelled', { client: approval.client, request: approval.request }));
  }

  /** Aprobar / Denegar desde el aviso o la tarjeta del panel. */
  async decideApproval(id: string, approve: boolean): Promise<void> {
    if (this.agent.deciding.includes(id)) return;
    this.agent.deciding.push(id);
    try {
      await api.post(`/agent/approvals/${id}`, { approve });
    } catch (error) {
      if (error instanceof ApiError && error.status === 404) this.toast('info', t('agent.gone'));
      else this.toastError(error);
    } finally {
      this.agent.pending = this.agent.pending.filter((a) => a.id !== id);
      this.agent.deciding = this.agent.deciding.filter((d) => d !== id);
    }
  }

  /** Una fila del panel abre su petición como pestaña normal. */
  openAgentCall(call: AgentCall): void {
    const id = call.request_id;
    const owner = id ? this.collections.find((c) => findRequest(c, id)) : undefined;
    if (!id || !owner) {
      this.toast('info', t('agent.noRequest'));
      return;
    }
    this.openRequest(owner.id, id);
  }

  showAgentPanel(): void {
    this.ui.pane = 'agent';
    this.ui.sideOpen = true;
  }

  /** Medidor en Markdown para docs/BENCH.md (19.6), al portapapeles. */
  async exportAgentMeter(): Promise<void> {
    const text = meterMarkdown($state.snapshot(this.agent.calls) as AgentCall[], this.agent.meter, new Date());
    try {
      await copyText(text);
      this.toast('ok', t('agent.meterCopied'));
    } catch {
      await new Promise<void>((resolve) => {
        this.dialog = { kind: 'text', title: t('agent.export'), body: t('agent.meterManual'), text, resolve };
      });
    }
  }

  // ---------- llavero (R5, sección 19.4) ----------

  /** "Mover al llavero": el valor sale del archivo del entorno y va al llavero del sistema. */
  async moveToKeyring(tab: EnvTab, index: number | null = null): Promise<void> {
    const targets = tab.draft.variables.filter(
      (v, i) => (index === null || i === index) && v.secret && v.store !== 'keyring' && v.key.trim() !== '',
    );
    if (targets.length === 0) return;
    for (const variable of targets) variable.store = 'keyring';
    const names = targets.map((v) => v.key);
    if (await this.saveTab(tab)) {
      this.toast('ok', names.length === 1 ? t('env.moved', { name: names[0] }) : t('env.movedMany', { count: names.length }));
    } else {
      for (const variable of targets) variable.store = 'file';
    }
  }

  // ---------- avisos, menús y diálogos ----------

  toast(kind: Toast['kind'], text: string): void {
    const id = ++this.#toastId;
    this.toasts.push({ id, kind, text });
    setTimeout(() => (this.toasts = this.toasts.filter((toast) => toast.id !== id)), kind === 'err' ? 8000 : 4500);
  }

  toastError(error: unknown): void {
    const text = error instanceof ApiError ? `${error.code}: ${error.message}` : String(error);
    this.toast('err', text);
  }

  openMenu(x: number, y: number, items: MenuItem[]): void {
    this.menu = { x, y, items };
  }

  askPrompt(title: string, value: string, confirm: string): Promise<string | null> {
    return new Promise((resolve) => (this.dialog = { kind: 'prompt', title, value, confirm, resolve }));
  }

  askConfirm(title: string, body: string, confirm: string, danger = false): Promise<boolean> {
    return new Promise((resolve) => (this.dialog = { kind: 'confirm', title, body, confirm, danger, resolve }));
  }

  askUnsaved(name: string): Promise<'save' | 'discard' | 'cancel'> {
    return new Promise((resolve) => (this.dialog = { kind: 'unsaved', name, resolve }));
  }

  askSaveAs(name: string, collectionId: string | null) {
    return new Promise<{ collectionId: string; parentId: string | null; name: string } | null>(
      (resolve) => (this.dialog = { kind: 'saveAs', name, collectionId, resolve }),
    );
  }

  closeDialog(): void {
    this.dialog = null;
  }
}

/** Nombre de archivo a partir de un nombre (como el slug del servidor). */
export function slug(name: string): string {
  const text = name
    .normalize('NFD')
    .replace(/[\u0300-\u036f]/g, '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
  return text || 'export';
}

export const store = new AppStore();
