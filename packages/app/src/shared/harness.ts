import type { AgentConnection, State } from '@astra/client'
import { agentOf, missionAgents, roleName } from '../select'

/** Which command-line harness an agent runs on, for its slash commands. */
export type Harness = 'claude' | 'codex' | 'astra'

export interface SlashCommand {
  /** Without the slash: `compact`. */
  name: string
  /** What it takes after the name, shown as a hint: `[instructions]`. */
  args?: string
  description: string
  harness: Harness
}

/**
 * Commands the composer offers after `/`. They are sent as the message text, as typed, and the agent's harness
 * runs them (Claude Code and Codex read a prompt that starts with `/` as a command). Only commands that make
 * sense for an agent working headless in a worktree are listed: no login, terminal or editor setup.
 * `astra` commands are handled here, in the app.
 */
export const slashCommands: SlashCommand[] = [
  // Astra
  {
    name: 'btw',
    args: '<question>',
    description: 'Ask a side question; the agents and the thread carry on',
    harness: 'astra',
  },
  // Claude Code
  {
    name: 'compact',
    args: '[instructions]',
    description: 'Summarise the conversation to free context',
    harness: 'claude',
  },
  { name: 'clear', description: 'Start the conversation over', harness: 'claude' },
  {
    name: 'review',
    args: '[PR]',
    description: 'Review a pull request or the current changes',
    harness: 'claude',
  },
  { name: 'security-review', description: 'Review pending changes for security issues', harness: 'claude' },
  { name: 'init', description: 'Write a CLAUDE.md that documents the codebase', harness: 'claude' },
  {
    name: 'pr-comments',
    args: '[PR]',
    description: 'Fetch the comments on a pull request',
    harness: 'claude',
  },
  { name: 'memory', description: 'Edit the CLAUDE.md memory files', harness: 'claude' },
  { name: 'model', args: '[model]', description: 'Show or change the model', harness: 'claude' },
  { name: 'context', description: 'Show how the context window is used', harness: 'claude' },
  { name: 'cost', description: 'Show token usage for this session', harness: 'claude' },
  { name: 'todos', description: 'List the current to-do items', harness: 'claude' },
  { name: 'agents', description: 'Manage subagents', harness: 'claude' },
  { name: 'mcp', description: 'Show MCP servers and their tools', harness: 'claude' },
  {
    name: 'add-dir',
    args: '<path>',
    description: 'Let the agent work in another directory',
    harness: 'claude',
  },
  { name: 'status', description: 'Show version, model and account', harness: 'claude' },
  // Codex
  { name: 'review', description: 'Review the working tree for problems', harness: 'codex' },
  { name: 'compact', description: 'Summarise the conversation to free context', harness: 'codex' },
  { name: 'new', description: 'Start a new conversation', harness: 'codex' },
  { name: 'init', description: 'Write an AGENTS.md with instructions for Codex', harness: 'codex' },
  { name: 'diff', description: 'Show the git diff, including untracked files', harness: 'codex' },
  { name: 'undo', description: 'Undo the last turn’s changes', harness: 'codex' },
  { name: 'model', args: '[model]', description: 'Choose the model and reasoning effort', harness: 'codex' },
  { name: 'approvals', description: 'Choose what Codex may do without asking', harness: 'codex' },
  { name: 'mention', args: '<path>', description: 'Point Codex at a file', harness: 'codex' },
  { name: 'mcp', description: 'List the configured MCP tools', harness: 'codex' },
  { name: 'status', description: 'Show the session configuration and token usage', harness: 'codex' },
]

export const harnessLabel: Record<Harness, string> = {
  claude: 'CLAUDE CODE',
  codex: 'CODEX',
  astra: 'ASTRA',
}

/** The harness an agent connection runs on, from its id, name and adapter command; null for others. */
export function harnessOf(agent: AgentConnection | undefined): Harness | null {
  if (!agent) return null
  const command = agent.adapter && 'command' in agent.adapter ? agent.adapter.command : ''
  const hay = `${agent.id} ${agent.name} ${command}`.toLowerCase()
  if (hay.includes('codex')) return 'codex'
  if (hay.includes('claude')) return 'claude'
  return null
}

