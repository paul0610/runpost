// Importar de Postman: diálogo nativo (Tauri) o selector del navegador; colección y entorno a la vez (R1).
import { pickFiles } from './native';
import { store } from './stores/app.svelte';

export async function importFromPostman(): Promise<void> {
  const picked = await pickFiles([{ name: 'Postman (JSON)', extensions: ['json'] }], true);
  await store.importPicked(picked);
}
