import {
  autocompletion,
  closeBrackets,
  closeBracketsKeymap,
  completionKeymap,
} from '@codemirror/autocomplete'
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands'
import { css } from '@codemirror/lang-css'
import { go } from '@codemirror/lang-go'
import { html } from '@codemirror/lang-html'
import { javascript } from '@codemirror/lang-javascript'
import { json } from '@codemirror/lang-json'
import { markdown } from '@codemirror/lang-markdown'
import { python } from '@codemirror/lang-python'
import { rust } from '@codemirror/lang-rust'
import { yaml } from '@codemirror/lang-yaml'
import {
  bracketMatching,
  foldGutter,
  foldKeymap,
  HighlightStyle,
  indentOnInput,
  StreamLanguage,
  syntaxHighlighting,
} from '@codemirror/language'
import { dockerFile } from '@codemirror/legacy-modes/mode/dockerfile'
import { shell } from '@codemirror/legacy-modes/mode/shell'
import { toml } from '@codemirror/legacy-modes/mode/toml'
import { highlightSelectionMatches, search, searchKeymap } from '@codemirror/search'
import { EditorState, type Extension } from '@codemirror/state'
import {
  crosshairCursor,
  drawSelection,
  dropCursor,
  EditorView,
  highlightActiveLine,
  highlightActiveLineGutter,
  highlightSpecialChars,
  keymap,
  lineNumbers,
  rectangularSelection,
} from '@codemirror/view'
import { tags as t } from '@lezer/highlight'
import { useEffect, useRef } from 'react'

// ── Languages ──────────────────────────────────────────────────────────────

export interface LanguageInfo {
  label: string
  extension: () => Extension
}

const plain: LanguageInfo = { label: 'Plain text', extension: () => [] }
const ts: LanguageInfo = { label: 'TypeScript', extension: () => javascript({ typescript: true }) }
const tsx: LanguageInfo = {
  label: 'TypeScript JSX',
  extension: () => javascript({ typescript: true, jsx: true }),
}
const js: LanguageInfo = { label: 'JavaScript', extension: () => javascript() }
const jsx: LanguageInfo = { label: 'JavaScript JSX', extension: () => javascript({ jsx: true }) }
const jsonLang: LanguageInfo = { label: 'JSON', extension: () => json() }
const cssLang: LanguageInfo = { label: 'CSS', extension: () => css() }
const htmlLang: LanguageInfo = { label: 'HTML', extension: () => html() }
const md: LanguageInfo = { label: 'Markdown', extension: () => markdown() }
const rs: LanguageInfo = { label: 'Rust', extension: () => rust() }
const py: LanguageInfo = { label: 'Python', extension: () => python() }
const yml: LanguageInfo = { label: 'YAML', extension: () => yaml() }
const golang: LanguageInfo = { label: 'Go', extension: () => go() }
const sh: LanguageInfo = { label: 'Shell', extension: () => StreamLanguage.define(shell) }
const tomlLang: LanguageInfo = { label: 'TOML', extension: () => StreamLanguage.define(toml) }
const docker: LanguageInfo = { label: 'Dockerfile', extension: () => StreamLanguage.define(dockerFile) }

const byExtension: Record<string, LanguageInfo> = {
  ts,
  mts: ts,
  cts: ts,
  tsx,
  js,
  mjs: js,
  cjs: js,
  jsx,
  json: jsonLang,
  jsonc: jsonLang,
  json5: jsonLang,
  css: cssLang,
  scss: cssLang,
  less: cssLang,
  html: htmlLang,
  htm: htmlLang,
  svg: htmlLang,
  xml: htmlLang,
  vue: htmlLang,
  md,
  markdown: md,
  mdx: md,
  rs,
  py,
  pyi: py,
  yaml: yml,
  yml,
  go: golang,
  sh,
  bash: sh,
  zsh: sh,
  fish: sh,
  toml: tomlLang,
}

const byName: Record<string, LanguageInfo> = {
  dockerfile: docker,
  '.bashrc': sh,
  '.zshrc': sh,
  '.profile': sh,
  '.env': sh,
  'cargo.lock': tomlLang,
}

/** The language of a file, from its name. */
export function languageFor(path: string): LanguageInfo {
  const name = (path.split('/').pop() ?? path).toLowerCase()
  if (byName[name]) return byName[name]
  if (name.startsWith('dockerfile')) return docker
  if (name.startsWith('.env')) return sh
  const ext = name.includes('.') ? name.slice(name.lastIndexOf('.') + 1) : ''
  return byExtension[ext] ?? plain
}

// ── Look ───────────────────────────────────────────────────────────────────

