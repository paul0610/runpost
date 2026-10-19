// Capturas de la app (modo navegador, misma SPA que la ventana Tauri) y del mockup, a 1280×800,
// con Edge sin interfaz vía CDP. Herramienta de desarrollo para los informes de sprints de UI.
// Solo captura la página (Page.captureScreenshot), nunca el escritorio.
//
// Requisitos: Edge instalado; `runpost serve --browser` sobre un workspace con la colección Acme y
// el entorno falso de tests/fixtures (activo); y scripts/dev/mock_gateway.py escuchando en 3080
// (gatewayUrl del entorno falso) para que "#7 sign" devuelva un 500 como en el mockup.
// Para s4 conviene además un segundo entorno y una llamada hecha por MCP (marca 🤖 del historial).
//
// Para s5: scripts/dev/mock_acme.py, el token del entorno falso cambiado a "fake-view-token" y
// <dataFile> (tests/fixtures/tokens-matriz-fake.csv) como séptimo argumento.
//
// Para s6: scripts/dev/mock_acme.py y el servidor con RUNPOST_KEYRING=memory (nunca el llavero real)
// y settings.mcp.http_port libre; el script hace de Claude Code por MCP sobre HTTP.
//
// Uso: node scripts/dev/ui-screens.mjs <outDir> <appUrl> <mockupFileUrl> <apiPort> <token> [s3|s4|s5|s6|s7|gif] [dataFile]
import { spawn, spawnSync } from 'node:child_process';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const [outDir, appUrl, mockupUrl, apiPort, token, sprint = 's3', dataFile] = process.argv.slice(2);
const EDGE = 'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe';
const PORT = 9333;
const profile = mkdtempSync(join(tmpdir(), 'runpost-edge-'));
const edge = spawn(EDGE, ['--headless=new', `--remote-debugging-port=${PORT}`, `--user-data-dir=${profile}`, '--hide-scrollbars', '--disable-lcd-text', '--no-first-run', 'about:blank'], { stdio: 'ignore' });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function json(path, method = 'GET') {
  for (let i = 0; i < 50; i++) {
    try { return await (await fetch(`http://127.0.0.1:${PORT}${path}`, { method })).json(); } catch { await sleep(200); }
  }
  throw new Error('edge not reachable');
}

await json('/json/version');
const target = await json('/json/new?about:blank', 'PUT');
const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));
let next = 1;
const pending = new Map();
const waiters = [];
ws.onmessage = (m) => {
  const msg = JSON.parse(m.data);
  if (msg.id && pending.has(msg.id)) { pending.get(msg.id)(msg); pending.delete(msg.id); }
  if (msg.method) waiters.filter((w) => w.method === msg.method).forEach((w) => w.resolve(msg));
};
const send = (method, params = {}) => new Promise((resolve) => { const id = next++; pending.set(id, resolve); ws.send(JSON.stringify({ id, method, params })); });
const once = (method) => new Promise((resolve) => waiters.push({ method, resolve }));
// Ayudas disponibles dentro de la página en cada evaluate.
const HELPERS = `
  const wait = (ms) => new Promise(r => setTimeout(r, ms));
  const row = (text) => [...document.querySelectorAll('.side .row')].find(r => r.textContent.includes(text));
  const key = (k, o = {}) => window.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, ...o }));
  // Sección de la barra lateral (0 colecciones, 1 entornos, 2 historial); pulsar la activa plegaría el panel.
  const pane = async (i) => {
    const b = document.querySelectorAll('.rail button')[i];
    if (!b.classList.contains('on') || !document.querySelector('.side')) b.click();
    await wait(300);
  };
  const menuOn = async (el, label) => {
    const r = el.getBoundingClientRect();
    el.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: r.left + 60, clientY: r.top + 10 }));
    await wait(300);
    [...document.querySelectorAll('[role=menu] button')].find(b => b.textContent.trim() === label).click();
    await wait(400);
  };
`;
const evaluate = async (expression) => {
  const r = await send('Runtime.evaluate', { expression: `(async () => { ${HELPERS} ${expression} })()`, awaitPromise: true, returnByValue: true });
  if (r.result?.exceptionDetails) throw new Error(JSON.stringify(r.result.exceptionDetails));
  return r.result?.result?.value;
};
const shot = async (name) => {
  const r = await send('Page.captureScreenshot', { format: 'png' });
  writeFileSync(join(outDir, name), Buffer.from(r.result.data, 'base64'));
  console.log('saved', name);
};
const navigate = async (url) => { const loaded = once('Page.loadEventFired'); await send('Page.navigate', { url }); await loaded; await sleep(1200); };
const api = (path, body, method = body ? 'PUT' : 'GET') => fetch(`http://127.0.0.1:${apiPort}/api${path}`, { method, headers: { 'X-RunPost-Token': token, 'Content-Type': 'application/json' }, body: body ? JSON.stringify(body) : undefined }).then((r) => r.json());
const setTheme = async (theme) => { const s = await api('/settings'); s.theme = theme; s.ui.lang = 'es'; await api('/settings', s); };

