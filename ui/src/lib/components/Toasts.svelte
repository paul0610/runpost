<script lang="ts">
  // Avisos breves (importación, copiar, errores) arriba a la derecha y, encima, las peticiones del
  // agente que esperan aprobación (R4.3, decisión UX 3: solo dentro de la app, 60 s).
  import { store } from '../stores/app.svelte';
  import ApprovalCard from './ApprovalCard.svelte';
</script>

<div class="toasts" role="status" aria-live="polite">
  {#each store.agent.pending as approval (approval.id)}
    <ApprovalCard {approval} />
  {/each}
  {#each store.toasts as toast (toast.id)}
    <div class="toast {toast.kind}">{toast.text}</div>
  {/each}
</div>

<style>
  .toasts { position: fixed; top: 58px; right: 16px; display: flex; flex-direction: column; gap: 8px; z-index: 90; width: 380px; pointer-events: none; }
  .toast { background: var(--bg); border: 1px solid var(--border); border-left: 4px solid var(--info); border-radius: 8px; box-shadow: var(--shadow); padding: 10px 14px; line-height: 1.45; word-break: break-word; }
  .toast.ok { border-left-color: var(--ok); }
  .toast.err { border-left-color: var(--err); }
</style>
