import { demoSeed, type FilesHost, MockClient, memoryFiles, type SeedFile } from '@astra/client'
import { EditorView } from '@codemirror/view'
import { act, fireEvent, render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { AstraApp } from '../src'
import type { HostCapabilities } from '../src/context'
import { demoFiles } from '../src/screens/Editor'

/** The demo workspace, with its first repository (gateway) checked out at `path`. */
const seedWith = (path: string | null): SeedFile => ({
  ...demoSeed,
  entities: demoSeed.entities.map((e) =>
    e.kind === 'repository' && e.id === 'r-gateway' ? { ...e, path } : e,
  ),
})

function mount(host: HostCapabilities, path: string | null = '/work/gateway') {
  const client = new MockClient({ seed: seedWith(path), latency: 0 })
  return render(<AstraApp client={client} platform="mac" history={{ memory: '/code' }} host={host} />)
}

/** The CodeMirror view showing `path`. */
async function editorFor(path: string): Promise<EditorView> {
  const content = await screen.findByLabelText(`Contents of ${path}`)
  const view = EditorView.findFromDOM(content)
  if (!view) throw new Error(`no editor for ${path}`)
  return view
}
const textOf = (v: EditorView) => v.state.doc.toString()
/** Types at the end of the file, as a person would (one transaction, caret after the text). */
const typeAtEnd = (v: EditorView, text: string) =>
  act(() => {
    const end = v.state.doc.length
    v.dispatch({
      changes: { from: end, insert: text },
      selection: { anchor: end + text.length },
      userEvent: 'input',
    })
  })

/** A memory tree whose calls are recorded. */
function spyFiles(initial: Record<string, string>) {
  const inner = memoryFiles(initial)
  const files: FilesHost = {
    list: vi.fn(inner.list),
    read: vi.fn(inner.read),
    write: vi.fn(inner.write),
  }
  return files
}

describe('code editor', () => {
  it('lists the root, opens a file, and saves edits with Cmd+S', async () => {
    const files = spyFiles({ 'README.md': '# Gateway', 'src/main.ts': 'const a = 1', 'src/util.ts': '' })
    mount({ files })
    const user = userEvent.setup()

    const tree = await screen.findByRole('tree', { name: 'Files' })
    expect(
      within(tree)
        .getAllByRole('treeitem')
        .map((i) => i.textContent),
    ).toEqual(['src', 'README.md'])
    expect(files.list).toHaveBeenCalledWith('/work/gateway', '')
    // Folders load when they are opened.
    expect(files.list).toHaveBeenCalledTimes(1)
    const src = within(tree).getByRole('treeitem', { name: 'src' })
    expect(src.getAttribute('aria-expanded')).toBe('false')
    await user.click(src)
    expect(src.getAttribute('aria-expanded')).toBe('true')
    await user.click(await within(tree).findByRole('treeitem', { name: 'main.ts' }))
    expect(files.read).toHaveBeenCalledWith('/work/gateway', 'src/main.ts')

    const editor = await editorFor('src/main.ts')
    expect(textOf(editor)).toBe('const a = 1')
    expect(screen.getByRole('tab', { name: 'main.ts', selected: true })).toBeTruthy()
    const status = document.querySelector('section[aria-label="Editor"] footer') as HTMLElement
    expect(within(status).getByText('src/main.ts')).toBeTruthy()
    expect(within(status).getByText('TypeScript')).toBeTruthy()
    expect(within(status).getByText('SAVED')).toBeTruthy()

    await typeAtEnd(editor, '\n  b')
    expect(textOf(editor)).toBe('const a = 1\n  b')
    expect(screen.getByRole('tab', { name: 'main.ts, unsaved' })).toBeTruthy()
    expect(within(status).getByText('UNSAVED')).toBeTruthy()
    expect(within(status).getByText('LN 2, COL 4')).toBeTruthy()

    await user.keyboard('{Meta>}s{/Meta}')
    expect(files.write).toHaveBeenCalledWith('/work/gateway', 'src/main.ts', 'const a = 1\n  b')
    expect(await within(status).findByText('SAVED')).toBeTruthy()
    expect(screen.getByRole('tab', { name: 'main.ts' })).toBeTruthy()
    expect(await files.read('/work/gateway', 'src/main.ts')).toBe('const a = 1\n  b')
  })

  it('shows a failed save and keeps the edits', async () => {
    const files = spyFiles({ 'a.json': '{}' })
    files.write = vi.fn(async () => {
      throw new Error('Permission denied')
    })
    mount({ files })
    const user = userEvent.setup()
    await user.click(await screen.findByRole('treeitem', { name: 'a.json' }))
    const editor = await editorFor('a.json')
    await typeAtEnd(editor, ' ')
    await user.keyboard('{Control>}s{/Control}')
    expect(await screen.findByText('SAVE FAILED')).toBeTruthy()
    expect(screen.getByText('Permission denied')).toBeTruthy()
    expect(textOf(editor)).toBe('{} ')
  })

  it('walks the tree with the keyboard', async () => {
    const files = spyFiles({ 'src/lib/deep.rs': 'fn main() {}', 'src/main.ts': '', 'z.md': '' })
    mount({ files })
    const user = userEvent.setup()
    const tree = await screen.findByRole('tree', { name: 'Files' })
    const src = within(tree).getByRole('treeitem', { name: 'src' })
    expect(src.tabIndex).toBe(0)
    src.focus()
    await user.keyboard('{ArrowRight}')
    expect(src.getAttribute('aria-expanded')).toBe('true')
    await within(tree).findByRole('treeitem', { name: 'lib' })
    await user.keyboard('{ArrowRight}')
    expect(document.activeElement?.textContent).toBe('lib')
    await user.keyboard('{Enter}')
    await within(tree).findByRole('treeitem', { name: 'deep.rs' })
    await user.keyboard('{ArrowDown}{Enter}')
    expect(textOf(await editorFor('src/lib/deep.rs'))).toBe('fn main() {}')
    expect(screen.getByText('Rust')).toBeTruthy()
    // Left goes to the parent, then collapses it.
    await user.click(within(tree).getByRole('treeitem', { name: 'deep.rs' }))
    await user.keyboard('{ArrowLeft}')
    expect(document.activeElement?.textContent).toBe('lib')
    await user.keyboard('{ArrowLeft}')
    expect(within(tree).queryByRole('treeitem', { name: 'deep.rs' })).toBeNull()
  })

  it('closes tabs, asking before it discards edits', async () => {
    const files = spyFiles({ 'a.ts': 'a', 'b.ts': 'b' })
    mount({ files })
    const user = userEvent.setup()
    await user.click(await screen.findByRole('treeitem', { name: 'a.ts' }))
    await user.click(screen.getByRole('treeitem', { name: 'b.ts' }))
    expect(screen.getAllByRole('tab')).toHaveLength(2)
    await typeAtEnd(await editorFor('b.ts'), '!')
    await user.click(screen.getByRole('button', { name: 'Close b.ts' }))
    expect(screen.getAllByRole('tab')).toHaveLength(2)
    await user.click(screen.getByRole('button', { name: 'Discard changes to b.ts' }))
    expect(screen.getAllByRole('tab').map((t) => t.textContent)).toEqual(['a.ts'])
    expect(textOf(await editorFor('a.ts'))).toBe('a')
    await user.click(screen.getByRole('button', { name: 'Close a.ts' }))
    expect(screen.queryAllByRole('tab')).toHaveLength(0)
    expect(screen.getByText('Open a file from the explorer.')).toBeTruthy()
  })

  it('finds in the open file with the editor search, from inside the editor or anywhere on screen', async () => {
    const files = spyFiles({ 'a.ts': 'let x = 1\nlet y = x' })
    mount({ files })
    const user = userEvent.setup()
    await user.click(await screen.findByRole('treeitem', { name: 'a.ts' }))
    const editor = await editorFor('a.ts')
    // ⌘F from the explorer opens the editor's search.
    await user.keyboard('{Meta>}f{/Meta}')
    const input = await screen.findByRole('textbox', { name: 'Find' })
    expect(document.activeElement).toBe(input)
    await user.type(input, 'let')
    // CodeMirror's search box reads the legacy keyCode for Enter, which user-event leaves out in jsdom.
    const enter = () => fireEvent.keyDown(input, { key: 'Enter', keyCode: 13 })
    enter()
    expect([editor.state.selection.main.from, editor.state.selection.main.to]).toEqual([0, 3])
    enter()
    expect([editor.state.selection.main.from, editor.state.selection.main.to]).toEqual([10, 13])
    await user.keyboard('{Escape}')
    expect(screen.queryByRole('textbox', { name: 'Find' })).toBeNull()
  })

  it('keeps each tab’s undo history when switching tabs', async () => {
    const files = spyFiles({ 'a.ts': 'a', 'b.ts': 'b' })
    mount({ files })
    const user = userEvent.setup()
    await user.click(await screen.findByRole('treeitem', { name: 'a.ts' }))
    await typeAtEnd(await editorFor('a.ts'), '1')
    await user.click(screen.getByRole('treeitem', { name: 'b.ts' }))
    await editorFor('b.ts')
    await user.click(screen.getByRole('tab', { name: 'a.ts, unsaved' }))
    const a = await editorFor('a.ts')
    expect(textOf(a)).toBe('a1')
    const { undo } = await import('@codemirror/commands')
    act(() => {
      undo(a)
    })
    expect(textOf(a)).toBe('a')
    expect(screen.getByRole('tab', { name: 'a.ts' })).toBeTruthy()
  })

  it('says so when the repository has no local folder', async () => {
    mount({ files: memoryFiles({}) }, null)
    expect(await screen.findByRole('heading', { name: 'This repository has no local folder.' })).toBeTruthy()
    expect(screen.queryByRole('tree')).toBeNull()
  })

  it('on the web, shows the mission results as demo files', async () => {
    mount({}, null)
    expect(await screen.findByText('DEMO FILES · CHANGES STAY IN THIS TAB')).toBeTruthy()
    const user = userEvent.setup()
    await user.click(await screen.findByRole('treeitem', { name: 'src' }))
    await user.click(await screen.findByRole('treeitem', { name: 'pricing' }))
    await user.click(await screen.findByRole('treeitem', { name: 'handler.ts' }))
    const editor = textOf(await editorFor('src/pricing/handler.ts'))
    const result = demoSeed.entities.find((e) => e.kind === 'result' && e.id === 'res-015-a')
    const diff = result?.kind === 'result' ? result.diff[0] : undefined
    const expected = diff?.lines
      .filter((l) => l.kind !== 'del')
      .map((l) => l.text)
      .join('\n')
    expect(editor).toBe(expected)
    expect(diff?.lines.some((l) => l.kind === 'del' && editor.includes(l.text))).toBe(false)
  })

  it('builds demo files only from the repository’s own missions', () => {
    const client = new MockClient({ seed: demoSeed, latency: 0 })
    const s = client.getState()
    expect(Object.keys(demoFiles(s, 'r-gateway') ?? {}).sort()).toEqual([
      'src/cache/cached.ts',
      'src/pricing/handler.ts',
    ])
    expect(demoFiles(s, 'r-atlas')).toBeNull()
  })

  it('opens from the command palette and the header', async () => {
    const client = new MockClient({ seed: demoSeed, latency: 0 })
    render(<AstraApp client={client} platform="mac" history={{ memory: '/' }} />)
    const user = userEvent.setup()
    await screen.findByRole('heading', { name: 'Start a mission.' })
    expect(screen.getByRole('link', { name: 'CODE' }).getAttribute('href')).toBe('/code')
    await user.keyboard('{Meta>}k{/Meta}')
    await user.type(
      screen.getByRole('combobox', { name: 'Search missions, screens and actions' }),
      'code editor',
    )
    await user.click(screen.getByRole('option', { name: /Open code editor/ }))
    expect(await screen.findByRole('heading', { name: 'Code editor' })).toBeTruthy()
  })
})