await send('Page.enable');
await send('Emulation.setDeviceMetricsOverride', { width: 1280, height: 800, deviceScaleFactor: 1, mobile: false });

async function s3() {
  // ---- mockup ----
  await navigate(mockupUrl);
  await shot('mockup-dark.png');
  await evaluate(`document.getElementById('d-empty').click();`);
  await shot('mockup-empty-dark.png');
  await evaluate(`document.getElementById('d-work').click(); document.getElementById('d-theme').click();`);
  await shot('mockup-light.png');

  // ---- app ----
  await setTheme('dark');
  await navigate(appUrl);
  await shot('app-empty-dark.png');
  await evaluate(`
    row('00.2 Gateway - ping').click(); await wait(300);
    row('httpbin GET')?.click(); await wait(300);
    row('#7 POST sign (sonda').click(); await wait(300);
    row('02 - Controles negativos').click(); await wait(200);
    [...document.querySelectorAll('.editor .subtabs button')].find(b => b.innerText.startsWith('Headers')).click(); await wait(200);
    key('Enter', { ctrlKey: true });
    await wait(2500);
  `);
  await shot('app-dark.png');
  await setTheme('light');
  await navigate(appUrl);
  await evaluate(`await wait(300); key('Enter', { ctrlKey: true }); await wait(2500);`);
  await shot('app-light.png');
}

// S4: gestor de entornos (P3), historial, consola y vista rápida frente al mockup; Docs y Exportar
// (sin equivalente en el mockup) solo de la app.
async function s4() {
  // ---- mockup (arranca en oscuro) ----
  await navigate(mockupUrl);
  const mockup = async (theme) => {
    await evaluate(`document.querySelector('.rail [data-side=env]').click(); await wait(200);`);
    await shot(`mockup-env-${theme}.png`);
    await evaluate(`document.querySelector('.rail [data-side=his]').click(); await wait(200);`);
    await shot(`mockup-history-${theme}.png`);
    await evaluate(`
      document.querySelector('.rail [data-side=col]').click();
      const con = document.querySelector('.drawer .bar [data-dr=con]');
      if (!con.classList.contains('on')) con.click();
      await wait(200);
    `);
    await shot(`mockup-console-${theme}.png`);
    await evaluate(`document.querySelector('.drawer .bar [data-dr=con]').click(); document.getElementById('eye').click(); await wait(200);`);
    await shot(`mockup-quicklook-${theme}.png`);
    await evaluate(`document.body.click(); await wait(100);`);
  };
  await mockup('dark');
  await evaluate(`document.getElementById('d-theme').click(); await wait(200);`);
  await mockup('light');

  // ---- app ----
  const app = async (theme) => {
    await setTheme(theme);
    await navigate(appUrl);
    // Dos envíos para la consola (vive en memoria, se repite en cada tema).
    await evaluate(`
      await pane(0);
      row('00.2 Gateway - ping').click(); await wait(300);
      key('Enter', { ctrlKey: true }); await wait(1500);
      row('#7 POST sign (sonda').click(); await wait(300);
      [...document.querySelectorAll('.editor .subtabs button')].find(b => b.innerText.startsWith('Headers')).click(); await wait(200);
      key('Enter', { ctrlKey: true }); await wait(1500);
      if (!document.querySelector('.drawer .content')) key('c', { ctrlKey: true, altKey: true });
      await wait(400);
    `);
    await shot(`app-console-${theme}.png`);
    await evaluate(`
      key('c', { ctrlKey: true, altKey: true }); await wait(200);
      await pane(1);
      [...document.querySelectorAll('.side .env-row')].find(r => r.textContent.includes('DESA')).querySelector('button.open').click();
      await wait(500);
    `);
    await shot(`app-env-${theme}.png`);
    await evaluate(`
      await pane(2);
      [...document.querySelectorAll('.tab')].find(t => t.textContent.includes('#7 POST sign')).click();
      await wait(500);
    `);
    await shot(`app-history-${theme}.png`);
    await evaluate(`document.querySelector('.quick-wrap > button').click(); await wait(300);`);
    await shot(`app-quicklook-${theme}.png`);
    await evaluate(`
      document.querySelector('.quick-wrap > button').click(); await wait(200);
      await pane(0);
    `);
  };
  await app('dark');
  await evaluate(`await menuOn(document.querySelector('[role=tree] [role=treeitem]'), 'Exportar');`);
  await shot('app-export-dark.png');
  await evaluate(`[...document.querySelectorAll('.dialog button')].find(b => b.textContent.trim() === 'Cancelar').click(); await wait(200);`);
  await app('light');
  await evaluate(`await menuOn(document.querySelector('[role=tree] [role=treeitem]'), 'Ver documentación');`);
  await shot('app-docs-light.png');
}


