/** One row of a folder listing. `path` is relative to the root and uses `/` separators. */
export interface FileEntry {
  name: string
  path: string
  kind: 'file' | 'dir'
}

/**
 * Reads and writes the files of a repository folder for the code editor. Every `path`/`dir` is relative
 * to `root` ("" is the root itself); implementations refuse anything that would leave `root`.
 * The desktop shell provides one backed by the disk; the web and tests use `memoryFiles`.
 */
export interface FilesHost {
  /** The folder's entries, folders first, then by name. */
  list(root: string, dir: string): Promise<FileEntry[]>
  /** A UTF-8 text file's contents. */
  read(root: string, path: string): Promise<string>
  /** Replaces a file's contents (creating the file if its folder exists). */
  write(root: string, path: string, text: string): Promise<void>
  /**
   * The file as last committed (git `HEAD`), which the editor colours changes against. Null when there is
   * no committed copy (a new file, or a folder outside version control). Hosts without version control leave
   * it out, and the editor compares with the last saved text instead.
   */
  base?(root: string, path: string): Promise<string | null>
}

/** Folders the explorer never shows: version control internals and build output. */
export const HIDDEN_DIRS: readonly string[] = ['.git', 'node_modules', 'target']

/** Splits a relative path into its segments, refusing absolute paths and `..`. */
export function relativeSegments(path: string): string[] {
  if (path.startsWith('/') || path.startsWith('\\') || /^[a-zA-Z]:/.test(path)) {
    throw new Error(`Path must be relative to the repository: ${path}`)
  }
  const parts = path.split(/[\\/]/).filter((p) => p !== '' && p !== '.')
  if (parts.some((p) => p === '..')) throw new Error(`Path leaves the repository: ${path}`)
  return parts
}

const byKindThenName = (a: FileEntry, b: FileEntry) =>
  a.kind === b.kind ? a.name.localeCompare(b.name) : a.kind === 'dir' ? -1 : 1

/**
 * An in-memory file tree (the web demo and tests). Keys are file paths relative to the root; folders are
 * implied by them. One tree serves every `root`.
 */
export function memoryFiles(
  initial: Record<string, string>,
  /** Committed copies by path, for `base`; leave it out for a tree without version control. */
  bases?: Record<string, string>,
): FilesHost {
  const files = new Map<string, string>()
  for (const [path, text] of Object.entries(initial)) files.set(relativeSegments(path).join('/'), text)
  const hasDir = (dir: string) => dir === '' || [...files.keys()].some((p) => p.startsWith(`${dir}/`))

  const committed = bases
    ? new Map(Object.entries(bases).map(([p, t]) => [relativeSegments(p).join('/'), t]))
    : null

  return {
    ...(committed
      ? { base: async (_root: string, path: string) => committed.get(relativeSegments(path).join('/')) ?? null }
      : {}),
    async list(_root, dir) {
      const d = relativeSegments(dir).join('/')
      if (!hasDir(d)) throw new Error(`No such folder: ${dir}`)
      const prefix = d ? `${d}/` : ''
      const seen = new Map<string, FileEntry>()
      for (const p of files.keys()) {
        if (!p.startsWith(prefix)) continue
        const rest = p.slice(prefix.length).split('/')
        const name = rest[0] ?? ''
        if (!name || seen.has(name) || HIDDEN_DIRS.includes(name)) continue
        seen.set(name, { name, path: prefix + name, kind: rest.length > 1 ? 'dir' : 'file' })
      }
      return [...seen.values()].sort(byKindThenName)
    },
    async read(_root, path) {
      const text = files.get(relativeSegments(path).join('/'))
      if (text === undefined) throw new Error(`No such file: ${path}`)
      return text
    },
    async write(_root, path, text) {
      const parts = relativeSegments(path)
      if (parts.length === 0) throw new Error('A file path is required')
      const key = parts.join('/')
      if (hasDir(key)) throw new Error(`Not a file: ${path}`)
      files.set(key, text)
    },
  }
}
