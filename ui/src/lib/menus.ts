// Menús contextuales del árbol, de entornos y del historial (docs/UX.md P1). Lo que aún no existe
// aparece deshabilitado, no oculto.
import { t } from './i18n.svelte';
import { store, type MenuItem } from './stores/app.svelte';
import type { Collection, Environment, HistorySummary, Item } from './types';

/** Dónde abrir el menú: en el ratón o, si se abrió con el teclado (Mayús+F10), bajo la fila. */
export function menuPoint(event: MouseEvent): { x: number; y: number } {
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
  const inside = event.clientX >= rect.left && event.clientX <= rect.right && event.clientY >= rect.top && event.clientY <= rect.bottom;
  return inside ? { x: event.clientX, y: event.clientY } : { x: rect.left + 24, y: rect.bottom };
}
const separator: MenuItem = { label: '', separator: true };

export function collectionMenu(collection: Collection): MenuItem[] {
  return [
    { label: t('menu.newRequest'), action: () => store.createRequest(collection.id, null) },
    { label: t('menu.newFolder'), action: () => store.createFolder(collection.id, null) },
    { label: t('menu.run'), action: () => store.openRunner(collection.id, null) },
    separator,
    { label: t('menu.docs'), action: () => store.openDocs(collection.id, null) },
    { label: t('menu.agent'), action: () => store.openPolicy(collection.id) },
    { label: t('menu.findReplace'), action: () => (store.ui.findIn = collection.id) },
    { label: t('menu.duplicate'), action: () => store.duplicateCollection(collection) },
    { label: t('menu.rename'), action: () => store.renameCollection(collection) },
    { label: t('menu.export'), action: () => store.exportCollection(collection.id) },
    { label: t('menu.copyId'), action: () => store.copyId(collection.id) },
    separator,
    { label: t('menu.delete'), danger: true, action: () => store.deleteCollection(collection) },
  ];
}

export function itemMenu(collectionId: string, item: Item): MenuItem[] {
  if (item.type === 'folder') {
    return [
      { label: t('menu.newRequest'), action: () => store.createRequest(collectionId, item.id) },
      { label: t('menu.newFolder'), action: () => store.createFolder(collectionId, item.id) },
      { label: t('menu.run'), action: () => store.openRunner(collectionId, item.id) },
      separator,
      { label: t('menu.docs'), action: () => store.openDocs(collectionId, item.id) },
      { label: t('menu.duplicate'), action: () => store.duplicateItem(collectionId, item) },
      { label: t('menu.rename'), action: () => store.renameItem(collectionId, item) },
      { label: t('menu.export'), action: () => store.exportCollection(collectionId, item.id) },
      { label: t('menu.copyId'), action: () => store.copyId(item.id) },
      separator,
      { label: t('menu.delete'), danger: true, action: () => store.deleteItem(collectionId, item) },
    ];
  }
  return [
    { label: t('menu.duplicate'), action: () => store.duplicateItem(collectionId, item) },
    { label: t('menu.rename'), action: () => store.renameItem(collectionId, item) },
    { label: t('menu.copyCurl'), action: () => store.copyCurl(collectionId, item) },
    { label: t('menu.copyId'), action: () => store.copyId(item.id) },
    separator,
    { label: t('menu.delete'), danger: true, action: () => store.deleteItem(collectionId, item) },
  ];
}

export function environmentMenu(env: Environment): MenuItem[] {
  const active = store.activeEnvironment?.id === env.id;
  return [
    { label: t('menu.open'), action: () => store.openEnvironment(env.id) },
    { label: active ? t('env.deactivate') : t('side.activate'), action: () => store.setActiveEnvironment(active ? null : env.id) },
    separator,
    { label: t('menu.duplicate'), action: () => store.duplicateEnvironment(env) },
    { label: t('menu.rename'), action: () => store.renameEnvironment(env) },
    { label: t('menu.export'), action: () => store.exportEnvironment(env) },
    { label: t('menu.copyId'), action: () => store.copyId(env.id) },
    separator,
    { label: t('menu.delete'), danger: true, action: () => store.deleteEnvironment(env) },
  ];
}

export function historyMenu(row: HistorySummary): MenuItem[] {
  return [
    { label: t('history.open'), action: () => void store.openHistory(row.id) },
    { label: t('history.saveToCollection'), action: () => store.saveHistoryToCollection(row.id) },
  ];
}
