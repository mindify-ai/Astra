// End-to-end test of the whole stack, with no mocks:
//   browser (hosted web build) → astra-server relay → astrad runner → real git worktrees → merge.
// Signs in with a dev token, adds a repository and three command-line agents, starts a race from Home,
// merges the reviewer's pick on the Review screen, then checks the git repository.
//
// Prereqs: `cargo build -p astra-server -p astrad` and `pnpm install`. Playwright (project or global).
// Usage: node scripts/e2e-stack.mjs [--keep]   (--keep leaves the temp dir for inspection)
import { execFileSync, execSync, spawn } from 'node:child_process'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

const root = resolve(import.meta.dirname, '..')
const tmp = mkdtempSync(join(tmpdir(), 'astra-e2e-'))
const RELAY = 'http://127.0.0.1:8787'
const WEB = 'http://127.0.0.1:4174'
const children = []
const keep = process.argv.includes('--keep')

async function loadPlaywright() {
  try {
    return await import('playwright')
  } catch {
    return createRequire(`${execSync('npm root -g').toString().trim()}/`)('playwright')
  }
}

function start(name, cmd, args, env = {}) {
  const child = spawn(cmd, args, {
    cwd: root,
    env: { ...process.env, ...env },
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  const log = []
  child.stdout.on('data', (d) => log.push(String(d)))
  child.stderr.on('data', (d) => log.push(String(d)))
  children.push({ name, child, log })
  return child
}

async function waitFor(url, what) {
  for (let i = 0; i < 120; i++) {
    try {
      if ((await fetch(url)).ok) return
    } catch {}
    await new Promise((r) => setTimeout(r, 250))
  }
  throw new Error(`${what} did not start (${url})`)
}

function check(cond, msg) {
  if (!cond) throw new Error(`FAILED: ${msg}`)
  console.log(`  ✓ ${msg}`)
}

async function main() {
  // A repository the agents will work on.
  const repo = join(tmp, 'repo')
  execFileSync('git', ['init', '-q', '-b', 'main', repo])
  writeFileSync(join(repo, 'README.md'), '# pricing\n')
  execFileSync('git', ['-C', repo, 'add', '.'])
  execFileSync('git', [
    '-C',
    repo,
    '-c',
    'user.name=e2e',
    '-c',
    'user.email=e2e@example.com',
    'commit',
    '-qm',
    'init',
  ])

  // Relay (in-memory backend, dev sign-in).
  start('astra-server', join(root, 'target/debug/astra-server'), [], {
    ASTRA_DEV_AUTH: '1',
    PORT: '8787',
    ASTRA_ALLOWED_ORIGINS: WEB,
    RUST_LOG: 'warn',
  })
  await waitFor(`${RELAY}/healthz`, 'astra-server')
  const res = await fetch(`${RELAY}/v1/auth/dev-token`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ name: 'Mark' }),
  })
  const { session } = await res.json()

  // Runner node, signed in as the same person.
  start(
    'astrad',
    join(root, 'target/debug/astrad'),
    [
      'run',
      '--data-dir',
      join(tmp, 'node'),
      '--node-id',
      'n-desk',
      '--workspace',
      session.workspaceId,
      '--me',
      session.person.id,
      '--relay-url',
      RELAY,
      '--no-mdns',
    ],
    { ASTRA_TOKEN: session.token, RUST_LOG: 'warn' },
  )

  // Hosted web build pointed at the relay.
  const out = join(tmp, 'web')
  execSync(`pnpm --filter @astra/web exec vite build --outDir ${out}`, {
    cwd: root,
    env: { ...process.env, VITE_ASTRA_RELAY_URL: RELAY, VITE_ASTRA_DEV_AUTH: '1' },
    stdio: 'ignore',
  })
  start('web', 'pnpm', [
    '--filter',
    '@astra/web',
    'exec',
    'vite',
    'preview',
    '--outDir',
    out,
    '--port',
    '4174',
    '--strictPort',
  ])
  await waitFor(WEB, 'web preview')

  const { chromium } = await loadPlaywright()
  const browser = await chromium.launch()
  try {
    const page = await browser.newPage({ viewport: { width: 1440, height: 960 } })
    await page.addInitScript(() => localStorage.setItem('astra.repo', 'r-e2e'))
    await page.goto(`${WEB}/?platform=mac`)
    await page.getByLabel('Developer name').fill('Mark')
    await page.getByRole('button', { name: 'Dev sign-in' }).click()
    await page.getByRole('heading', { name: 'Start a mission.' }).waitFor()
    check(true, 'signed in through the relay')

    const agent = (id, name, script) => ({
      type: 'put',
      entity: {
        kind: 'agent',
        id,
        name,
        vendor: 'E2E',
        category: 'codingAgent',
        transport: 'CLI on this machine',
        adapter: { type: 'command', command: 'sh', args: ['-c', `cat >/dev/null; ${script}`] },
        runsOn: 'local',
        nodeId: 'n-desk',
        commandPolicy: 'auto',
        status: 'connected',
        requiresNetwork: false,
      },
    })
    await page.evaluate(
      (bodies) => window.astra.submit(bodies),
      [
        {
          type: 'put',
          entity: {
            kind: 'repository',
            id: 'r-e2e',
            name: 'e2e/pricing',
            source: 'local',
            defaultBranch: 'main',
            path: repo,
            remoteUrl: null,
          },
        },
        // Named like the default team so Home assigns two builders and a reviewer.
        agent(
          'a-one',
          'Codex',
          'printf "export const ttl = 60\\n" > cache.ts; echo "Cached the endpoint with a 60s TTL."',
        ),
        agent(
          'a-two',
          'Claude Code',
          'mkdir -p lib; printf "export const cached = 1\\n" > lib/cached.ts; echo "Added a reusable cache helper."',
        ),
        agent('a-rev', 'Gemini CLI', 'echo "Both work. A is smaller. Pick: A"'),
      ],
    )
    await page.reload()
    await page.getByText('Agents · 3 connected').waitFor()
    check(true, 'repository and agents synced')

    await page.getByLabel('Goal').fill('Cache the pricing endpoint.')
    await page.getByLabel('How the agents work').selectOption('race')
    await page.getByRole('button', { name: /start mission/i }).click()
    await page.getByRole('list', { name: 'Thread' }).waitFor()
    const missionId = decodeURIComponent(new URL(page.url()).pathname.split('/')[2])

    let results = []
    for (let i = 0; i < 120 && !(results.length >= 2 && results.some((r) => r.reviewerPick)); i++) {
      await page.waitForTimeout(500)
      results = await page.evaluate(
        (id) => Object.values(window.astra.getState().results).filter((r) => r.missionId === id),
        missionId,
      )
    }
    check(results.length === 2, 'the runner raced two builders in separate worktrees')
    check(results.find((r) => r.label === 'A')?.reviewerPick === true, 'the reviewer picked A')

    await page.goto(`${WEB}/missions/${missionId}/review?platform=mac`)
    await page.getByRole('button', { name: 'Merge A' }).click()
    await page.getByRole('heading', { name: 'Mission complete.' }).waitFor({ timeout: 30000 })
    const merge = await page.evaluate((id) => window.astra.getState().merges[id], missionId)
    check(merge?.approvedBy === session.person.id, 'merge recorded, approved by the signed-in person')

    const files = execFileSync('git', ['-C', repo, 'ls-files'], { encoding: 'utf8' }).split('\n')
    check(files.includes('cache.ts') && !files.includes('lib/cached.ts'), "A's change is on main, B's is not")
    const branches = execFileSync('git', ['-C', repo, 'branch', '--format=%(refname:short)'], {
      encoding: 'utf8',
    })
    check(branches.includes(merge.keptBranches[0]), `B is kept on ${merge.keptBranches[0]}`)
  } finally {
    await browser.close()
  }
}

let failed = false
try {
  await main()
  console.log('e2e: passed')
} catch (e) {
  failed = true
  console.error(String(e))
  for (const c of children) console.error(`--- ${c.name} log (tail)\n${c.log.join('').slice(-2000)}`)
} finally {
  for (const c of children) c.child.kill()
  if (!keep) rmSync(tmp, { recursive: true, force: true })
  else console.log(`kept ${tmp}`)
}
process.exit(failed ? 1 : 0)
