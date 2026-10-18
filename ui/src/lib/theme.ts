// Tema claro / oscuro / sistema (sección 9.4). La fuente de verdad es settings.theme;
// localStorage es solo un espejo para el script anti-destello (public/theme-boot.js).
import type { ThemeMode } from './types';
import { setWindowTheme } from './native';

export const THEME_KEY = 'runpost.theme';
export const THEME_ICON: Record<ThemeMode, string> = { light: '☀', dark: '🌙', system: '🖥' };

/** Ciclo del botón de la barra superior y de Ctrl+Shift+L. */
export function nextTheme(mode: ThemeMode): ThemeMode {
  return mode === 'light' ? 'dark' : mode === 'dark' ? 'system' : 'light';
}

export function resolveTheme(mode: ThemeMode, systemDark: boolean): 'light' | 'dark' {
  return mode === 'system' ? (systemDark ? 'dark' : 'light') : mode;
}

const darkQuery = (): MediaQueryList | undefined =>
  typeof window.matchMedia === 'function' ? window.matchMedia('(prefers-color-scheme: dark)') : undefined;

export function systemIsDark(): boolean {
  return darkQuery()?.matches ?? true;
}

/** Aplica el tema al documento, al espejo y a la ventana nativa. */
export function applyTheme(mode: ThemeMode): 'light' | 'dark' {
  const effective = resolveTheme(mode, systemIsDark());
  document.documentElement.dataset.theme = effective;
  try {
    localStorage.setItem(THEME_KEY, mode);
  } catch {
    /* almacenamiento no disponible */
  }
  void setWindowTheme(mode === 'system' ? null : mode).catch(() => undefined);
  return effective;
}

/** En modo sistema, sigue los cambios del SO en caliente. Devuelve la función para dejar de escuchar. */
export function watchSystemTheme(getMode: () => ThemeMode, onChange: () => void): () => void {
  const query = darkQuery();
  if (!query) return () => undefined;
  const handler = () => {
    if (getMode() === 'system') {
      applyTheme('system');
      onChange();
    }
  };
  query.addEventListener('change', handler);
  return () => query.removeEventListener('change', handler);
}
