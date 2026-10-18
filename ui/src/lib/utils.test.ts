import { describe, expect, it } from 'vitest';
import { applyUrlText, composeUrl, normalizeUrl, parseQuery, splitUrl } from './url';
import { beautifyJson, extensionFor, formatBytes, methodClass, methodLabel, prettyBody, statusClass } from './format';
import { fromBulk, toBulk } from './kv';
import { lookupFrom, segments, variableNames } from './vars';
import { allDictionaries, detectLang, keysOf, locale, t } from './i18n.svelte';

describe('URL ↔ Params (sección 18)', () => {
  it('separa base, query y #', () => {
    expect(splitUrl('{{base}}/a?x=1&y#top')).toEqual({ base: '{{base}}/a#top', query: 'x=1&y' });
    expect(splitUrl('http://h/a')).toEqual({ base: 'http://h/a', query: null });
    expect(parseQuery('x=1&y&=v&&a=b=c')).toEqual([
      { key: 'x', value: '1' },
      { key: 'y', value: '' },
      { key: '', value: 'v' },
      { key: 'a', value: 'b=c' },
    ]);
  });

  it('compone solo las filas activas y conserva el #', () => {
    const params = [
      { key: 'a', value: '1' },
      { key: 'b', value: '2', enabled: false },
      { key: 'c', value: '' },
    ];
    expect(composeUrl('http://h/p', params)).toBe('http://h/p?a=1&c');
    expect(composeUrl('http://h/p#f', params)).toBe('http://h/p?a=1&c#f');
    expect(composeUrl('http://h/p', [])).toBe('http://h/p');
  });

  it('escribir en la URL respeta las filas desactivadas de la tabla', () => {
    const result = applyUrlText('http://h/p?x=9', [
      { key: 'old', value: '1' },
      { key: 'off', value: '2', enabled: false },
    ]);
    expect(result).toEqual({ url: 'http://h/p', params: [{ key: 'x', value: '9' }, { key: 'off', value: '2', enabled: false }] });
  });

  it('al abrir, la query embebida pasa a params', () => {
    expect(normalizeUrl('{{u}}/a?x=1', [{ key: 'y', value: '2' }])).toEqual({
      url: '{{u}}/a',
      params: [{ key: 'x', value: '1' }, { key: 'y', value: '2' }],
    });
  });
});

describe('formatos', () => {
  it('tamaños, métodos y estados', () => {
    expect(formatBytes(512)).toBe('512 B');
    expect(formatBytes(1229, 'es')).toBe('1,2 KB');
    expect(formatBytes(1229, 'en')).toBe('1.2 KB');
    expect(formatBytes(5 * 1024 * 1024)).toBe('5 MB');
    expect(methodLabel('delete')).toBe('DEL');
    expect(methodLabel('OPTIONS')).toBe('OPT');
    expect(methodClass('patch')).toBe('m-PATCH');
    expect(methodClass('HEAD')).toBe('m-OTHER');
    expect([statusClass(200), statusClass(302), statusClass(404), statusClass(500)]).toEqual(['ok', 'info', 'err', 'err']);
    expect(extensionFor('application/json; charset=utf-8')).toBe('json');
    expect(extensionFor('image/png')).toBe('txt');
  });

  it('beautify tolera {{variables}} sin comillas', () => {
    expect(beautifyJson('{"a":1}')).toBe('{\n  "a": 1\n}');
    expect(beautifyJson('{"tx": {{draftJson}}, "s": "{{x}}"}')).toBe('{\n  "tx": {{draftJson}},\n  "s": "{{x}}"\n}');
    expect(beautifyJson('{"a": "\\"{{"}')).toBe('{\n  "a": "\\"{{"\n}');
    expect(beautifyJson('not json')).toBeNull();
  });

  it('pretty por Content-Type', () => {
    expect(prettyBody('{"a":1}', 'application/json')).toEqual({ text: '{\n  "a": 1\n}', lang: 'json' });
    expect(prettyBody('[1]', 'text/plain').lang).toBe('json');
    expect(prettyBody('{bad', 'application/json')).toEqual({ text: '{bad', lang: 'text' });
    expect(prettyBody('<p/>', 'text/html').lang).toBe('html');
    expect(prettyBody('<a/>', 'application/xml').lang).toBe('xml');
    expect(prettyBody('x()', 'application/javascript').lang).toBe('javascript');
    expect(prettyBody('hola', '').lang).toBe('text');
  });
});

describe('bulk edit', () => {
  it('ida y vuelta con filas desactivadas', () => {
    const rows = [
      { key: 'Accept', value: 'application/json', enabled: true },
      { key: 'X-Off', value: 'a: b', enabled: false },
    ];
    const text = toBulk(rows);
    expect(text).toBe('Accept: application/json\n//X-Off: a: b');
    expect(fromBulk(`${text}\n\n  solo`)).toEqual([...rows, { key: 'solo', value: '', enabled: true }]);
  });
});

describe('variables (5.7)', () => {
  it('segmenta y nombra', () => {
    expect(segments('{{a}}/x{{ b }}').map((s) => s.name ?? s.text)).toEqual(['a', '/x', 'b']);
    expect(variableNames('{{a}}{{a}}{{$guid}}')).toEqual(['a', '$guid']);
  });

  it('entorno > colección > globales > dinámicas; deshabilitadas y sin valor no cuentan', () => {
    const lookup = lookupFrom([
      { scope: 'environment', vars: [{ key: 'x', value: 'env' }, { key: 'off', value: '1', enabled: false }, { key: 's', value: 'k', secret: true }] },
      { scope: 'collection', vars: [{ key: 'x', value: 'col' }, { key: 'c', value: 'C' }, { key: 'kr', secret: true }] },
      { scope: 'globals', vars: [{ key: 'off', value: 'g' }] },
    ]);
    expect(lookup('x')).toEqual({ value: 'env', secret: false, scope: 'environment' });
    expect(lookup('c')?.scope).toBe('collection');
    expect(lookup('off')).toEqual({ value: 'g', secret: false, scope: 'globals' });
    expect(lookup('s')?.secret).toBe(true);
    expect(lookup('kr')).toBeNull();
    expect(lookup('$timestamp')?.scope).toBe('dynamic');
    expect(lookup('nope')).toBeNull();
  });
});

describe('i18n', () => {
  it('es y en tienen exactamente las mismas claves', () => {
    expect(keysOf(allDictionaries.en).sort()).toEqual(keysOf(allDictionaries.es).sort());
  });

  it('interpola y cambia de idioma en vivo', () => {
    locale.lang = 'es';
    expect(t('import.done', { name: 'B', count: 10 })).toBe('Importada «B»: 10 peticiones');
    locale.lang = 'en';
    expect(t('import.done', { name: 'B', count: 10 })).toBe('Imported "B": 10 requests');
    expect(t('req.urlPlaceholder')).toContain('{{variable}}');
    expect(t('no.such.key')).toBe('no.such.key');
    locale.lang = 'es';
  });

  it('idioma del sistema con fallback español', () => {
    expect(detectLang(['en-US', 'es'])).toBe('en');
    expect(detectLang(['es-PE'])).toBe('es');
    expect(detectLang(['fr-FR', 'de'])).toBe('es');
  });
});
