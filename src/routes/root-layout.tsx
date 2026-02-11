import { Outlet, useRouterState } from '@tanstack/react-router';

export function RootLayout() {
  const pathname = useRouterState({
    select: (state) => state.location.pathname,
  });
  const isRoomRoute = pathname.startsWith('/room/');

  if (isRoomRoute) {
    return (
      <div className="h-dvh min-h-0 overflow-hidden bg-[#04070c] text-white">
        <Outlet />
      </div>
    );
  }

  return (
    <div className="relative min-h-screen overflow-hidden bg-[#04070c] text-[#edf3ff]">
      <div className="mesh-orb mesh-orb-a" />
      <div className="mesh-orb mesh-orb-b" />
      <div className="scanlines" />
      <main className="relative mx-auto w-full max-w-[1180px] px-6 py-8 md:px-10 md:py-12">
        <Outlet />
      </main>
    </div>
  );
}
