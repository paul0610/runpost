<script lang="ts">
  // Petición arriba, respuesta abajo, separador arrastrable entre ambas (P1); el panel Código, a la
  // derecha de las dos cuando está abierto (como en Postman).
  import { store, type RequestTab } from '../stores/app.svelte';
  import RequestEditor from './RequestEditor.svelte';
  import ResponseViewer from './ResponseViewer.svelte';
  import CodeSnippet from './CodeSnippet.svelte';

  let { tab }: { tab: RequestTab } = $props();
  let container: HTMLDivElement;

  function startDrag(event: PointerEvent): void {
    event.preventDefault();
    const handle = event.currentTarget as HTMLElement;
    handle.setPointerCapture(event.pointerId);
    const top = container.getBoundingClientRect().top;
    const height = container.getBoundingClientRect().height;
    const move = (e: PointerEvent) => {
      store.ui.splitPx = Math.round(Math.min(Math.max(e.clientY - top, 120), height - 120));
    };
    const up = () => {
      handle.removeEventListener('pointermove', move);
      handle.removeEventListener('pointerup', up);
    };
    handle.addEventListener('pointermove', move);
    handle.addEventListener('pointerup', up);
  }

  function keyResize(event: KeyboardEvent): void {
    if (event.key === 'ArrowUp') store.ui.splitPx = Math.max(120, store.ui.splitPx - 20);
    if (event.key === 'ArrowDown') store.ui.splitPx += 20;
  }
</script>

<div class="bench">
  <div class="work" bind:this={container}>
    <div class="req" style:height="{store.ui.splitPx}px">
      <RequestEditor {tab} />
    </div>
    <!-- Patrón ARIA "window splitter": separador enfocable que se mueve con ↑/↓. -->
    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
    <div
      class="split"
      role="separator"
      aria-orientation="horizontal"
      aria-valuenow={store.ui.splitPx}
      aria-valuemin={120}
      aria-label="⋯"
      tabindex="0"
      onpointerdown={startDrag}
      onkeydown={keyResize}
    >⋯</div>
    <div class="resp">
      <ResponseViewer {tab} />
    </div>
  </div>
  {#if store.ui.code}<CodeSnippet {tab} />{/if}
</div>

<style>
  .bench { flex: 1; display: flex; min-height: 0; min-width: 0; }
  .work { flex: 1; display: flex; flex-direction: column; min-height: 0; min-width: 0; }
  .req { flex: none; min-height: 120px; display: flex; flex-direction: column; overflow: hidden; }
  .split { height: 7px; flex: none; background: var(--bg-2); border-top: 1px solid var(--border); border-bottom: 1px solid var(--border); cursor: row-resize; display: grid; place-items: center; color: var(--muted); font-size: 9px; line-height: 1; user-select: none; touch-action: none; }
  .split:focus-visible { outline: 2px solid var(--focus); outline-offset: -2px; }
  .resp { flex: 1; min-height: 0; display: flex; flex-direction: column; }
</style>