/** Syntax colours from the design tokens, so they follow the light and dark themes. */
export const astraHighlight = HighlightStyle.define([
  {
    tag: [t.keyword, t.modifier, t.operatorKeyword, t.controlKeyword, t.definitionKeyword],
    color: 'var(--chart-3)',
  },
  { tag: [t.string, t.special(t.string), t.character, t.inserted], color: 'var(--signal)' },
  { tag: [t.number, t.bool, t.null, t.atom, t.unit], color: 'var(--warning)' },
  { tag: [t.regexp, t.escape, t.deleted, t.invalid], color: 'var(--danger)' },
  {
    tag: [t.comment, t.lineComment, t.blockComment, t.docComment],
    color: 'var(--ink-muted)',
    fontStyle: 'italic',
  },
  { tag: [t.typeName, t.className, t.namespace, t.standard(t.typeName)], color: 'var(--success)' },
  {
    tag: [t.function(t.variableName), t.function(t.propertyName), t.definition(t.function(t.variableName))],
    color: 'var(--ink)',
    fontWeight: '600',
  },
  { tag: [t.propertyName, t.attributeName], color: 'var(--marking-confidential)' },
  { tag: [t.tagName, t.angleBracket], color: 'var(--chart-3)' },
  {
    tag: [t.meta, t.annotation, t.processingInstruction, t.punctuation, t.separator],
    color: 'var(--ink-muted)',
  },
  { tag: [t.operator, t.derefOperator, t.arithmeticOperator, t.logicOperator], color: 'var(--ink-muted)' },
  { tag: [t.heading, t.strong], color: 'var(--ink)', fontWeight: '600' },
  { tag: t.emphasis, fontStyle: 'italic' },
  { tag: [t.link, t.url], color: 'var(--chart-3)', textDecoration: 'underline' },
  { tag: t.monospace, color: 'var(--signal)' },
  { tag: [t.variableName, t.labelName], color: 'var(--ink)' },
])

const control = {
  backgroundColor: 'var(--surface-raised)',
  border: '1px solid var(--line-strong)',
  borderRadius: '2px',
  color: 'var(--ink)',
  font: '13px/20px var(--font-mono)',
  padding: '2px 8px',
}

export const astraTheme = EditorView.theme({
  '&': { height: '100%', backgroundColor: 'var(--surface-sunken)', color: 'var(--ink)', fontSize: '13px' },
  '&.cm-focused': { outline: 'none' },
  '.cm-scroller': { fontFamily: 'var(--font-mono)', lineHeight: '20px' },
  '.cm-content': { caretColor: 'var(--ink)', padding: '12px 0' },
  '.cm-line': { padding: '0 16px 0 8px' },
  '.cm-cursor, .cm-dropCursor': { borderLeftColor: 'var(--ink)', borderLeftWidth: '2px' },
  // Selection: a clear blue tint (dimmer when the editor isn't focused); text keeps its syntax colour.
  '.cm-selectionBackground': { backgroundColor: 'var(--selection-inactive)' },
  '&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground': {
    backgroundColor: 'var(--selection)',
  },
  '.cm-content ::selection': { backgroundColor: 'var(--selection)' },
  '.cm-gutters': {
    backgroundColor: 'var(--surface-sunken)',
    color: 'var(--ink-muted)',
    borderRight: '1px solid var(--line)',
  },
  '.cm-lineNumbers .cm-gutterElement': { padding: '0 8px 0 16px', minWidth: '32px' },
  // Hairlines, not a fill: the selection is drawn beneath the lines, and a fill would hide it.
  '.cm-activeLine': {
    backgroundColor: 'transparent',
    boxShadow: 'inset 0 1px var(--line-subtle), inset 0 -1px var(--line-subtle)',
  },
  '.cm-activeLineGutter': { backgroundColor: 'var(--surface-alt)', color: 'var(--ink)' },
  '.cm-foldGutter .cm-gutterElement': { padding: '0 4px', cursor: 'pointer' },
  '.cm-foldPlaceholder': {
    backgroundColor: 'var(--surface-raised)',
    border: '1px solid var(--line)',
    color: 'var(--ink-muted)',
    borderRadius: '2px',
  },
  '&.cm-focused .cm-matchingBracket': {
    backgroundColor: 'transparent',
    outline: '1px solid var(--ink-muted)',
  },
  '&.cm-focused .cm-nonmatchingBracket': { backgroundColor: 'var(--danger-soft)' },
  '.cm-selectionMatch': {
    backgroundColor: 'var(--selection-match)',
    outline: '1px solid var(--selection)',
    borderRadius: '2px',
  },
  '.cm-searchMatch': { backgroundColor: 'var(--warning-soft)', outline: '1px solid var(--warning)' },
  '.cm-searchMatch.cm-searchMatch-selected': {
    backgroundColor: 'var(--warning)',
    color: 'var(--on-marking)',
  },
  '.cm-panels': { backgroundColor: 'var(--surface-alt)', color: 'var(--ink)' },
  '.cm-panels.cm-panels-top': { borderBottom: '1px solid var(--line)' },
  '.cm-panels.cm-panels-bottom': { borderTop: '1px solid var(--line)' },
  '.cm-panel.cm-search': { padding: '6px 40px 6px 16px', fontFamily: 'var(--font-sans)', fontSize: '13px' },
  '.cm-panel.cm-search label': { color: 'var(--ink-muted)', fontSize: '13px' },
  '.cm-panel.cm-search [name=close]': {
    color: 'var(--ink-muted)',
    fontSize: '18px',
    right: '12px',
    top: '6px',
    cursor: 'pointer',
  },
  '.cm-textfield': control,
  '.cm-textfield:focus': { outline: 'none', boxShadow: 'var(--focus-ring)' },
  '.cm-button': {
    backgroundImage: 'none',
    backgroundColor: 'transparent',
    border: '1px solid var(--line-strong)',
    borderRadius: '0',
    color: 'var(--ink)',
    fontFamily: 'var(--font-sans)',
    fontSize: '11px',
    fontWeight: '600',
    letterSpacing: '0.14em',
    textTransform: 'uppercase',
    padding: '3px 10px',
    cursor: 'pointer',
  },
  '.cm-button:hover': { backgroundColor: 'var(--invert)', color: 'var(--on-invert)' },
  '.cm-button:active': { backgroundImage: 'none' },
  '.cm-tooltip': {
    backgroundColor: 'var(--surface-overlay)',
    border: '1px solid var(--line)',
    color: 'var(--ink)',
    boxShadow: 'var(--shadow-lg)',
  },
  '.cm-tooltip-autocomplete > ul': { fontFamily: 'var(--font-mono)', fontSize: '13px' },
  '.cm-tooltip-autocomplete > ul > li': { padding: '2px 8px' },
  '.cm-tooltip-autocomplete > ul > li[aria-selected]': {
    backgroundColor: 'var(--signal-soft)',
    color: 'var(--ink)',
  },
  '.cm-completionIcon': { opacity: '0.7' },
  '.cm-specialChar': { color: 'var(--danger)' },
})

