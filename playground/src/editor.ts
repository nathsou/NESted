// The code editor: CodeMirror 6 with NESted highlighting and the Rust
// language server (running in the compiler worker) for diagnostics, hover,
// completion, signatures, go to definition, rename and formatting.
import { EditorState, type Extension, StateEffect, StateField } from '@codemirror/state';
import { EditorView, keymap, lineNumbers, highlightActiveLine, highlightActiveLineGutter, drawSelection, hoverTooltip, type Tooltip, showTooltip, Decoration, type DecorationSet } from '@codemirror/view';
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands';
import { StreamLanguage, HighlightStyle, syntaxHighlighting, bracketMatching, indentOnInput, indentUnit } from '@codemirror/language';
import { autocompletion, closeBrackets, closeBracketsKeymap, completionKeymap, type CompletionContext, type CompletionResult } from '@codemirror/autocomplete';
import { setDiagnostics, lintGutter, type Diagnostic as LintDiagnostic } from '@codemirror/lint';
import { searchKeymap, highlightSelectionMatches } from '@codemirror/search';
import { tags as t } from '@lezer/highlight';

const KEYWORDS = new Set(['fn', 'let', 'var', 'const', 'if', 'else', 'while', 'for', 'in', 'loop', 'break', 'continue', 'return', 'match', 'struct', 'enum', 'type', 'use', 'pub', 'as', 'raw', 'asm', 'cartridge']);
const TYPES = new Set(['u8', 'i8', 'u16', 'i16', 'bool']);
const BUILTINS = new Set(['lo', 'hi', 'len', 'peek', 'poke', 'asset', 'defined', 'addr']);

interface State {
  asm: number;
  comment: number;
}

