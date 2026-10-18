<script lang="ts">
  // Editor CodeMirror 6 reutilizable: cuerpos raw, scripts y visor de respuesta (solo lectura, con plegado).
  import { onDestroy, onMount } from 'svelte';
  import { Compartment, EditorState } from '@codemirror/state';
  import { EditorView, drawSelection, keymap, lineNumbers as gutterLines, placeholder as cmPlaceholder } from '@codemirror/view';
  import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands';
  import { bracketMatching, foldGutter, foldKeymap, indentOnInput } from '@codemirror/language';
  import { highlightSelectionMatches, openSearchPanel, search, searchKeymap } from '@codemirror/search';
  import { closeBrackets, closeBracketsKeymap } from '@codemirror/autocomplete';
  import { editorTheme, highlight, language, refreshVars, variables, type EditorLang, type VarDescriber } from '../cm';

  interface Props {
    value: string;
    lang?: EditorLang;
    readonly?: boolean;
    lineNumbers?: boolean;
    fold?: boolean;
    wrap?: boolean;
    placeholder?: string;
    describeVar?: VarDescriber | null;
    label?: string;
    onchange?: (value: string) => void;
  }

  let {
    value,
    lang = 'text',
    readonly = false,
    lineNumbers = true,
    fold = false,
    wrap = false,
    placeholder = '',
    describeVar = null,
    label = '',
    onchange,
  }: Props = $props();

  let host: HTMLDivElement;
  let view: EditorView | undefined;
  const langSlot = new Compartment();
  const wrapSlot = new Compartment();
  const resolver = { fn: ((name: string) => ({ ok: true, text: name })) as VarDescriber };

  onMount(() => {
    if (describeVar) resolver.fn = describeVar;
    view = new EditorView({
      parent: host,
      state: EditorState.create({
        doc: value,
        extensions: [
          editorTheme,
          highlight,
          lineNumbers ? gutterLines() : [],
          fold ? foldGutter() : [],
          history(),
          drawSelection(),
          indentOnInput(),
          bracketMatching(),
          readonly ? [] : closeBrackets(),
          highlightSelectionMatches(),
          search({ top: true }),
          keymap.of([...closeBracketsKeymap, ...defaultKeymap, ...searchKeymap, ...historyKeymap, ...foldKeymap, indentWithTab]),
          langSlot.of(language(lang)),
          wrapSlot.of(wrap ? EditorView.lineWrapping : []),
          describeVar ? variables(resolver) : [],
          placeholder ? cmPlaceholder(placeholder) : [],
          EditorState.readOnly.of(readonly),
          EditorView.contentAttributes.of(label ? { 'aria-label': label } : {}),
          EditorView.updateListener.of((update) => {
            if (update.docChanged) onchange?.(update.state.doc.toString());
          }),
        ],
      }),
    });
  });

  onDestroy(() => view?.destroy());

  // El valor externo manda (cambio de pestaña, beautify, recarga); lo tecleado ya coincide.
  $effect(() => {
    const next = value;
    if (view && next !== view.state.doc.toString()) {
      view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: next } });
    }
  });

  $effect(() => {
    const next = lang;
    view?.dispatch({ effects: langSlot.reconfigure(language(next)) });
  });

  $effect(() => {
    const next = wrap;
    view?.dispatch({ effects: wrapSlot.reconfigure(next ? EditorView.lineWrapping : []) });
  });

  $effect(() => {
    const next = describeVar;
    if (next) {
      resolver.fn = next;
      view?.dispatch({ effects: refreshVars.of(null) });
    }
  });

  export function openSearch(): void {
    if (view) openSearchPanel(view);
  }

  export function focus(): void {
    view?.focus();
  }
</script>

<div class="editor" bind:this={host}></div>

<style>
  .editor {
    height: 100%;
    min-height: 0;
    border: 1px solid var(--border);
    border-radius: 6px;
    overflow: hidden;
    background: var(--code-bg);
  }
  .editor :global(.cm-editor) { height: 100%; }
</style>
