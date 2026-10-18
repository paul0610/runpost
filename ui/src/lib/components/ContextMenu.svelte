<script lang="ts">
  // Menú contextual flotante; los elementos no disponibles se muestran deshabilitados (UX P1).
  // Teclado (S4): abre con el foco en la primera opción, ↑ ↓ Inicio Fin, Enter, Esc devuelve el foco.
  import { store, type MenuItem } from '../stores/app.svelte';

  const menu = $derived(store.menu);
  let box = $state<HTMLDivElement>();
  let position = $state({ left: 0, top: 0 });
  let opener: HTMLElement | null = null;

  $effect(() => {
    const current = store.menu;
    if (!current) return;
    opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    position = { left: current.x, top: current.y };
    // Que no se salga de la ventana.
    requestAnimationFrame(() => {
      if (!box) return;
      const rect = box.getBoundingClientRect();
      position = {
        left: Math.max(4, Math.min(current.x, window.innerWidth - rect.width - 4)),
        top: Math.max(4, Math.min(current.y, window.innerHeight - rect.height - 4)),
      };
      box.querySelector<HTMLElement>('button:not(:disabled)')?.focus();
    });
  });

  function run(item: MenuItem): void {
    if (item.disabled) return;
    close();
    void item.action?.();
  }

  function close(): void {
    if (!store.menu) return;
    store.menu = null;
    opener?.focus();
  }

  function key(event: KeyboardEvent): void {
    const items = [...(box?.querySelectorAll<HTMLElement>('button:not(:disabled)') ?? [])];
    const index = items.indexOf(document.activeElement as HTMLElement);
    const go = (i: number) => items[(i + items.length) % items.length]?.focus();
    switch (event.key) {
      case 'ArrowDown':
        go(index + 1);
        break;
      case 'ArrowUp':
        go(index < 0 ? items.length - 1 : index - 1);
        break;
      case 'Home':
        go(0);
        break;
      case 'End':
        go(items.length - 1);
        break;
      case 'Tab':
        close();
        break;
      default:
        return;
    }
    event.preventDefault();
  }
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && close()} onblur={close} />

{#if menu}
  <div class="backdrop" role="presentation" onclick={close} oncontextmenu={(e) => (e.preventDefault(), close())}></div>
  <div class="menu" role="menu" tabindex="-1" bind:this={box} style:left="{position.left}px" style:top="{position.top}px" onkeydown={key}>
    {#each menu.items as item, index (index)}
      {#if item.separator}
        <div class="sep"></div>
      {:else}
        <button role="menuitem" class:danger={item.danger} disabled={item.disabled} title={item.hint ?? ''} onclick={() => run(item)}>
          {item.label}
          {#if item.disabled && item.hint}<span class="hint">{item.hint}</span>{/if}
        </button>
      {/if}
    {/each}
  </div>
{/if}

<style>
  .backdrop { position: fixed; inset: 0; z-index: 80; }
  .menu { position: fixed; z-index: 81; min-width: 220px; background: var(--bg); border: 1px solid var(--border); border-radius: 8px; box-shadow: var(--shadow); padding: 4px; display: flex; flex-direction: column; }
  button { border: none; text-align: left; padding: 6px 10px; display: flex; gap: 10px; align-items: center; }
  button:focus-visible { outline: 2px solid var(--focus); outline-offset: -2px; }
  .danger { color: var(--err); }
  .hint { margin-left: auto; font-size: 10px; color: var(--muted); }
  .sep { height: 1px; background: var(--border); margin: 4px 2px; }
</style>