/** A tokenizer for NESted; assembly bodies are highlighted as such. */
const nested = StreamLanguage.define<State>({
  name: 'nested',
  startState: () => ({ asm: 0, comment: 0 }),
  token(stream, state) {
    if (state.comment > 0) {
      while (!stream.eol()) {
        if (stream.match('/*')) state.comment++;
        else if (stream.match('*/')) {
          state.comment--;
          if (state.comment === 0) break;
        } else stream.next();
      }
      return 'comment';
    }
    if (state.asm > 0) {
      if (stream.eatSpace()) return null;
      if (stream.peek() === ';') {
        stream.skipToEnd();
        return 'comment';
      }
      if (stream.peek() === '}') {
        stream.next();
        state.asm--;
        return 'brace';
      }
      if (stream.peek() === '{') {
        stream.match(/\{[^}]*\}/);
        return 'variableName.special';
      }
      if (stream.match(/^[a-zA-Z_][\w]*:/)) return 'labelName';
      if (stream.match(/^[a-zA-Z]{3}\b/)) return 'keyword';
      if (stream.match(/^#?[$%]?[0-9a-fA-F]+/)) return 'number';
      stream.next();
      return null;
    }
    if (stream.eatSpace()) return null;
    if (stream.match('//')) {
      stream.skipToEnd();
      return 'comment';
    }
    if (stream.match('/*')) {
      state.comment = 1;
      return 'comment';
    }
    if (stream.match(/^"([^"\\]|\\.)*"?/)) return 'string';
    if (stream.match(/^'(\\.|[^'])'/)) return 'string';
    if (stream.match(/^0x[0-9a-fA-F_]+|^0b[01_]+|^\d[\d_]*/)) return 'number';
    if (stream.match(/^@\w+/)) return 'meta';
    const word = stream.match(/^[A-Za-z_]\w*/) as RegExpMatchArray | null;
    if (word) {
      const w = word[0];
      if (w === 'asm') {
        // The body after an optional input list is assembly.
        stream.match(/^\s*(\([^)]*\))?\s*\{/) && state.asm++;
        return 'keyword';
      }
      if (KEYWORDS.has(w)) return 'keyword';
      if (w === 'true' || w === 'false') return 'bool';
      if (TYPES.has(w)) return 'typeName';
      if (BUILTINS.has(w)) return 'variableName.standard';
      if (/^[A-Z][A-Z0-9_]+$/.test(w)) return 'variableName.constant';
      if (/^[A-Z]/.test(w)) return 'typeName';
      if (stream.match(/^\s*\(/, false)) return 'variableName.function';
      if (stream.match(/^::/, false)) return 'namespace';
      return 'variableName';
    }
    stream.next();
    return 'operator';
  },
  languageData: { commentTokens: { line: '//' }, closeBrackets: { brackets: ['(', '[', '{', '"'] } },
});

const highlight = HighlightStyle.define([
  { tag: t.keyword, color: 'var(--syn-keyword)' },
  { tag: t.comment, color: 'var(--syn-comment)', fontStyle: 'italic' },
  { tag: t.string, color: 'var(--syn-string)' },
  { tag: t.number, color: 'var(--syn-number)' },
  { tag: t.bool, color: 'var(--syn-number)' },
  { tag: t.typeName, color: 'var(--syn-type)' },
  { tag: t.function(t.variableName), color: 'var(--syn-function)' },
  { tag: t.namespace, color: 'var(--syn-namespace)' },
  { tag: t.meta, color: 'var(--syn-meta)' },
  { tag: t.constant(t.variableName), color: 'var(--syn-constant)' },
  { tag: t.standard(t.variableName), color: 'var(--syn-function)' },
  { tag: t.special(t.variableName), color: 'var(--syn-meta)' },
  { tag: t.labelName, color: 'var(--syn-meta)' },
  { tag: t.operator, color: 'var(--syn-operator)' },
]);

export interface Location {
  path: string;
  line: number;
  character: number;
}

export interface LspClient {
  request(method: string, params: unknown): Promise<unknown>;
  notify(method: string, params: unknown): void;
}

export interface EditorHost {
  lsp: LspClient;
  uri(path: string): string;
  pathOf(uri: string): string | null;
  open(location: Location): void;
  changed(path: string, text: string): void;
  format(text: string): Promise<string>;
  applyEdits(edits: Record<string, { range: Range; newText: string }[]>): void;
}

interface Range {
  start: { line: number; character: number };
  end: { line: number; character: number };
}

function offset(doc: EditorState['doc'], pos: { line: number; character: number }): number {
  const line = doc.line(Math.min(doc.lines, pos.line + 1));
  return Math.min(line.to, line.from + pos.character);
}

function position(doc: EditorState['doc'], at: number) {
  const line = doc.lineAt(at);
  return { line: line.number - 1, character: at - line.from };
}

function markdown(text: string): HTMLElement {
  const box = document.createElement('div');
  box.className = 'cm-docs';
  const parts = text.split(/```(?:nested)?\n?/);
  parts.forEach((part, i) => {
    if (!part.trim()) return;
    const el = document.createElement(i % 2 ? 'pre' : 'p');
    el.textContent = part.trim();
    box.appendChild(el);
  });
  return box;
}

// Signature help shown as a tooltip above the cursor.
const signatureEffect = StateEffect.define<Tooltip | null>();
const signatureField = StateField.define<Tooltip | null>({
  create: () => null,
  update(value, tr) {
    for (const e of tr.effects) if (e.is(signatureEffect)) return e.value;
    if (tr.docChanged && value) return null;
    return value;
  },
  provide: (f) => showTooltip.from(f),
});

// The line of the current program counter, highlighted while paused.
export const setPcLine = StateEffect.define<number | null>();
const pcField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(value, tr) {
    for (const e of tr.effects) {
      if (e.is(setPcLine)) {
        if (e.value === null || e.value >= tr.state.doc.lines) return Decoration.none;
        const line = tr.state.doc.line(e.value + 1);
        return Decoration.set([Decoration.line({ class: 'cm-pc-line' }).range(line.from)]);
      }
    }
    return value.map(tr.changes);
  },
  provide: (f) => EditorView.decorations.from(f),
});

export class Editor {
  readonly view: EditorView;
  private states = new Map<string, EditorState>();
  private versions = new Map<string, number>();
  path = '';
  private timer: ReturnType<typeof setTimeout> | undefined;
  private readonly host: EditorHost;
  private readonly diagnostics = new Map<string, LintDiagnostic[]>();
  readOnly = false;

  constructor(parent: HTMLElement, host: EditorHost) {
    this.host = host;
    this.view = new EditorView({ parent, state: EditorState.create({ doc: '', extensions: this.extensions() }) });
  }

  private extensions(): Extension[] {
    const host = this.host;
    return [
      lineNumbers(),
      highlightActiveLineGutter(),
      history(),
      drawSelection(),
      indentOnInput(),
      bracketMatching(),
      closeBrackets(),
      highlightActiveLine(),
      highlightSelectionMatches(),
      lintGutter(),
      indentUnit.of('    '),
      nested,
      syntaxHighlighting(highlight),
      signatureField,
      pcField,
      EditorState.tabSize.of(4),
      EditorView.updateListener.of((u) => {
        if (u.docChanged) {
          const text = u.state.doc.toString();
          this.states.set(this.path, u.state);
          clearTimeout(this.timer);
          const path = this.path;
          this.timer = setTimeout(() => this.sync(path, text), 120);
          host.changed(path, text);
        }
        if (u.selectionSet || u.docChanged) this.maybeSignature(u.view);
      }),
      autocompletion({ override: [(ctx) => this.complete(ctx)], activateOnTyping: true }),
      hoverTooltip(async (view, pos) => {
        this.flush();
        const result = (await host.lsp.request('textDocument/hover', { textDocument: { uri: host.uri(this.path) }, position: position(view.state.doc, pos) })) as { contents?: { value: string } } | null;
        if (!result?.contents?.value) return null;
        return { pos, above: true, create: () => ({ dom: markdown(result.contents!.value) }) };
      }),
      keymap.of([
        { key: 'F12', run: () => (this.definition(), true) },
        { key: 'Mod-b', run: () => (this.definition(), true) },
        { key: 'F2', run: () => (this.rename(), true) },
        { key: 'Shift-Alt-f', run: () => (this.format(), true) },
        { key: 'Mod-Shift-i', run: () => (this.format(), true) },
        ...closeBracketsKeymap,
        ...defaultKeymap,
        ...searchKeymap,
        ...historyKeymap,
        ...completionKeymap,
        indentWithTab,
      ]),
      EditorView.domEventHandlers({
        mousedown: (event, view) => {
          if (!(event.ctrlKey || event.metaKey)) return false;
          const pos = view.posAtCoords({ x: event.clientX, y: event.clientY });
          if (pos === null) return false;
          view.dispatch({ selection: { anchor: pos } });
          this.definition();
          return true;
        },
      }),
      EditorState.readOnly.of(false),
    ];
  }

  /** Open a document in the language server and keep a state for it. */
  addFile(path: string, text: string) {
    this.states.set(path, EditorState.create({ doc: text, extensions: this.extensions() }));
    this.versions.set(path, 1);
    this.host.lsp.notify('textDocument/didOpen', { textDocument: { uri: this.host.uri(path), languageId: 'nested', version: 1, text } });
  }

  removeFile(path: string) {
    this.states.delete(path);
    this.versions.delete(path);
  }

  clear() {
    this.states.clear();
    this.versions.clear();
    this.diagnostics.clear();
  }

  show(path: string) {
    if (this.path === path) return;
    if (this.path && this.states.has(this.path)) this.states.set(this.path, this.view.state);
    this.path = path;
    const state = this.states.get(path);
    if (state) this.view.setState(state);
    this.applyDiagnostics();
  }

  text(path = this.path): string {
    if (path === this.path) return this.view.state.doc.toString();
    return this.states.get(path)?.doc.toString() ?? '';
  }

  setText(path: string, text: string) {
    if (path === this.path) {
      this.view.dispatch({ changes: { from: 0, to: this.view.state.doc.length, insert: text } });
    } else {
      const state = this.states.get(path);
      if (!state) return;
      this.states.set(path, state.update({ changes: { from: 0, to: state.doc.length, insert: text } }).state);
      this.sync(path, text);
      this.host.changed(path, text);
    }
  }

  private sync(path: string, text: string) {
    const version = (this.versions.get(path) ?? 1) + 1;
    this.versions.set(path, version);
    this.host.lsp.notify('textDocument/didChange', { textDocument: { uri: this.host.uri(path), version }, contentChanges: [{ text }] });
  }

  /** Send pending edits before a request that depends on them. */
  flush() {
    if (this.timer) {
      clearTimeout(this.timer);
      this.timer = undefined;
      this.sync(this.path, this.view.state.doc.toString());
    }
  }

  setDiagnostics(path: string, items: { range: Range; severity: number; message: string }[]) {
    const state = path === this.path ? this.view.state : this.states.get(path);
    if (!state) return;
    const doc = state.doc;
    this.diagnostics.set(
      path,
      items.map((d) => {
        const from = offset(doc, d.range.start);
        let to = offset(doc, d.range.end);
        if (to <= from) to = Math.min(doc.length, from + 1);
        return { from, to, severity: d.severity === 1 ? 'error' : d.severity === 2 ? 'warning' : 'info', message: d.message } as LintDiagnostic;
      }),
    );
    if (path === this.path) this.applyDiagnostics();
  }

  private applyDiagnostics() {
    this.view.dispatch(setDiagnostics(this.view.state, this.diagnostics.get(this.path) ?? []));
  }

  goto(line: number, character = 0) {
    const doc = this.view.state.doc;
    const at = offset(doc, { line, character });
    this.view.dispatch({ selection: { anchor: at }, effects: EditorView.scrollIntoView(at, { y: 'center' }) });
    this.view.focus();
  }

  markPc(line: number | null) {
    this.view.dispatch({ effects: setPcLine.of(line) });
  }

  private async complete(ctx: CompletionContext): Promise<CompletionResult | null> {
    const word = ctx.matchBefore(/[\w@]*/);
    const trigger = ctx.matchBefore(/(::|\.)$/);
    if (!ctx.explicit && !trigger && (!word || word.from === word.to)) return null;
    this.flush();
    type Item = { label: string; kind: number; detail?: string };
    const reply = (await this.host.lsp.request('textDocument/completion', { textDocument: { uri: this.host.uri(this.path) }, position: position(ctx.state.doc, ctx.pos) })) as Item[] | { items: Item[] } | null;
    const result = Array.isArray(reply) ? reply : reply?.items;
    if (!result) return null;
    const kinds: Record<number, string> = { 3: 'function', 5: 'property', 6: 'variable', 7: 'type', 9: 'namespace', 13: 'enum', 14: 'keyword', 20: 'enum', 22: 'class' };
    return {
      from: word ? word.from + (word.text.startsWith('@') ? 1 : 0) : ctx.pos,
      options: result.map((i) => ({ label: i.label, type: kinds[i.kind] ?? 'text', detail: i.detail?.split('\n')[0]?.slice(0, 60) })),
      validFor: /^\w*$/,
    };
  }

  private async maybeSignature(view: EditorView) {
    const pos = view.state.selection.main.head;
    const before = view.state.doc.sliceString(Math.max(0, pos - 80), pos);
    let depth = 0;
    let inside = false;
    for (let i = before.length - 1; i >= 0; i--) {
      const c = before[i];
      if (c === ')') depth++;
      else if (c === '(') {
        if (depth === 0) {
          inside = true;
          break;
        }
        depth--;
      } else if (c === ';' || c === '{' || c === '}') break;
    }
    if (!inside) {
      if (view.state.field(signatureField)) view.dispatch({ effects: signatureEffect.of(null) });
      return;
    }
    this.flush();
    const help = (await this.host.lsp.request('textDocument/signatureHelp', { textDocument: { uri: this.host.uri(this.path) }, position: position(view.state.doc, pos) })) as { signatures: { label: string; documentation?: string; parameters: { label: string }[] }[]; activeParameter: number } | null;
    if (!help?.signatures?.length) {
      view.dispatch({ effects: signatureEffect.of(null) });
      return;
    }
    const sig = help.signatures[0];
    view.dispatch({
      effects: signatureEffect.of({
        pos,
        above: true,
        create: () => {
          const dom = document.createElement('div');
          dom.className = 'cm-signature';
          const active = sig.parameters[help.activeParameter]?.label;
          const label = document.createElement('code');
          if (active && sig.label.includes(active)) {
            const [a, b] = [sig.label.indexOf(active), sig.label.indexOf(active) + active.length];
            label.append(sig.label.slice(0, a));
            const em = document.createElement('b');
            em.textContent = active;
            label.append(em, sig.label.slice(b));
          } else label.textContent = sig.label;
          dom.append(label);
          if (sig.documentation) {
            const p = document.createElement('div');
            p.textContent = sig.documentation;
            dom.append(p);
          }
          return { dom };
        },
      }),
    });
  }

  async definition() {
    this.flush();
    const pos = this.view.state.selection.main.head;
    const result = (await this.host.lsp.request('textDocument/definition', { textDocument: { uri: this.host.uri(this.path) }, position: position(this.view.state.doc, pos) })) as { uri: string; range: Range } | null;
    if (!result?.uri) return;
    const path = this.host.pathOf(result.uri);
    if (path) this.host.open({ path, line: result.range.start.line, character: result.range.start.character });
  }

  async rename() {
    this.flush();
    const pos = position(this.view.state.doc, this.view.state.selection.main.head);
    const range = (await this.host.lsp.request('textDocument/prepareRename', { textDocument: { uri: this.host.uri(this.path) }, position: pos })) as Range | null;
    if (!range) return;
    const doc = this.view.state.doc;
    const current = doc.sliceString(offset(doc, range.start), offset(doc, range.end));
    const name = prompt(`Rename '${current}' to`, current);
    if (!name || name === current) return;
    const edit = (await this.host.lsp.request('textDocument/rename', { textDocument: { uri: this.host.uri(this.path) }, position: pos, newName: name })) as { changes?: Record<string, { range: Range; newText: string }[]> } | null;
    if (edit?.changes) this.host.applyEdits(edit.changes);
  }

  /** Apply LSP text edits to a file. */
  edit(path: string, edits: { range: Range; newText: string }[]) {
    const apply = (state: EditorState) => {
      const changes = edits.map((e) => ({ from: offset(state.doc, e.range.start), to: offset(state.doc, e.range.end), insert: e.newText })).sort((a, b) => a.from - b.from);
      return changes;
    };
    if (path === this.path) this.view.dispatch({ changes: apply(this.view.state) });
    else {
      const state = this.states.get(path);
      if (!state) return;
      const next = state.update({ changes: apply(state) }).state;
      this.states.set(path, next);
      this.sync(path, next.doc.toString());
      this.host.changed(path, next.doc.toString());
    }
  }

  async format() {
    const text = this.view.state.doc.toString();
    const formatted = await this.host.format(text);
    if (formatted !== text) {
      const head = this.view.state.selection.main.head;
      this.view.dispatch({ changes: { from: 0, to: this.view.state.doc.length, insert: formatted }, selection: { anchor: Math.min(head, formatted.length) } });
    }
  }
}
