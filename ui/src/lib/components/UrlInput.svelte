<script lang="ts">
  // Barra de URL (sección 9.1): una línea, {{variables}} en verde si resuelven y en rojo si no,
  // hover con el valor (o *** si es secreto). Enter envía, como en Postman.
  import { onDestroy, onMount } from 'svelte';
  import { EditorState } from '@codemirror/state';
  import { EditorView, keymap, placeholder as cmPlaceholder } from '@codemirror/view';
  import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
  import { editorTheme, refreshVars, variables, type VarDescriber } from '../cm';

  interface Props {
    value: string;
    describeVar: VarDescriber;
    placeholder?: string;
    label?: string;
    onchange: (value: string) => void;
    onsubmit: () => void;
  }

  let { value, describeVar, placeholder = '', label = '', onchange, onsubmit }: Props = $props();

  let host: HTMLDivElement;
  let view: EditorView | undefined;
  const resolver = { fn: ((name: string) => ({ ok: true, text: name })) as VarDescriber };

  const singleLine = EditorState.transactionFilter.of((tr) => {
    if (!tr.docChanged || tr.newDoc.lines <= 1) return tr;
    const flat = tr.newDoc.toString().replace(/\r?\n/g, '');
    return [tr, { changes: { from: 0, to: tr.newDoc.length, insert: flat }, sequential: true }];
  });

  const urlTheme = EditorView.theme({
    '&': { backgroundColor: 'var(--bg)', height: 'auto' },
    '.cm-scroller': { overflow: 'hidden', lineHeight: '1.5' },
    '.cm-content': { padding: '5px 0' },
    '.cm-line': { padding: '0 10px' },
  });

  onMount(() => {
    resolver.fn = describeVar;
    view = new EditorView({
      parent: host,
      state: EditorState.create({
        doc: value,
        extensions: [
          editorTheme,
          urlTheme,
          singleLine,
          history(),
          keymap.of([
            {
              key: 'Enter',
              run: () => {
                onsubmit();
                return true;
              },
            },
            ...historyKeymap,
            ...defaultKeymap,
          ]),
          variables(resolver),
          placeholder ? cmPlaceholder(placeholder) : [],
          EditorView.contentAttributes.of({ 'aria-label': label, spellcheck: 'false' }),
          EditorView.updateListener.of((update) => {
            if (update.docChanged) onchange(update.state.doc.toString());
          }),
        ],
      }),
    });
  });

  onDestroy(() => view?.destroy());

  $effect(() => {
    const next = value;
    if (view && next !== view.state.doc.toString()) {
      view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: next } });
    }
  });

  $effect(() => {
    resolver.fn = describeVar;
    view?.dispatch({ effects: refreshVars.of(null) });
  });

  export function focus(): void {
    view?.focus();
  }
</script>

<div class="url" bind:this={host}></div>

<style>
  .url {
    flex: 1;
    min-width: 0;
    border: 1px solid var(--border);
    border-radius: 6px;
    overflow: hidden;
    background: var(--bg);
  }
  .url:focus-within { outline: 2px solid var(--focus); outline-offset: -1px; }
</style>