// S5: Runner (P4, pestaña del panel inferior) y pestaña Tests frente al mockup.
async function s5() {
  await navigate(mockupUrl);
  const mockupRunner = async (theme) => {
    await evaluate(`
      const run = document.querySelector('.drawer .bar [data-dr=run]');
      if (!run.classList.contains('on')) run.click();
      await wait(200);
    `);
    await shot(`mockup-runner-${theme}.png`);
  };
  await mockupRunner('dark');
  await evaluate(`document.getElementById('d-theme').click(); await wait(200);`);
  await mockupRunner('light');

  const app = async (theme) => {
    await setTheme(theme);
    await navigate(appUrl);
    // Envío de la sonda #7 con el token de solo lectura: su test falla (pestaña Tests y consola).
    await evaluate(`
      await pane(0);
      row('#7 POST sign (sonda').click(); await wait(300);
      key('Enter', { ctrlKey: true }); await wait(1500);
      [...document.querySelectorAll('.resp [role=tab]')].find(b => b.textContent.includes('Tests')).click();
      await wait(300);
    `);
    await shot(`app-tests-${theme}.png`);
    await evaluate(`key('c', { ctrlKey: true, altKey: true }); await wait(1000);`);
    await shot(`app-console-scripts-${theme}.png`);
    // Runner con el archivo de datos (dos tokens): Ejecutar desde el menú del árbol.
    await evaluate(`
      key('c', { ctrlKey: true, altKey: true }); await wait(200);
      await menuOn(document.querySelector('[role=tree] [role=treeitem]'), 'Ejecutar');
      await wait(300);
    `);
    await send('Page.setInterceptFileChooserDialog', { enabled: true });
    const chooser = once('Page.fileChooserOpened');
    // Un selector de archivos necesita un gesto de usuario real: clic con el ratón de CDP.
    const at = await evaluate(`
      const r = [...document.querySelectorAll('.runner button')].find(b => b.textContent.includes('Elegir CSV')).getBoundingClientRect();
      return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
    `);
    for (const type of ['mousePressed', 'mouseReleased']) {
      await send('Input.dispatchMouseEvent', { type, x: at.x, y: at.y, button: 'left', clickCount: 1 });
    }
    const opened = await chooser;
    await send('DOM.setFileInputFiles', { files: [dataFile], backendNodeId: opened.params.backendNodeId });
    await send('Page.setInterceptFileChooserDialog', { enabled: false });
    await evaluate(`
      await wait(500);
      [...document.querySelectorAll('.runner button')].find(b => b.textContent.includes('Ejecutar')).click();
      for (let i = 0; i < 100; i += 1) {
        await wait(100);
        if (document.querySelector('.runner .summary')?.textContent.includes('ms')) break;
      }
      const failed = [...document.querySelectorAll('.runner tr.row.fail')][0];
      failed?.click();
      await wait(800);
    `);
    await shot(`app-runner-${theme}.png`);
  };
  await app('dark');
  await app('light');
}

