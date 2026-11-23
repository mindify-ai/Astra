/**
 * Hybrid logical clock. Formatted as "<ms 16 digits>:<counter 6 digits>:<nodeId>" so that
 * plain string comparison gives causal order (ties broken by node id).
 */
export function formatHlc(ms: number, counter: number, node: string): string {
  return `${String(ms).padStart(16, '0')}:${String(counter).padStart(6, '0')}:${node}`
}

export function parseHlc(hlc: string): { ms: number; counter: number; node: string } {
  const [ms = '0', counter = '0', ...node] = hlc.split(':')
  return { ms: Number(ms), counter: Number(counter), node: node.join(':') }
}

export class HlcClock {
  private ms = 0
  private counter = 0
  constructor(
    readonly node: string,
    private readonly now: () => number = Date.now,
  ) {}

  /** A new timestamp, strictly greater than every timestamp issued or observed. */
  tick(): string {
    const wall = this.now()
    if (wall > this.ms) {
      this.ms = wall
      this.counter = 0
    } else {
      this.counter += 1
    }
    return formatHlc(this.ms, this.counter, this.node)
  }

  /** Advance past a remote timestamp. */
  observe(hlc: string): void {
    const r = parseHlc(hlc)
    if (r.ms > this.ms || (r.ms === this.ms && r.counter > this.counter)) {
      this.ms = r.ms
      this.counter = r.counter
    }
  }
}

const CROCKFORD = '0123456789ABCDEFGHJKMNPQRSTVWXYZ'

/** A ULID: 48-bit time + 80 random bits, Crockford base32. */
export function ulid(now: number = Date.now()): string {
  let time = ''
  let t = now
  for (let i = 0; i < 10; i++) {
    time = CROCKFORD[t % 32] + time
    t = Math.floor(t / 32)
  }
  const bytes = new Uint8Array(16)
  crypto.getRandomValues(bytes)
  let rand = ''
  for (let i = 0; i < 16; i++) rand += CROCKFORD[(bytes[i] ?? 0) % 32]
  return time + rand
}
