import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { ClerkProvider } from '@clerk/clerk-react';
import { RouterProvider } from '@tanstack/react-router';
import { router } from './router';
import './index.css';

const rootElement = document.getElementById('root');
const env = import.meta.env as Record<string, string | undefined>;
const publishableKey = env.VITE_CLERK_PUBLISHABLE_KEY ?? env.NEXT_PUBLIC_CLERK_PUBLISHABLE_KEY;

if (!rootElement) {
  throw new Error('Root element #root was not found.');
}

if (!publishableKey) {
  createRoot(rootElement).render(
    <StrictMode>
      <div className="grid min-h-screen place-items-center bg-[#070b12] px-6 text-[#dbe4f6]">
        <div className="glass-panel max-w-xl rounded-3xl p-8">
          <p className="text-xs uppercase tracking-[0.24em] text-[#9cb4df]">Auth Setup Needed</p>
          <h1 className="mt-3 font-display text-3xl text-white">Missing Clerk Publishable Key</h1>
          <p className="mt-4 text-sm text-[#b8c7e6]">
            Set <code>VITE_CLERK_PUBLISHABLE_KEY</code> before loading the app.
          </p>
        </div>
      </div>
    </StrictMode>,
  );
} else {
  createRoot(rootElement).render(
    <StrictMode>
      <ClerkProvider publishableKey={publishableKey} afterSignOutUrl="/">
        <RouterProvider router={router} />
      </ClerkProvider>
    </StrictMode>,
  );
}