// Datos de la demo del agente (S6 y GIF del S7): un DELETE protegido (`require_approval`), el
// accessToken en el llavero (de memoria) y un "Claude Code" simulado por MCP sobre HTTP.
async function agentDemo() {
  const summaries = await api('/collections');
  const collectionId = summaries.find((c) => c.name.startsWith('Acme')).id;
  let collection = await api(`/collections/${collectionId}`);
  if (!collection.items.some((i) => i.name.startsWith('#8'))) {
    await api(`/collections/${collectionId}/items`, { parent_id: null, item: { type: 'request', id: '', name: '#8 DELETE account (sonda)', method: 'DELETE', url: '{{apiUrl}}/v1/{{currency}}/accounts/{{accountId}}', headers: [{ key: 'Authorization', value: 'Bearer {{accessToken}}', enabled: true }] } }, 'POST');
    collection = await api(`/collections/${collectionId}`);
  }
  collection.agent_policy = { require_approval: ['DELETE'], max_calls_per_minute: 30 };
  await api(`/collections/${collectionId}`, collection);
  const env = (await api('/environments')).find((e) => e.name.includes('DESA'));
  env.variables = env.variables.map((v) => (v.key === 'accessToken' ? { ...v, store: 'keyring' } : v));
  await api(`/environments/${env.id}`, env);

  // ---- Claude Code simulado por MCP sobre HTTP ----
  const mcpUrl = (await api('/agent')).mcp.url;
  let session = null;
  let rpc = 0;
  const post = async (body) => {
    const headers = { 'X-RunPost-Token': token, 'Content-Type': 'application/json', Accept: 'application/json, text/event-stream' };
    if (session) Object.assign(headers, { 'mcp-session-id': session, 'mcp-protocol-version': '2025-06-18' });
    const response = await fetch(mcpUrl, { method: 'POST', headers, body: JSON.stringify(body) });
    session = response.headers.get('mcp-session-id') ?? session;
    return response.text();
  };
  const call = async (method, params) => {
    const id = ++rpc;
    const text = await post({ jsonrpc: '2.0', id, method, params });
    const messages = text.split('\n').filter((l) => l.startsWith('data:')).map((l) => l.slice(5).trim()).filter(Boolean).map((l) => JSON.parse(l));
    return messages.find((m) => m.id === id) ?? (text ? JSON.parse(text) : null);
  };
  const tool = (name, args) => call('tools/call', { name, arguments: args });
  await call('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'claude-code', version: '2.1.302' } });
  await post({ jsonrpc: '2.0', method: 'notifications/initialized' });
  await call('tools/list', {});
  return { tool, env };
}

// S6: panel Agente (P5), aviso de aprobación (R4.3) y pestaña Agente del panel inferior frente al
// mockup, más la pestaña Agente de la colección (R4.4) y el llavero en Entornos (R5). El script hace
// de Claude Code: MCP Streamable HTTP contra /mcp con el mismo token local.
async function s6() {
  // ---- mockup: panel Agente + pestaña Agente del panel inferior + aviso; y Entornos ----
  await navigate(mockupUrl);
  const mockup = async (theme) => {
    await evaluate(`
      document.querySelector('.rail [data-side=ag]').click();
      const ag = document.querySelector('.drawer .bar [data-dr=ag]');
      if (!ag.classList.contains('on')) ag.click();
      document.getElementById('d-toast').click();
      await wait(300);
    `);
    await shot(`mockup-agent-${theme}.png`);
    await evaluate(`document.getElementById('t-no').click(); document.querySelector('.rail [data-side=env]').click(); await wait(200);`);
    await shot(`mockup-env-${theme}.png`);
  };
  await mockup('dark');
  await evaluate(`document.getElementById('d-theme').click(); await wait(200);`);
  await mockup('light');

  const { tool, env } = await agentDemo();
  await tool('run_request', { request: '00.2 Gateway - ping' });
  await tool('run_request', { request: '#7 POST sign (sonda de autorización)', select: '$.error' });
  await tool('env_set', { env: env.name, vars: { scopeLabel: 'accounts_read' } });
  await tool('run_collection', { collection: 'Acme Gateway Scopes', folder: '01 - Matriz de scopes (token bajo prueba)' });

  const app = async (theme) => {
    await setTheme(theme);
    await navigate(appUrl);
    // Con la app conectada (WebSocket), el DELETE protegido espera la decisión del usuario.
    await evaluate(`await pane(0); row('00.2 Gateway - ping').click(); await wait(300);`);
    const waiting = tool('run_request', { request: '#8 DELETE account (sonda)' });
    await evaluate(`
      const agentButton = document.querySelectorAll('.rail button')[3];
      if (!agentButton.classList.contains('on')) agentButton.click();
      await wait(300);
      const tab = [...document.querySelectorAll('.drawer .bar [role=tab]')].find(b => b.textContent.includes('Agente'));
      if (!tab.classList.contains('on')) tab.click();
      for (let i = 0; i < 50 && !document.querySelector('.toasts .toast[role=alert]'); i += 1) await wait(100);
      await wait(400);
    `);
    await shot(`app-agent-${theme}.png`);
    // Aprobar desde el aviso: se ejecuta y aparece en el panel.
    await evaluate(`[...document.querySelectorAll('.toasts .toast[role=alert] button')].find(b => b.textContent.trim() === 'Aprobar').click(); await wait(200);`);
    await waiting;
    await evaluate(`await wait(800);`);
    await shot(`app-agent-approved-${theme}.png`);
    if (theme === 'dark') {
      // "Exportar" del medidor: la tabla Markdown que va a docs/BENCH.md (por el portapapeles).
      await send('Browser.grantPermissions', { permissions: ['clipboardReadWrite', 'clipboardSanitizedWrite'], origin: new URL(appUrl).origin });
      await send('Emulation.setFocusEmulationEnabled', { enabled: true });
      const meter = await evaluate(`
        [...document.querySelectorAll('.side button')].find(b => b.textContent.trim() === 'Exportar').click();
        await wait(500);
        try { return await navigator.clipboard.readText(); } catch (e) { return 'clipboard: ' + e; }
      `);
      writeFileSync(join(outDir, 'meter.md'), meter);
      console.log('saved meter.md');
    }
    // Pestaña Agente de la colección (clic derecho → Agente…).
    await evaluate(`
      const drawerTab = [...document.querySelectorAll('.drawer .bar [role=tab]')].find(b => b.classList.contains('on'));
      drawerTab?.click();
      await pane(0);
      await menuOn(document.querySelector('[role=tree] [role=treeitem]'), 'Agente…');
      await wait(300);
    `);
    await shot(`app-policy-${theme}.png`);
    // Entornos: accessToken en el llavero, accountPassphrase todavía en archivo.
    await evaluate(`
      await pane(1);
      [...document.querySelectorAll('.side .env-row')].find(r => r.textContent.includes('DESA')).querySelector('button.open').click();
      await wait(500);
    `);
    await shot(`app-env-keyring-${theme}.png`);
  };
  await app('dark');
  await app('light');
}

// GIF de unos 20 s para el README y la web (S7): una petición, el cambio de tema y el panel Agente
// con un "Claude Code" que trabaja, incluida una aprobación. Fotogramas por CDP con su hora real;
// ffmpeg los monta (paleta propia, 960 px). Requisitos como s6, más ffmpeg en el PATH.
async function gif() {
  const { tool } = await agentDemo();
  await setTheme('light');
  await navigate(appUrl);
  // Empieza con la petición ya abierta: la pantalla de bienvenida enseña la ruta local del workspace.
  await evaluate(`await pane(0); await wait(600); row('00.2 Gateway - ping').click(); await wait(900);`);
  const framesDir = mkdtempSync(join(tmpdir(), 'runpost-gif-'));
  const frames = [];
  let recording = true;
  const recorder = (async () => {
    while (recording) {
      const shotAt = Date.now();
      const r = await send('Page.captureScreenshot', { format: 'png' });
      const name = `f${String(frames.length).padStart(4, '0')}.png`;
      writeFileSync(join(framesDir, name), Buffer.from(r.result.data, 'base64'));
      frames.push({ name, at: shotAt });
      await sleep(Math.max(0, 110 - (Date.now() - shotAt)));
    }
  })();
  // 1. Una petición guardada y su respuesta.
  await evaluate(`await wait(1500); key('Enter', { ctrlKey: true }); await wait(2000);`);
  // 2. Otra con sus tests.
  await evaluate(`row('#7 POST sign (sonda').click(); await wait(700); key('Enter', { ctrlKey: true }); await wait(1500);
    [...document.querySelectorAll('.resp [role=tab]')].find(b => b.textContent.includes('Tests')).click(); await wait(1600);`);
  // 3. Tema oscuro (Ctrl+Mayús+L).
  await evaluate(`key('l', { ctrlKey: true, shiftKey: true }); await wait(1800);`);
  // 4. El panel Agente mientras "Claude Code" trabaja; una petición protegida pide aprobación.
  await evaluate(`
    document.querySelectorAll('.rail button')[3].click(); await wait(300);
    const tab = [...document.querySelectorAll('.drawer .bar [role=tab]')].find(b => b.textContent.includes('Agente'));
    if (!tab.classList.contains('on')) tab.click();
    await wait(600);
  `);
  for (const [name, args] of [
    ['run_request', { request: '00.2 Gateway - ping' }],
    ['run_request', { request: '#7 POST sign (sonda de autorización)', select: '$.error' }],
    ['run_collection', { collection: 'Acme Gateway Scopes', folder: '01 - Matriz de scopes (token bajo prueba)' }],
  ]) {
    await tool(name, args);
    await sleep(800);
  }
  const waiting = tool('run_request', { request: '#8 DELETE account (sonda)' });
  await sleep(3200);
  await evaluate(`[...document.querySelectorAll('.toasts .toast[role=alert] button')].find(b => b.textContent.trim() === 'Aprobar').click();`);
  await waiting;
  await sleep(2600);
  recording = false;
  await recorder;
  const list = ['ffconcat version 1.0'];
  frames.forEach((f, i) => {
    const next = frames[i + 1]?.at ?? f.at + 1500;
    list.push(`file '${f.name}'`, `duration ${((next - f.at) / 1000).toFixed(3)}`);
  });
  list.push(`file '${frames.at(-1).name}'`);
  writeFileSync(join(framesDir, 'frames.txt'), list.join('\n'));
  const out = join(outDir, 'runpost-demo.gif');
  const filter = 'scale=960:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=128:stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle';
  const ffmpeg = spawnSync('ffmpeg', ['-y', '-loglevel', 'error', '-f', 'concat', '-safe', '0', '-i', join(framesDir, 'frames.txt'), '-vf', filter, '-loop', '0', out], { stdio: 'inherit' });
  if (ffmpeg.status !== 0) throw new Error('ffmpeg failed');
  const seconds = (frames.at(-1).at - frames[0].at) / 1000;
  console.log(`saved runpost-demo.gif (${frames.length} frames, ${seconds.toFixed(1)} s)`);
  try { rmSync(framesDir, { recursive: true, force: true }); } catch { /* opcional */ }
}

// S7: pantalla "Acerca de" (Ajustes → Acerca de RunPost, con los datos técnicos desplegados).
async function s7() {
  for (const theme of ['dark', 'light']) {
    await setTheme(theme);
    await navigate(appUrl);
    await evaluate(`
      key(',', { ctrlKey: true }); await wait(400);
      [...document.querySelectorAll('.dialog button')].find(b => b.textContent.includes('Acerca de')).click(); await wait(400);
      [...document.querySelectorAll('.dialog button')].find(b => b.textContent.includes('Datos técnicos')).click(); await wait(400);
    `);
    await shot(`app-about-${theme}.png`);
  }
}

// S8 (fase 4): las pantallas nuevas, como en Postman, en claro y oscuro. Requisitos como s6; además
// añade a la colección de ejemplo peticiones con OAuth 2.0 y body GraphQL, y dos ejemplos guardados.
async function s8() {
  const summaries = await api('/collections');
  const collectionId = summaries.find((c) => c.name.startsWith('Acme')).id;
  const collection = await api(`/collections/${collectionId}`);
  const add = async (name, extra) => {
    if (!collection.items.some((i) => i.name === name)) {
      await api(`/collections/${collectionId}/items`, { parent_id: null, item: { type: 'request', id: '', name, method: 'GET', ...extra } }, 'POST');
    }
  };
  // OAuth 2.0: client credentials contra el token falso del mock y authorization code (solo campos).
  await add('OAuth 2.0 - sesión', { url: '{{apiUrl}}/v1/session', auth: { type: 'oauth2_cc', token_url: '{{apiUrl}}/oauth/token', client_id: 'acme-client', client_secret: '{{encryptPassword}}', scope: 'accounts_read' } });
  await add('OAuth 2.0 - login', { url: '{{apiUrl}}/v1/session', auth: { type: 'oauth2_code', auth_url: 'https://id.acme.test/authorize', token_url: 'https://id.acme.test/oauth/token', client_id: 'acme-app', client_secret: '', scope: 'openid accounts_read', redirect_uri: 'http://127.0.0.1:7880/callback' } });
  if (!collection.items.some((i) => i.name === 'GraphQL - cuenta')) {
    const query = 'query Account($id: ID!) {\n  account(id: $id) {\n    id\n    currency\n    balance\n  }\n}';
    await api(`/collections/${collectionId}/items`, { parent_id: null, item: { type: 'request', id: '', name: 'GraphQL - cuenta', method: 'POST', url: '{{apiUrl}}/graphql', body: { mode: 'graphql', query, variables: '{\n  "id": "{{accountId}}"\n}' } } }, 'POST');
  }
  // Ejemplos de la sonda sign: el 500 del gateway local (token con permiso) y el 401 sin token.
  const byName = (items, name) => {
    for (const item of items) {
      if (item.name.startsWith(name)) return item;
      const found = item.items && byName(item.items, name);
      if (found) return found;
    }
    return null;
  };
  const items = (await api(`/collections/${collectionId}`)).items;
  const probe = byName(items, '#7 POST sign (sonda');
  if (!probe.examples?.length) {
    const envId = (await api('/settings')).active_environment;
    const examples = `/collections/${collectionId}/items/${probe.id}/examples`;
    const allowed = await api('/send', { request: probe, env_id: envId }, 'POST');
    await api(examples, { history_id: allowed.history_id, name: 'Token con permiso (error del gateway local)' }, 'POST');
    // La misma llamada sin token es la del control negativo de la colección.
    const anonymous = await api('/send', { request: byName(items, '#7 sign SIN token'), env_id: envId }, 'POST');
    await api(examples, { history_id: anonymous.history_id, name: 'Sin token' }, 'POST');
  }
  for (const theme of ['light', 'dark']) {
    await setTheme(theme);
    await navigate(appUrl);
    // Panel Código: snippet de Python de una petición con token secreto (queda como {{accessToken}}).
    await evaluate(`await pane(0); row('#7 POST sign (sonda').click(); await wait(500);
      const code = document.querySelector('.urlbar button.snippet'); if (!code.classList.contains('on')) code.click(); await wait(300);
      const select = document.querySelector('.code-panel select'); select.value = 'python'; select.dispatchEvent(new Event('change', { bubbles: true }));
      await wait(900);`);
    await shot(`app-code-${theme}.png`);
    // Body GraphQL: consulta y variables lado a lado.
    await evaluate(`document.querySelector('.code-panel header button').click(); await wait(200);
      row('GraphQL - cuenta').click(); await wait(500);
      [...document.querySelectorAll('.subtabs [role=tab]')].find(b => b.textContent.startsWith('Body')).click(); await wait(600);`);
    await shot(`app-graphql-${theme}.png`);
    // Importar: un cURL pegado se abre en una pestaña nueva sin guardar.
    const curl = "curl 'https://api.acme.test/v1/accounts?page=2' -H 'Accept: application/json' -H 'Authorization: Bearer {{accessToken}}'";
    await evaluate(`[...document.querySelectorAll('.side button')].find(b => b.textContent.includes('Importar')).click(); await wait(300);
      const area = document.querySelector('.dialog textarea'); area.value = ${JSON.stringify(curl)};
      area.dispatchEvent(new Event('input', { bubbles: true })); await wait(300);`);
    await shot(`app-import-${theme}.png`);
    await evaluate(`document.querySelector('.dialog button.primary').click(); await wait(900);`);
    await shot(`app-import-curl-${theme}.png`);
    // Auth OAuth 2.0: client credentials con su token listo (el mock lo da) y authorization code.
    await evaluate(`row('OAuth 2.0 - sesión').click(); await wait(500);
      [...document.querySelectorAll('.subtabs [role=tab]')].find(b => b.textContent.startsWith('Auth')).click(); await wait(500);
      [...document.querySelectorAll('.token button')].find(b => b.classList.contains('primary')).click();
      // Los avisos se van a los 4,5 s; el panel del token queda a la vista.
      await wait(5000); document.querySelector('.token').scrollIntoView({ block: 'end' }); await wait(300);`);
    await shot(`app-oauth-cc-${theme}.png`);
    // El formulario de authorization code es largo: se baja la división petición/respuesta (↓ en el
    // separador, como haría el usuario) y luego se devuelve.
    const moveSplit = (key) => `const split = document.querySelector('.split');
      for (let i = 0; i < 12; i++) split.dispatchEvent(new KeyboardEvent('keydown', { key: '${key}', bubbles: true }));
      await wait(300);`;
    await evaluate(`row('OAuth 2.0 - login').click(); await wait(500);
      [...document.querySelectorAll('.subtabs [role=tab]')].find(b => b.textContent.startsWith('Auth')).click(); await wait(800);
      ${moveSplit('ArrowDown')}`);
    await shot(`app-oauth-code-${theme}.png`);
    await evaluate(moveSplit('ArrowUp'));
    // Gestionar cookies: dos cookies de ejemplo (una de dominio) y una en edición.
    await api('/cookies', { domain: 'api.acme.test', cookie: 'session=fake-session-id; Path=/; HttpOnly; Secure' });
    await api('/cookies', { domain: 'acme.test', cookie: 'theme=dark; Domain=acme.test; Path=/; Max-Age=86400' });
    await evaluate(`document.querySelector('.cookies-link').click(); await wait(700);
      document.querySelector('.cookie .name').click(); await wait(300);`);
    await shot(`app-cookies-${theme}.png`);
    await evaluate(`document.querySelector('.dialog header button').click(); await wait(200);`);
    // Ajustes > Certificados de cliente, con uno guardado.
    const settings = await api('/settings');
    settings.http.certificates = [{ host: 'api.acme.test', port: 8443, cert: 'C:/certs/acme-client.crt', key: 'C:/certs/acme-client.key' }];
    await api('/settings', settings);
    await evaluate(`[...document.querySelectorAll('button')].find(b => b.title.startsWith('Ajustes')).click(); await wait(500);
      [...document.querySelectorAll('.dialog button.advanced')].at(-1).click(); await wait(300);
      document.querySelector('.cert-form').scrollIntoView({ block: 'end' }); await wait(200);`);
    await shot(`app-certs-${theme}.png`);
    settings.http.certificates = [];
    await api('/settings', settings);
    // Ejemplos guardados de la sonda, con la respuesta enviada debajo y su "Guardar como ejemplo".
    await evaluate(`key('Escape'); await wait(300); await pane(0);
      row('#7 POST sign (sonda').click(); await wait(500);
      [...document.querySelectorAll('.editor .subtabs [role=tab]')].find(b => b.textContent.startsWith('Ejemplos')).click(); await wait(300);
      key('Enter', { ctrlKey: true }); await wait(2000);`);
    await shot(`app-examples-${theme}.png`);
    // Buscar y reemplazar en la colección: "accountId" en URLs, cuerpos y variables.
    await evaluate(`await menuOn(row('Acme'), 'Buscar y reemplazar…');
      const [find, replace] = document.querySelectorAll('.dialog .inputs input');
      find.value = 'accountId'; find.dispatchEvent(new Event('input', { bubbles: true }));
      replace.value = 'walletId'; replace.dispatchEvent(new Event('input', { bubbles: true }));
      await wait(1000);`);
    await shot(`app-find-${theme}.png`);
    await evaluate(`key('Escape'); await wait(300);`);
    // Papelera: borrar una carpeta deja un aviso con "Deshacer"; la papelera lista lo borrado.
    await evaluate(`await menuOn(row('02 - Controles negativos'), 'Eliminar');
      [...document.querySelectorAll('.dialog')].at(-1).querySelector('button.primary').click(); await wait(900);`);
    await shot(`app-trash-undo-${theme}.png`);
    const login = byName((await api(`/collections/${collectionId}`)).items, 'OAuth 2.0 - login');
    await api(`/collections/${collectionId}/items/${login.id}`, undefined, 'DELETE');
    await evaluate(`await wait(500); document.querySelector('.side footer button').click(); await wait(800);`);
    await shot(`app-trash-${theme}.png`);
    // Todo vuelve a su sitio para el otro tema.
    for (const item of (await api('/trash')).items) await api(`/trash/${item.id}/restore`, undefined, 'POST');
    await evaluate(`key('Escape'); await wait(300);`);
  }
}

if (sprint === 's8') await s8();
else if (sprint === 's7') await s7();
else if (sprint === 'gif') await gif();
else if (sprint === 's6') await s6();
else if (sprint === 's5') await s5();
else if (sprint === 's4') await s4();
else await s3();

ws.close();
edge.kill();
await sleep(500);
try { rmSync(profile, { recursive: true, force: true }); } catch { /* Edge puede tardar en soltar el perfil */ }
process.exit(0);
