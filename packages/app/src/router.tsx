import {
  createHashHistory,
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  type RouterHistory,
} from '@tanstack/react-router'
import { useAstraState, usePlatform } from './context'
import { MobileAgents, MobileHome, MobilePeople, MobileReview, MobileThread } from './mobile/MobileScreens'
import { Board } from './screens/Board'
import { Connect } from './screens/Connect'
import { Home } from './screens/Home'
import { Launch } from './screens/Launch'
import { MapView } from './screens/Map'
import { Merged } from './screens/Merged'
import { PeopleDialog } from './screens/People'
import { Repo } from './screens/Repo'
import { Review } from './screens/Review'
import { Thread } from './screens/Thread'
import { Welcome } from './screens/Welcome'
import { CommandPalette } from './shell/CommandPalette'
import { WithTerminal } from './shell/Terminal'

const rootRoute = createRootRoute({
  component: () => (
    <>
      <WithTerminal>
        <Outlet />
      </WithTerminal>
      <PeopleDialog />
      <CommandPalette />
    </>
  ),
})

/** Renders the phone screen on phones (and narrow web), the desktop screen otherwise. */
function Adaptive({ desktop, mobile }: { desktop: React.ReactNode; mobile: React.ReactNode }) {
  const { layout } = usePlatform()
  return <>{layout === 'mobile' ? mobile : desktop}</>
}

/** A workspace with no repository yet (a first launch) starts at Welcome rather than an empty Home. */
function DesktopHome() {
  const s = useAstraState()
  return Object.keys(s.repositories).length === 0 ? <Welcome /> : <Home />
}

const homeRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/',
  component: () => <Adaptive desktop={<DesktopHome />} mobile={<MobileHome />} />,
})
const welcomeRoute = createRoute({ getParentRoute: () => rootRoute, path: '/welcome', component: Welcome })
const repoRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/setup/repo',
  validateSearch: (search: Record<string, unknown>): { source?: 'github' | 'gitlab' | 'local' } =>
    search.source === 'github' || search.source === 'gitlab' || search.source === 'local'
      ? { source: search.source }
      : {},
  component: Repo,
})
const connectRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/setup/agents',
  component: Connect,
})
const launchRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/missions/new',
  validateSearch: (search: Record<string, unknown>): { goal?: string } =>
    typeof search.goal === 'string' ? { goal: search.goal } : {},
  component: function LaunchRoute() {
    const { goal } = launchRoute.useSearch()
    return <Launch initialGoal={goal ?? ''} />
  },
})
const threadRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/missions/$missionId',
  component: function ThreadRoute() {
    const { missionId } = threadRoute.useParams()
    return (
      <Adaptive desktop={<Thread missionId={missionId} />} mobile={<MobileThread missionId={missionId} />} />
    )
  },
})
const mapRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/missions/$missionId/map',
  component: function MapRoute() {
    const { missionId } = mapRoute.useParams()
    return <MapView missionId={missionId} />
  },
})
const reviewRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/missions/$missionId/review',
  component: function ReviewRoute() {
    const { missionId } = reviewRoute.useParams()
    return (
      <Adaptive desktop={<Review missionId={missionId} />} mobile={<MobileReview missionId={missionId} />} />
    )
  },
})
const mergedRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/missions/$missionId/merged',
  component: function MergedRoute() {
    const { missionId } = mergedRoute.useParams()
    return <Merged missionId={missionId} />
  },
})
const boardRoute = createRoute({ getParentRoute: () => rootRoute, path: '/board', component: Board })
const agentsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/agents',
  component: () => <Adaptive desktop={<Connect />} mobile={<MobileAgents />} />,
})
const peopleRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/people',
  component: () => <Adaptive desktop={<Board />} mobile={<MobilePeople />} />,
})

const routeTree = rootRoute.addChildren([
  homeRoute,
  welcomeRoute,
  repoRoute,
  connectRoute,
  launchRoute,
  threadRoute,
  mapRoute,
  reviewRoute,
  mergedRoute,
  boardRoute,
  agentsRoute,
  peopleRoute,
])

export type HistoryKind = 'browser' | 'hash' | { memory: string }

export function createAstraRouter(history: HistoryKind = 'browser') {
  let h: RouterHistory | undefined
  if (history === 'hash') h = createHashHistory()
  else if (typeof history === 'object') h = createMemoryHistory({ initialEntries: [history.memory] })
  return createRouter({ routeTree, history: h, defaultPreload: 'intent', scrollRestoration: true })
}

declare module '@tanstack/react-router' {
  interface Register {
    router: ReturnType<typeof createAstraRouter>
  }
}
