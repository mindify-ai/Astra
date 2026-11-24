/** A shell session in the in-app terminal. */
export interface TerminalSession {
  write(data: string): void
  resize(cols: number, rows: number): void
  close(): void
}

export interface TerminalOpenOptions {
  /** Starting folder; the home folder when null or missing. */
  cwd: string | null
  cols: number
  rows: number
  onData: (data: string) => void
  onExit: (code: number | null) => void
}

/** Starts shells on this machine. Provided by the desktop shell; absent on the web and phones. */
export interface TerminalHost {
  open(options: TerminalOpenOptions): Promise<TerminalSession>
}
