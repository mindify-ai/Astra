// The UI package doesn't depend on Node types; this is the one Node API its tests use.
declare module 'node:fs' {
  export function readFileSync(path: URL, encoding: 'utf8'): string
}
