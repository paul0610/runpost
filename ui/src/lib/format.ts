// Formatos de presentación: tamaños, métodos, estados y JSON con variables.

/** `1234` → `"1,2 KB"` en español, `"1.2 KB"` en inglés. */
export function formatBytes(bytes: number, lang = 'es'): string {
  const locale = lang === 'en' ? 'en-US' : 'es-ES';
  if (bytes < 1024) return `${bytes} B`;
  const units = ['KB', 'MB', 'GB'];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toLocaleString(locale, { maximumFractionDigits: 1 })} ${units[unit]}`;
}

/** Etiqueta corta del árbol (como Postman): DELETE → DEL, OPTIONS → OPT. */
export function methodLabel(method: string): string {
  const upper = method.toUpperCase();
  return upper === 'DELETE' ? 'DEL' : upper === 'OPTIONS' ? 'OPT' : upper;
}

export function methodClass(method: string): string {
  const upper = method.toUpperCase();
  return ['GET', 'POST', 'PUT', 'DELETE', 'PATCH'].includes(upper) ? `m-${upper}` : 'm-OTHER';
}

/** Color del código de estado: 2xx verde, 3xx azul, 4xx/5xx rojo. */
export function statusClass(status: number): 'ok' | 'info' | 'err' {
  return status < 300 ? 'ok' : status < 400 ? 'info' : 'err';
}

const VAR = /\{\{\s*[^{}]+?\s*\}\}/g;

/**
 * "Beautify" de JSON (sección 18) que tolera {{variables}} fuera de comillas, como
 * `"draft": {{draftJson}}`. Devuelve null si no es JSON ni con ese apaño.
 */
export function beautifyJson(text: string): string | null {
  try {
    return JSON.stringify(JSON.parse(text), null, 2);
  } catch {
    /* puede llevar variables sin comillas */
  }
  const placeholders: string[] = [];
  let inString = false;
  let escaped = false;
  let out = '';
  for (let i = 0; i < text.length; i += 1) {
    const ch = text[i];
    if (inString) {
      out += ch;
      if (escaped) escaped = false;
      else if (ch === '\\') escaped = true;
      else if (ch === '"') inString = false;
      continue;
    }
    if (ch === '"') {
      inString = true;
      out += ch;
      continue;
    }
    VAR.lastIndex = i;
    const match = text.startsWith('{{', i) ? VAR.exec(text) : null;
    if (match && match.index === i) {
      out += `"__runpost_var_${placeholders.length}__"`;
      placeholders.push(match[0]);
      i += match[0].length - 1;
      continue;
    }
    out += ch;
  }
  try {
    let pretty = JSON.stringify(JSON.parse(out), null, 2);
    placeholders.forEach((original, index) => {
      pretty = pretty.replace(`"__runpost_var_${index}__"`, original);
    });
    return pretty;
  } catch {
    return null;
  }
}

/** Texto bonito de un cuerpo de respuesta según su Content-Type. */
export function prettyBody(body: string, contentType: string): { text: string; lang: 'json' | 'xml' | 'html' | 'javascript' | 'text' } {
  const ct = contentType.toLowerCase();
  if (ct.includes('json')) {
    try {
      return { text: JSON.stringify(JSON.parse(body), null, 2), lang: 'json' };
    } catch {
      return { text: body, lang: 'text' };
    }
  }
  if (ct.includes('html')) return { text: body, lang: 'html' };
  if (ct.includes('xml')) return { text: body, lang: 'xml' };
  if (ct.includes('javascript')) return { text: body, lang: 'javascript' };
  const trimmed = body.trimStart();
  if (trimmed.startsWith('{') || trimmed.startsWith('[')) {
    try {
      return { text: JSON.stringify(JSON.parse(body), null, 2), lang: 'json' };
    } catch {
      /* no es JSON */
    }
  }
  return { text: body, lang: 'text' };
}

/** Extensión sugerida al guardar una respuesta. */
export function extensionFor(contentType: string): string {
  const ct = contentType.toLowerCase();
  if (ct.includes('json')) return 'json';
  if (ct.includes('html')) return 'html';
  if (ct.includes('xml')) return 'xml';
  if (ct.includes('javascript')) return 'js';
  return 'txt';
}