/** The editing features every file gets. */
function baseExtensions(): Extension[] {
  return [
    lineNumbers(),
    highlightActiveLineGutter(),
    foldGutter({
      markerDOM: (open) => {
        const el = document.createElement('span')
        el.textContent = open ? '⌄' : '›'
        el.setAttribute('aria-hidden', 'true')
        return el
      },
    }),
    highlightSpecialChars(),
    history(),
    drawSelection(),
    dropCursor(),
    EditorState.allowMultipleSelections.of(true),
    indentOnInput(),
    syntaxHighlighting(astraHighlight, { fallback: true }),
    bracketMatching(),
    closeBrackets(),
    autocompletion(),
    rectangularSelection(),
    crosshairCursor(),
    highlightActiveLine(),
    highlightSelectionMatches(),
    search({ top: true }),
    EditorState.tabSize.of(2),
    keymap.of([
      ...closeBracketsKeymap,
      ...defaultKeymap,
      ...searchKeymap,
      ...historyKeymap,
      ...foldKeymap,
      ...completionKeymap,
      indentWithTab,
    ]),
    astraTheme,
  ]
}

/** A fresh editor state for a file. */
export function createEditorState(text: string, path: string, label: string): EditorState {
  return EditorState.create({
    doc: text,
    extensions: [
      baseExtensions(),
      languageFor(path).extension(),
      EditorView.contentAttributes.of({ 'aria-label': label }),
    ],
  })
}

// ── Component ──────────────────────────────────────────────────────────────

export interface Cursor {
  line: number
  col: number
}

/**
 * CodeMirror for one open file. Its state (undo history, selection, folds) lives in `states` while the tab is
 * closed or another tab is showing, so switching tabs keeps it.
 */
export function CodeEditor({
  path,
  text,
  states,
  viewRef,
  onChange,
  onCursor,
}: {
  path: string
  text: string
  states: Map<string, EditorState>
  viewRef?: { current: EditorView | null }
  onChange: (text: string) => void
  onCursor: (c: Cursor) => void
}) {
  const host = useRef<HTMLDivElement>(null)
  const callbacks = useRef({ onChange, onCursor })
  callbacks.current = { onChange, onCursor }

  // biome-ignore lint/correctness/useExhaustiveDependencies: one view per file; later text comes from the view itself
  useEffect(() => {
    const parent = host.current
    if (!parent) return
    const report = (v: EditorView) => {
      const head = v.state.selection.main.head
      const line = v.state.doc.lineAt(head)
      callbacks.current.onCursor({ line: line.number, col: head - line.from + 1 })
    }
    const cached = states.get(path)
    const state =
      cached && cached.doc.toString() === text ? cached : createEditorState(text, path, `Contents of ${path}`)
    const view = new EditorView({
      state,
      parent,
      dispatchTransactions: (trs, v) => {
        v.update(trs)
        if (trs.some((tr) => tr.docChanged)) callbacks.current.onChange(v.state.doc.toString())
        if (trs.some((tr) => tr.selection || tr.docChanged)) report(v)
      },
    })
    if (viewRef) viewRef.current = view
    report(view)
    return () => {
      states.set(path, view.state)
      if (viewRef?.current === view) viewRef.current = null
      view.destroy()
    }
  }, [path])

  return <div ref={host} className="min-h-0 grow overflow-hidden" />
}
