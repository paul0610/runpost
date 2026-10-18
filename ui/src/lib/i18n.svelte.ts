// Español e inglés (16.4 y UX decisión 4): la primera vez sigue al sistema; después, settings.ui.lang.
import es from '../i18n/es.json';
import en from '../i18n/en.json';

export type Lang = 'es' | 'en';
type Dictionary = { [key: string]: string | Dictionary };

const dictionaries: Record<Lang, Dictionary> = { es, en };
export const LANG_FLAG = 'runpost.langInitialized';

export const locale = $state<{ lang: Lang }>({ lang: 'es' });

export function isLang(value: unknown): value is Lang {
  return value === 'es' || value === 'en';
}

/** Idioma del sistema; fallback español. */
export function detectLang(languages: readonly string[] = navigator.languages ?? [navigator.language]): Lang {
  for (const language of languages) {
    const code = language?.toLowerCase().slice(0, 2);
    if (isLang(code)) return code;
  }
  return 'es';
}

function lookup(dictionary: Dictionary, key: string): string | undefined {
  let node: string | Dictionary | undefined = dictionary;
  for (const part of key.split('.')) {
    if (typeof node !== 'object' || node === null) return undefined;
    node = node[part];
  }
  return typeof node === 'string' ? node : undefined;
}

/** `t('import.done', { name, count })` con `{name}` interpolado. Reactivo a `locale.lang`. */
export function t(key: string, params: Record<string, string | number> = {}): string {
  const text = lookup(dictionaries[locale.lang], key) ?? lookup(es, key) ?? key;
  return text.replace(/\{(\w+)\}/g, (match, name: string) => (name in params ? String(params[name]) : match));
}

/** Todas las claves hoja, para comprobar que los dos idiomas están completos. */
export function keysOf(dictionary: Dictionary = es, prefix = ''): string[] {
  return Object.entries(dictionary).flatMap(([key, value]) =>
    typeof value === 'string' ? [`${prefix}${key}`] : keysOf(value, `${prefix}${key}.`),
  );
}

export const allDictionaries = dictionaries;
