// Screenshots every route on desktop (each OS title bar) and phones, for design review.
// Usage: pnpm --filter @astra/web build && node scripts/screenshots.mjs [outDir] [baseUrl]

import { execSync } from 'node:child_process'
import { mkdirSync } from 'node:fs'
import { createRequire } from 'node:module'

// Use the project's Playwright if installed, otherwise the machine-wide one.
async function loadPlaywright() {
  try {
    return await import('playwright')
  } catch {
    const globalRoot = execSync('npm root -g').toString().trim()
    return createRequire(`${globalRoot}/`)('playwright')
  }
}
const { chromium } = await loadPlaywright()

const out = process.argv[2] ?? 'screenshots'
const base = process.argv[3] ?? 'http://127.0.0.1:4173'
mkdirSync(out, { recursive: true })

const desktop = [
  ['01-welcome', '/welcome'],
  ['02-repo', '/setup/repo'],
  ['03-connect', '/setup/agents'],
  ['04-home', '/'],
  ['05-launch', '/missions/new'],
  ['06-thread', '/missions/m-014'],
  ['07-review', '/missions/m-015/review'],
  ['08-merged', '/missions/m-013/merged'],
  ['09-map', '/missions/m-014/map'],
  ['10-board', '/board'],
  ['11-git', '/git'],
]
const phone = [
  ['home', '/'],
  ['git', '/git'],
  ['thread', '/missions/m-014'],
  ['review', '/missions/m-015/review'],
]

const browser = await chromium.launch()
const scale = Number(process.env.SCALE ?? 1)
for (const platform of process.env.PLATFORMS?.split(',') ?? ['mac']) {
  const page = await browser.newPage({ viewport: { width: 1440, height: 960 }, deviceScaleFactor: scale })
  for (const [name, path] of desktop) {
    await page.goto(`${base}${path}?platform=${platform}&instant`)
    await page.waitForLoadState('networkidle')
    await page.screenshot({ path: `${out}/desktop-${platform}-${name}.png`, fullPage: true })
  }
  await page.goto(`${base}/missions/m-014?platform=${platform}&instant`)
  await page.getByRole('button', { name: 'SHARE' }).click()
  await page.screenshot({ path: `${out}/desktop-${platform}-11-people.png` })
  await page.close()
}
for (const platform of ['ios', 'android']) {
  const page = await browser.newPage({ viewport: { width: 390, height: 844 }, deviceScaleFactor: scale })
  for (const [name, path] of phone) {
    await page.goto(`${base}${path}?platform=${platform}&instant`)
    await page.waitForLoadState('networkidle')
    await page.screenshot({ path: `${out}/${platform}-${name}.png` })
  }
  await page.close()
}
await browser.close()
console.log(`screenshots → ${out}`)
