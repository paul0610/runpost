// Lo nativo (sección 7): diálogos, portapapeles, tema de ventana y carpeta del workspace.
// En modo navegador cae a <input type=file>, navigator.clipboard y no-op.
import { invoke } from '@tauri-apps/api/core';
import { isTauri } from './transport';

export interface FileFilter {
  name: string;
  extensions: string[];
}

/** En Tauri, rutas del disco; en el navegador, objetos File (sin ruta). */
export type PickedFiles = { kind: 'paths'; paths: string[] } | { kind: 'files'; files: File[] };

function browserPick(filters: FileFilter[], multiple: boolean): Promise<PickedFiles | null> {
  return new Promise((resolve) => {
    const input = document.createElement('input');
    input.type = 'file';
    input.multiple = multiple;
    input.accept = filters.flatMap((f) => f.extensions.map((e) => `.${e}`)).join(',');
    input.onchange = () => resolve(input.files?.length ? { kind: 'files', files: [...input.files] } : null);
    input.oncancel = () => resolve(null);
    input.click();
  });
}

export async function pickFiles(filters: FileFilter[] = [], multiple = false): Promise<PickedFiles | null> {
  if (!isTauri()) return browserPick(filters, multiple);
  const paths = await invoke<string[] | null>('pick_file', { filters, multiple });
  return paths?.length ? { kind: 'paths', paths } : null;
}

/** Ruta absoluta elegida en el diálogo de guardar, o null (cancelado o modo navegador). */
export async function pickSavePath(defaultName: string, filters: FileFilter[] = []): Promise<string | null> {
  if (!isTauri()) return null;
  return invoke<string | null>('pick_save_path', { defaultName, filters });
}

/** Abre un enlace en el navegador del sistema. En Tauri, con el plugin opener y solo las URLs que
 * permite `capabilities/default.json` (el repositorio del proyecto). */
export async function openExternal(url: string): Promise<void> {
  if (isTauri()) {
    await invoke('plugin:opener|open_url', { url });
    return;
  }
  window.open(url, '_blank', 'noopener,noreferrer');
}

/** Descarga en el navegador (modo respaldo) cuando no hay diálogo nativo. */
export function downloadText(name: string, text: string, type = 'application/octet-stream'): void {
  const url = URL.createObjectURL(new Blob([text], { type }));
  const link = document.createElement('a');
  link.href = url;
  link.download = name;
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

/** Copia con el método antiguo (textarea + execCommand); algunos webviews no conceden clipboard-write. */
function legacyCopy(text: string): boolean {
  const area = document.createElement('textarea');
  area.value = text;
  area.setAttribute('readonly', '');
  area.style.position = 'fixed';
  area.style.opacity = '0';
  document.body.appendChild(area);
  area.select();
  let ok = false;
  try {
    ok = document.execCommand('copy');
  } catch {
    ok = false;
  }
  area.remove();
  return ok;
}

export async function copyText(text: string): Promise<void> {
  if (isTauri()) {
    // Comando del plugin clipboard-manager (permiso clipboard-manager:allow-write-text).
    await invoke('plugin:clipboard-manager|write_text', { text });
    return;
  }
  try {
    await navigator.clipboard.writeText(text);
  } catch (error) {
    if (!legacyCopy(text)) throw error;
  }
}

/** Barra de título nativa: 'light' | 'dark', o null para seguir al sistema (9.4). */
export async function setWindowTheme(theme: 'light' | 'dark' | null): Promise<void> {
  if (!isTauri()) return;
  await invoke('set_window_theme', { theme });
}

export async function revealWorkspace(): Promise<boolean> {
  if (!isTauri()) return false;
  await invoke('reveal_workspace');
  return true;
}
