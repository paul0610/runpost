import { describe, expect, it } from 'vitest';
import tokens from './tokens.css?raw';

// Revisión de accesibilidad S4: todo color de texto cumple AA (4,5:1) sobre los cuatro fondos del tema, el
// texto de los botones primarios también, y el anillo de foco destaca al menos 3:1 (WCAG 1.4.11).

/** Variables de un bloque `:root[data-theme="x"] { … }` de tokens.css. */
function theme(name: 'light' | 'dark'): Record<string, string> {
  const block = tokens.match(new RegExp(`:root\\[data-theme="${name}"\\]\\s*\\{([^}]*)\\}`))?.[1] ?? '';
  return Object.fromEntries([...block.matchAll(/--([\w-]+):\s*([^;]+);/g)].map((m) => [m[1], m[2].trim()]));
}

/** Luminancia relativa WCAG 2.x de un color `#rrggbb`. */
function luminance(hex: string): number {
  const [r, g, b] = [1, 3, 5]
    .map((i) => parseInt(hex.slice(i, i + 2), 16) / 255)
    .map((c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrast(a: string, b: string): number {
  const [high, low] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (high + 0.05) / (low + 0.05);
}

const BACKGROUNDS = ['bg', 'bg-2', 'bg-3', 'code-bg'];
const TEXT = ['fg', 'muted', 'ok', 'warn', 'err', 'info', 'patch', 'accent-text'];

describe.each(['light', 'dark'] as const)('contraste AA, tema %s', (name) => {
  const vars = theme(name);

  it('los tokens existen y son #rrggbb', () => {
    for (const key of [...BACKGROUNDS, ...TEXT, 'accent', 'on-accent', 'focus']) expect(vars[key], key).toMatch(/^#[0-9a-f]{6}$/i);
  });

  it.each(TEXT)('--%s sobre los cuatro fondos ≥ 4,5:1', (text) => {
    for (const bg of BACKGROUNDS) expect(contrast(vars[text], vars[bg]), `--${text} sobre --${bg}`).toBeGreaterThanOrEqual(4.5);
  });

  it('texto de los botones primarios sobre el acento ≥ 4,5:1', () => {
    expect(contrast(vars['on-accent'], vars.accent)).toBeGreaterThanOrEqual(4.5);
  });

  it('el anillo de foco destaca ≥ 3:1 sobre los fondos', () => {
    for (const bg of BACKGROUNDS) expect(contrast(vars.focus, vars[bg]), `--focus sobre --${bg}`).toBeGreaterThanOrEqual(3);
  });
});
