import { describe, expect, it } from 'vitest';
import { escapeHtml, renderMarkdown } from './markdown';

describe('markdown de la pestaña Docs (S4)', () => {
  it('títulos, negrita, cursiva, código en línea y párrafos', () => {
    expect(renderMarkdown('# Título\n\nTexto **fuerte** y *suave* con `a*b*c`.\nSegunda línea')).toBe(
      '<h1>Título</h1>\n<p>Texto <strong>fuerte</strong> y <em>suave</em> con <code>a*b*c</code>.<br>Segunda línea</p>',
    );
  });

  it('listas, citas y bloques de código (sin formato dentro)', () => {
    expect(renderMarkdown('- uno\n- dos\n1. a\n2. b\n> cita\n```\n<b>x</b>\n**y**\n```')).toBe(
      '<ul><li>uno</li><li>dos</li></ul>\n<ol><li>a</li><li>b</li></ol>\n<blockquote>cita</blockquote>\n<pre><code>&lt;b&gt;x&lt;/b&gt;\n**y**</code></pre>',
    );
  });

  it('solo enlaza http(s) y mailto, en pestaña nueva y con rel seguro', () => {
    expect(renderMarkdown('[docs](https://x.test/a?b=1&c=2) [m](mailto:a@b.c)')).toBe(
      '<p><a href="https://x.test/a?b=1&amp;c=2" target="_blank" rel="noopener noreferrer">docs</a> ' +
        '<a href="mailto:a@b.c" target="_blank" rel="noopener noreferrer">m</a></p>',
    );
    expect(renderMarkdown('[x](javascript:alert(1))')).not.toContain('<a');
    expect(renderMarkdown('[x](JavaScript:alert(1))')).not.toContain('<a');
    expect(renderMarkdown('[x](data:text/html,hola)')).not.toContain('<a');
  });

  it('escapa todo el HTML del texto (XSS)', () => {
    const html = renderMarkdown(
      [
        '<script>alert(1)</script>',
        '<img src=x onerror="alert(1)">',
        '[a](https://x.test/"onmouseover="alert(1))',
        '**<svg onload=alert(1)>**',
        '# <iframe src="javascript:alert(1)">',
      ].join('\n'),
    );
    expect(html).toContain('&lt;script&gt;');
    const doc = new DOMParser().parseFromString(html, 'text/html');
    expect(doc.querySelectorAll('script, img, svg, iframe')).toHaveLength(0);
    for (const element of doc.body.querySelectorAll('*')) {
      for (const attribute of element.attributes) expect(attribute.name.startsWith('on'), `${element.tagName} ${attribute.name}`).toBe(false);
    }
    for (const link of doc.querySelectorAll('a')) expect(link.getAttribute('href')).toMatch(/^https:\/\/x\.test\//);
  });

  it('el texto no puede falsear los marcadores internos del código en línea', () => {
    const nul = String.fromCharCode(0);
    expect(renderMarkdown(`a ${nul}0${nul} b`)).toBe('<p>a 0 b</p>');
  });

  it('escapeHtml', () => {
    expect(escapeHtml(`<a href="x">'&'</a>`)).toBe('&lt;a href=&quot;x&quot;&gt;&#39;&amp;&#39;&lt;/a&gt;');
  });
});
