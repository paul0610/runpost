import { describe, expect, it } from 'vitest';
import { resolveSafe, shellQuote, toCurl, type CurlRequest } from './curl';
import { lookupFrom } from './vars';

const lookup = lookupFrom([
  {
    scope: 'environment',
    vars: [
      { key: 'base', value: 'https://api.test' },
      { key: 'token', value: 's3cr3t-T0ken', secret: true },
      { key: 'user', value: 'ana' },
      { key: 'off', value: 'x', enabled: false },
    ],
  },
]);

const URL_LINE = "curl --location --request POST 'https://api.test/v2/ping'";
const lines = (...parts: string[]) => parts.join(' \\\n');

function req(overrides: Partial<CurlRequest> = {}): CurlRequest {
  return { method: 'GET', url: '{{base}}/v2/ping', params: [], headers: [], body: { mode: 'none' }, ...overrides };
}

describe('copiar como curl (S4)', () => {
  it('comillas simples de shell', () => {
    expect(shellQuote("it's")).toBe("'it'\\''s'");
  });

  it('sustituye las variables normales; secretas, dinámicas, desconocidas y apagadas quedan como {{x}}', () => {
    expect(resolveSafe('{{base}}/{{token}}/{{$guid}}/{{nope}}/{{off}}', lookup)).toBe('https://api.test/{{token}}/{{$guid}}/{{nope}}/{{off}}');
  });

  it('GET con params y cabeceras activas, bearer secreto sin su valor', () => {
    const text = toCurl(
      req({
        params: [
          { key: 'a', value: '1' },
          { key: 'b', value: '2', enabled: false },
        ],
        headers: [
          { key: 'Accept', value: 'application/json' },
          { key: 'X-Off', value: '1', enabled: false },
        ],
      }),
      { type: 'bearer', token: '{{token}}' },
      lookup,
    );
    expect(text).toBe(
      lines("curl --location 'https://api.test/v2/ping?a=1'", "--header 'Accept: application/json'", "--header 'Authorization: Bearer {{token}}'"),
    );
    expect(text).not.toContain('s3cr3t');
  });

  it('POST raw JSON añade Content-Type y --data-raw con comillas escapadas', () => {
    const text = toCurl(req({ method: 'post', body: { mode: 'raw', lang: 'json', text: `{"user":"{{user}}","note":"it's"}` } }), { type: 'none' }, lookup);
    expect(text).toBe(lines(URL_LINE, "--header 'Content-Type: application/json'", `--data-raw '{"user":"ana","note":"it'\\''s"}'`));
  });

  it('basic con --user, API key en la query y en cabecera (reemplaza la existente)', () => {
    expect(toCurl(req(), { type: 'basic', username: '{{user}}', password: '{{token}}' }, lookup)).toBe(
      lines("curl --location 'https://api.test/v2/ping'", "--user 'ana:{{token}}'"),
    );
    expect(toCurl(req(), { type: 'apikey', key: 'api_key', value: '{{token}}', in: 'query' }, lookup)).toBe(
      "curl --location 'https://api.test/v2/ping?api_key={{token}}'",
    );
    expect(toCurl(req({ headers: [{ key: 'x-api-key', value: 'old' }] }), { type: 'apikey', key: 'X-API-Key', value: 'k', in: 'header' }, lookup)).toBe(
      lines("curl --location 'https://api.test/v2/ping'", "--header 'X-API-Key: k'"),
    );
  });

  it('urlencoded, multipart, binario y GraphQL', () => {
    const none = { type: 'none' } as const;
    expect(
      toCurl(req({ method: 'POST', body: { mode: 'urlencoded', fields: [{ key: 'a', value: '{{user}}' }, { key: 'b', value: '', enabled: false }] } }), none, lookup),
    ).toBe(lines(URL_LINE, "--data-urlencode 'a=ana'"));
    expect(
      toCurl(
        req({
          method: 'POST',
          body: {
            mode: 'multipart',
            fields: [
              { key: 'f', value: '', kind: 'file', path: 'C:/tmp/a.txt' },
              { key: 'n', value: 'say "hi"', kind: 'text' },
            ],
          },
        }),
        none,
        lookup,
      ),
    ).toBe(lines(URL_LINE, `--form 'f=@"C:/tmp/a.txt"'`, `--form 'n="say \\"hi\\""'`));
    expect(toCurl(req({ method: 'POST', body: { mode: 'binary', path: 'C:/tmp/b.bin' } }), none, lookup)).toBe(
      lines(URL_LINE, "--data-binary '@C:/tmp/b.bin'"),
    );
    expect(toCurl(req({ method: 'POST', body: { mode: 'graphql', query: '{ me { id } }', variables: '{"u":"{{user}}"}' } }), none, lookup)).toBe(
      lines(URL_LINE, "--header 'Content-Type: application/json'", `--data-raw '{"query":"{ me { id } }","variables":{"u":"ana"}}'`),
    );
  });
});
