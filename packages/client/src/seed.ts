import seed from '../../../fixtures/seed.json'
import starter from '../../../fixtures/starter.json'
import type { SeedFile } from './store'

/** The demo workspace from the design (missions 013–015, four agents, three people). */
export const demoSeed = seed as SeedFile

/** A clean first launch: only you and the catalog of agents you can connect. The desktop node uses the same file. */
export const starterSeed = starter as SeedFile
