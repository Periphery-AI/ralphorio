import {
  createRootRoute,
  createRoute,
  createRouter,
} from '@tanstack/react-router';
import { ConnectRoute } from './routes/connect-route';
import { RoomRoute } from './routes/room-route';
import { RootLayout } from './routes/root-layout';

const rootRoute = createRootRoute({
  component: RootLayout,
});

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/',
  component: ConnectRoute,
});

const roomRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/room/$roomCode',
  component: RoomRoute,
});

const routeTree = rootRoute.addChildren([indexRoute, roomRoute]);

export const router = createRouter({
  routeTree,
  defaultPreload: 'intent',
});

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router;
  }
}
