// CodeMirror 6 (sección 9.1): tema y resaltado con variables CSS, de modo que siguen al tema de la app
// sin reconfigurar el editor; y {{variables}} coloreadas (verde/rojo) con valor al pasar el ratón.
import { StateEffect, type Extension } from '@codemirror/state';
import { Decoration, EditorView, MatchDecorator, ViewPlugin, hoverTooltip, type DecorationSet, type ViewUpdate } from '@codemirror/view';
import { HighlightStyle, syntaxHighlighting } from '@codemirror/language';
import { tags } from '@lezer/highlight';
import { json } from '@codemirror/lang-json';
import { javascript } from '@codemirror/lang-javascript';
import { xml } from '@codemirror/lang-xml';
import { html } from '@codemirror/lang-html';

export type EditorLang = 'json' | 'javascript' | 'xml' | 'html' | 'text';

export function language(lang: EditorLang): Extension {
  switch (lang) {
    case 'json':
      return json();
    case 'javascript':
      return javascript();
    case 'xml':
      return xml();
    case 'html':
      return html();
    default:
      return [];
  }
}

export const editorTheme = EditorView.theme({
  '&': { color: 'var(--fg)', backgroundColor: 'var(--code-bg)', height: '100%' },
  '&.cm-focused': { outline: 'none' },
  '.cm-scroller': { fontFamily: 'Consolas, "Cascadia Mono", monospace', fontSize: '12.5px', lineHeight: '1.55' },
  '.cm-content': { caretColor: 'var(--fg)', padding: '8px 0' },
  '.cm-cursor, .cm-dropCursor': { borderLeftColor: 'var(--fg)' },
  '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection': { backgroundColor: 'var(--selection)' },
  '.cm-gutters': { backgroundColor: 'var(--code-bg)', color: 'var(--muted)', border: 'none' },
  '.cm-activeLine, .cm-activeLineGutter': { backgroundColor: 'transparent' },
  '.cm-foldPlaceholder': { backgroundColor: 'var(--bg-3)', border: 'none', color: 'var(--muted)', padding: '0 6px' },
  '.cm-foldGutter span': { color: 'var(--muted)' },
  '.cm-matchingBracket': { backgroundColor: 'var(--bg-3)', outline: '1px solid var(--border)' },
  '.cm-searchMatch': { backgroundColor: 'var(--selection)', outline: '1px solid var(--info)' },
  '.cm-searchMatch.cm-searchMatch-selected': { backgroundColor: 'var(--accent)', color: 'var(--on-accent)' },
  '.cm-panels': { backgroundColor: 'var(--bg-2)', color: 'var(--fg)', borderColor: 'var(--border)' },
  '.cm-panels.cm-panels-top': { borderBottom: '1px solid var(--border)' },
  '.cm-panel input, .cm-panel button': { fontSize: '12px' },
  '.cm-panel.cm-search label': { fontSize: '12px', color: 'var(--muted)' },
  '.cm-tooltip': { backgroundColor: 'var(--bg)', color: 'var(--fg)', border: '1px solid var(--border)', borderRadius: '6px', boxShadow: 'var(--shadow)' },
  '.cm-var-tip': { padding: '4px 8px', fontFamily: 'Consolas, monospace', fontSize: '12px', maxWidth: '420px', wordBreak: 'break-all' },
  '.cm-var-ok': { color: 'var(--ok)' },
  '.cm-var-bad': { color: 'var(--err)', textDecoration: 'underline wavy', textUnderlineOffset: '3px' },
  '.cm-placeholder': { color: 'var(--muted)' },
});

export const highlight = syntaxHighlighting(
  HighlightStyle.define([
    { tag: [tags.propertyName, tags.attributeName], color: 'var(--info)' },
    { tag: [tags.string, tags.special(tags.string), tags.attributeValue], color: 'var(--ok)' },
    { tag: [tags.number, tags.bool, tags.null, tags.atom], color: 'var(--warn)' },
    { tag: [tags.keyword, tags.operatorKeyword, tags.controlKeyword, tags.definitionKeyword, tags.modifier], color: 'var(--patch)' },
    { tag: [tags.comment, tags.lineComment, tags.blockComment], color: 'var(--muted)', fontStyle: 'italic' },
    { tag: [tags.tagName, tags.typeName, tags.className], color: 'var(--accent-text)' },
    { tag: [tags.invalid], color: 'var(--err)' },
  ]),
);

/** Cómo se presenta una variable: si resuelve y qué decir en el hover. */
export type VarDescriber = (name: string) => { ok: boolean; text: string };

/** Contenedor mutable: el componente cambia `fn` y despacha `refreshVars`. */
export interface VarResolver {
  fn: VarDescriber;
}

export const refreshVars = StateEffect.define<null>();
const VAR_RE = /\{\{\s*([^{}]+?)\s*\}\}/g;

export function variables(resolver: VarResolver): Extension {
  const decorator = new MatchDecorator({
    regexp: VAR_RE,
    decoration: (match) => Decoration.mark({ class: resolver.fn(match[1]).ok ? 'cm-var-ok' : 'cm-var-bad' }),
  });
  const plugin = ViewPlugin.fromClass(
    class {
      decorations: DecorationSet;
      constructor(view: EditorView) {
        this.decorations = decorator.createDeco(view);
      }
      update(update: ViewUpdate) {
        const refreshed = update.transactions.some((tr) => tr.effects.some((e) => e.is(refreshVars)));
        this.decorations = refreshed ? decorator.createDeco(update.view) : decorator.updateDeco(update, this.decorations);
      }
    },
    { decorations: (value) => value.decorations },
  );
  const tooltip = hoverTooltip((view, pos) => {
    const line = view.state.doc.lineAt(pos);
    for (const match of line.text.matchAll(VAR_RE)) {
      const from = line.from + (match.index ?? 0);
      const to = from + match[0].length;
      if (pos >= from && pos <= to) {
        return {
          pos: from,
          end: to,
          above: true,
          create: () => {
            const dom = document.createElement('div');
            dom.className = 'cm-var-tip';
            dom.textContent = resolver.fn(match[1]).text;
            return { dom };
          },
        };
      }
    }
    return null;
  });
  return [plugin, tooltip];
}
