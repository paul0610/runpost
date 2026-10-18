// "Copiar como curl" (S4, adelanto del generador de snippets de la fase 4). Sintaxis bash, como Postman.
// Las variables conocidas se sustituyen; los secretos, las dinámicas y las desconocidas quedan como
// {{nombre}}: el texto puede acabar en un chat o un ticket y nunca lleva un secreto.
import type { Auth, Body, KeyValue, RawLang } from './types';
import { composeUrl, isEnabled } from './url';
import type { VarLookup } from './vars';

export interface CurlRequest {
  method: string;
  url: string;
  params: KeyValue[];
  headers: KeyValue[];
  body: Body;
}

const CONTENT_TYPE: Record<RawLang, string> = {
  json: 'application/json',
  text: 'text/plain',
  xml: 'application/xml',
  html: 'text/html',
  javascript: 'application/javascript',
};

/** Comillas simples de shell: `it's` → `'it'\''s'`. */
export function shellQuote(text: string): string {
  return `'${text.replace(/'/g, "'\\''")}'`;
}

/** Sustituye solo las variables que no son secretas ni dinámicas. */
export function resolveSafe(text: string, lookup: VarLookup): string {
  return text.replace(/\{\{\s*([^{}]+?)\s*\}\}/g, (match, name: string) => {
    const info = lookup(name);
    return info && !info.secret && info.scope !== 'dynamic' ? info.value : match;
  });
}

export function toCurl(request: CurlRequest, auth: Auth, lookup: VarLookup): string {
  const r = (text: string) => resolveSafe(text, lookup);
  const method = (request.method || 'GET').toUpperCase();
  const params = request.params.filter((p) => isEnabled(p) && p.key !== '');
  if (auth.type === 'apikey' && auth.in === 'query' && auth.key) params.push({ key: auth.key, value: auth.value });
  const url = r(composeUrl(request.url, params));
  const headers: [string, string][] = request.headers
    .filter((h) => isEnabled(h) && h.key.trim() !== '')
    .map((h) => [r(h.key.trim()), r(h.value)]);
  const has = (name: string) => headers.some(([k]) => k.toLowerCase() === name.toLowerCase());
  const setHeader = (name: string, value: string) => {
    const at = headers.findIndex(([k]) => k.toLowerCase() === name.toLowerCase());
    if (at >= 0) headers.splice(at, 1);
    headers.push([name, value]);
  };
  const extra: string[] = [];

  if (auth.type === 'bearer' && auth.token) setHeader('Authorization', `Bearer ${r(auth.token)}`);
  if (auth.type === 'apikey' && auth.in === 'header' && auth.key) setHeader(r(auth.key), r(auth.value));
  if (auth.type === 'basic') extra.push(`--user ${shellQuote(`${r(auth.username)}:${r(auth.password)}`)}`);

  const body = request.body;
  const data: string[] = [];
  if (body.mode === 'raw') {
    if (!has('content-type')) headers.push(['Content-Type', CONTENT_TYPE[body.lang]]);
    data.push(`--data-raw ${shellQuote(r(body.text))}`);
  } else if (body.mode === 'urlencoded') {
    for (const field of body.fields.filter((f) => isEnabled(f) && f.key !== '')) {
      data.push(`--data-urlencode ${shellQuote(`${r(field.key)}=${r(field.value)}`)}`);
    }
  } else if (body.mode === 'multipart') {
    for (const field of body.fields.filter((f) => f.enabled !== false && f.key !== '')) {
      const value = field.kind === 'file' ? `@"${r(field.path ?? '')}"` : `"${r(field.value).replace(/"/g, '\\"')}"`;
      data.push(`--form ${shellQuote(`${r(field.key)}=${value}`)}`);
    }
  } else if (body.mode === 'binary') {
    data.push(`--data-binary ${shellQuote(`@${r(body.path)}`)}`);
  } else if (body.mode === 'graphql') {
    if (!has('content-type')) headers.push(['Content-Type', 'application/json']);
    let variables: unknown = null;
    try {
      variables = body.variables.trim() ? JSON.parse(r(body.variables)) : null;
    } catch {
      variables = r(body.variables);
    }
    data.push(`--data-raw ${shellQuote(JSON.stringify({ query: r(body.query), variables }))}`);
  }

  const first = method === 'GET' && !data.length ? `curl --location ${shellQuote(url)}` : `curl --location --request ${method} ${shellQuote(url)}`;
  const lines = [first, ...headers.map(([k, v]) => `--header ${shellQuote(`${k}: ${v}`)}`), ...extra, ...data];
  return lines.join(' \\\n');
}
