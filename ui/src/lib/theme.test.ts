import { beforeEach, describe, expect, it, vi } from 'vitest';

const setWindowTheme = vi.fn(async (_theme: 'light' | 'dark' | null) => undefined);
vi.mock('./native', () => ({ setWindowTheme: (theme: 'light' | 'dark' | null) => setWindowTheme(theme) }));

import { THEME_ICON, THEME_KEY, applyTheme, nextTheme, resolveTheme, watchSystemTheme } from './theme';

function mockSystem(dark: boolean) {
  const listeners: (() => void)[] = [];
  const query = {
    matches: dark,
    addEventListener: (_: string, fn: () => void) => listeners.push(fn),
    removeEventListener: vi.fn(),
  };
  window.matchMedia = vi.fn(() => query) as unknown as typeof window.matchMedia;
  return { query, fire: () => listeners.forEach((fn) => fn()) };
}

describe('ciclo de tema (9.4)', () => {
  beforeEach(() => {
    setWindowTheme.mockClear();
    localStorage.clear();
  });

  it('claro → oscuro → sistema → claro', () => {
    expect(nextTheme('light')).toBe('dark');
    expect(nextTheme('dark')).toBe('system');
    expect(nextTheme('system')).toBe('light');
    expect(THEME_ICON).toEqual({ light: '☀', dark: '🌙', system: '🖥' });
  });

  it('el modo sistema sigue al SO', () => {
    expect(resolveTheme('system', true)).toBe('dark');
    expect(resolveTheme('system', false)).toBe('light');
    expect(resolveTheme('light', true)).toBe('light');
  });

  it('aplica data-theme, el espejo en localStorage y la barra de título nativa', () => {
    mockSystem(true);
    expect(applyTheme('light')).toBe('light');
    expect(document.documentElement.dataset.theme).toBe('light');
    expect(localStorage.getItem(THEME_KEY)).toBe('light');
    expect(setWindowTheme).toHaveBeenLastCalledWith('light');
    expect(applyTheme('system')).toBe('dark');
    expect(setWindowTheme).toHaveBeenLastCalledWith(null);
  });

  it('en modo sistema reacciona al cambio del SO; en modo fijo no', () => {
    const system = mockSystem(false);
    let mode: 'light' | 'dark' | 'system' = 'system';
    const changed = vi.fn();
    const stop = watchSystemTheme(() => mode, changed);
    system.query.matches = true;
    system.fire();
    expect(document.documentElement.dataset.theme).toBe('dark');
    expect(changed).toHaveBeenCalledTimes(1);
    mode = 'light';
    system.fire();
    expect(changed).toHaveBeenCalledTimes(1);
    stop();
    expect(system.query.removeEventListener).toHaveBeenCalled();
  });
});
