// Importar (fase 4, como Postman): el diálogo Importar acepta texto pegado (cURL o el contenido de una
// colección) y archivos Postman, OpenAPI, HAR, Insomnia o Bruno. Diálogo nativo en Tauri, selector del
// navegador en modo navegador; colección y entorno a la vez (R1).
import { pickFiles } from './native';
import { store } from './stores/app.svelte';

export function openImport(): void {
  store.ui.importOpen = true;
}

export async function importFiles(): Promise<void> {
  const picked = await pickFiles([{ name: 'Postman, OpenAPI, HAR, Insomnia, Bruno', extensions: ['json', 'yaml', 'yml', 'har'] }], true);
  if (picked) store.ui.importOpen = false;
  await store.importPicked(picked);
}