/** The harnesses the given mission agents run on (the target agent's alone when `to` is set). */
export function missionHarnesses(s: State, missionId: string, to: string | null): Set<Harness> {
  const out = new Set<Harness>()
  for (const ma of missionAgents(s, missionId)) {
    if (to && ma.id !== to) continue
    const h = harnessOf(agentOf(s, ma))
    if (h) out.add(h)
  }
  return out
}

/** Slash commands for a set of harnesses: Astra's own first, then each harness's, one row per name+harness. */
export function commandsFor(harnesses: ReadonlySet<Harness>, includeAstra = true): SlashCommand[] {
  return slashCommands.filter((c) => (c.harness === 'astra' ? includeAstra : harnesses.has(c.harness)))
}

export type MentionKind = 'file' | 'agent'

export interface Mention {
  kind: MentionKind
  /** Inserted after `@`. */
  value: string
  /** Shown in the list. */
  label: string
  detail?: string
  /** For agents: the mission agent to address. */
  missionAgentId?: string
}

/** `@` targets for a mission's agents: `@builder` addresses the Builder. */
export function agentMentions(s: State, missionId: string): Mention[] {
  return missionAgents(s, missionId)
    .filter((ma) => ma.enabled)
    .map((ma) => ({
      kind: 'agent' as const,
      value: roleName[ma.role].toLowerCase(),
      label: roleName[ma.role],
      detail: agentOf(s, ma)?.name,
      missionAgentId: ma.id,
    }))
}

const looksLikeFile = (t: string) => /^[\w@.-][\w@./-]*\.[\w]+$/.test(t) && !t.includes('://')

/**
 * Files people can `@`-mention without reading the disk: every path a mission on this repository changed or an
 * agent touched. The composer adds the repository's own files when the host can list them.
 */
export function knownFiles(
  s: Pick<State, 'missions' | 'results' | 'messages' | 'missionAgents'>,
  repoId: string,
): string[] {
  const missions = new Set(
    Object.values(s.missions)
      .filter((m) => m.repoId === repoId)
      .map((m) => m.id),
  )
  const out = new Set<string>()
  for (const r of Object.values(s.results)) {
    if (missions.has(r.missionId)) for (const f of r.diff) out.add(f.path)
  }
  for (const m of Object.values(s.messages)) {
    if (!missions.has(m.missionId)) continue
    for (const l of m.log) if (looksLikeFile(l.target)) out.add(l.target)
  }
  for (const ma of Object.values(s.missionAgents)) {
    if (!missions.has(ma.missionId)) continue
    for (const l of ma.latest) if (looksLikeFile(l.target)) out.add(l.target)
  }
  return [...out].sort()
}

/** Ranks `items` against what was typed: prefix matches first, then substring, then subsequence. */
export function rank<T>(items: T[], query: string, key: (t: T) => string, limit = 8): T[] {
  const q = query.toLowerCase()
  if (!q) return items.slice(0, limit)
  const scored: { t: T; score: number }[] = []
  for (const t of items) {
    const k = key(t).toLowerCase()
    const base = k.split('/').pop() ?? k
    let score = -1
    if (base.startsWith(q)) score = 0
    else if (k.startsWith(q)) score = 1
    else if (k.includes(q)) score = 2
    else {
      let i = 0
      for (const ch of k) if (ch === q[i]) i++
      if (i === q.length) score = 3
    }
    if (score >= 0) scored.push({ t, score })
  }
  return scored
    .sort((a, b) => a.score - b.score || key(a.t).length - key(b.t).length)
    .slice(0, limit)
    .map((x) => x.t)
}

/** The `/` or `@` token being typed at the caret, if any. `/` only counts at the start of the message. */
export function activeToken(
  text: string,
  caret: number,
): { trigger: '/' | '@'; query: string; start: number } | null {
  const before = text.slice(0, caret)
  const slash = /^\/([\w-]*)$/.exec(before)
  if (slash) return { trigger: '/', query: slash[1] ?? '', start: 0 }
  const at = /(^|\s)@([\w@./-]*)$/.exec(before)
  if (at) {
    const query = at[2] ?? ''
    return { trigger: '@', query, start: caret - query.length - 1 }
  }
  return null
}

/** A message that is an Astra `/btw` side question: its question, or null. */
export function sideQuestion(text: string): string | null {
  const m = /^\/btw(?:\s+([\s\S]*))?$/.exec(text.trim())
  return m ? (m[1] ?? '').trim() : null
}
