import { useState } from 'react';
import type { FormEvent } from 'react';
import { useNavigate } from '@tanstack/react-router';
import {
  SignedIn,
  SignedOut,
  SignInButton,
  SignUpButton,
  UserButton,
  useUser,
} from '@clerk/clerk-react';

function normalizeRoomCode(value: string) {
  return value.trim().toUpperCase().replace(/[^A-Z0-9_-]/g, '').slice(0, 24);
}

function displayNameForUser(userName: string | null, firstName: string | null, userId: string) {
  if (userName) {
    return userName;
  }

  if (firstName) {
    return firstName;
  }

  return userId.slice(0, 16);
}

export function ConnectRoute() {
  const [roomCode, setRoomCode] = useState('');
  const navigate = useNavigate();
  const { user } = useUser();

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();

    const normalized = normalizeRoomCode(roomCode);
    if (!normalized) {
      return;
    }

    void navigate({
      to: '/room/$roomCode',
      params: { roomCode: normalized },
    });
  };

  return (
    <section className="grid gap-8 lg:grid-cols-[1.15fr_0.85fr] lg:items-stretch">
      <article className="glass-panel relative overflow-hidden rounded-[2rem] p-8 sm:p-10">
        <div className="absolute -left-12 top-10 h-40 w-40 rounded-full bg-[#3ac7ff]/25 blur-3xl" />
        <div className="absolute -right-6 bottom-0 h-44 w-44 rounded-full bg-[#52ff8f]/15 blur-3xl" />

        <p className="hud-pill w-fit">Ralph Island // Colony Sim Prototype</p>
        <h1 className="chromatic-title mt-6 max-w-2xl font-display text-4xl leading-[1.05] tracking-tight text-white sm:text-5xl lg:text-6xl">
          Build Together In The Same World, In Real Time.
        </h1>

        <p className="mt-6 max-w-xl text-base leading-relaxed text-[#c8d5ef] sm:text-lg">
          Each room code maps to one Cloudflare Durable Object backed by SQLite. Same code means same shared
          simulation space, persistent player identity, and instant multiplayer.
        </p>

        <div className="mt-10 grid gap-3 text-sm text-[#adc0e2] sm:grid-cols-3">
          <div className="rounded-xl border border-white/10 bg-[#0e1526]/70 px-4 py-3">
            <p className="font-display text-lg text-white">Bevy</p>
            <p>WASM runtime</p>
          </div>
          <div className="rounded-xl border border-white/10 bg-[#0e1526]/70 px-4 py-3">
            <p className="font-display text-lg text-white">Durable Objects</p>
            <p>Room authority</p>
          </div>
          <div className="rounded-xl border border-white/10 bg-[#0e1526]/70 px-4 py-3">
            <p className="font-display text-lg text-white">SQLite</p>
            <p>Fast state persistence</p>
          </div>
        </div>
      </article>

      <aside className="glass-panel rounded-[2rem] p-7 sm:p-8">
        <SignedOut>
          <p className="hud-pill w-fit">Authentication Required</p>
          <h2 className="mt-5 font-display text-3xl text-white">Sign In To Enter A Room</h2>
          <p className="mt-4 text-sm leading-relaxed text-[#b8c7e6]">
            Your Clerk user ID is used as your multiplayer identity, so reconnecting keeps your persistent player
            data.
          </p>

          <div className="mt-7 flex flex-col gap-3">
            <SignInButton mode="modal">
              <button className="btn-neon">Sign In</button>
            </SignInButton>
            <SignUpButton mode="modal">
              <button className="btn-ghost">Create Account</button>
            </SignUpButton>
          </div>
        </SignedOut>

        <SignedIn>
          <div className="flex items-start justify-between gap-4">
            <div>
              <p className="hud-pill w-fit">Mission Control</p>
              <h2 className="mt-5 font-display text-3xl text-white">Join A Multiplayer Room</h2>
            </div>
            <UserButton afterSignOutUrl="/" />
          </div>

          <p className="mt-4 text-sm leading-relaxed text-[#b8c7e6]">
            Signed in as{' '}
            <span className="font-semibold text-white">
              {user ? displayNameForUser(user.username, user.firstName, user.id) : 'Unknown'}
            </span>
            .
          </p>

          <form className="mt-8 flex flex-col gap-3" onSubmit={submit}>
            <label className="text-xs font-semibold uppercase tracking-[0.24em] text-[#9db3db]" htmlFor="roomCode">
              Room Code
            </label>
            <input
              id="roomCode"
              className="h-12 rounded-xl border border-[#2f3f61] bg-[#0a111f] px-4 font-mono text-lg tracking-[0.08em] text-white outline-none transition focus:border-[#67f0c1] focus:ring-2 focus:ring-[#67f0c1]/30"
              placeholder="OMEGA-01"
              value={roomCode}
              onChange={(event) => setRoomCode(event.target.value)}
              autoComplete="off"
              autoCapitalize="characters"
              spellCheck={false}
            />
            <button type="submit" className="btn-neon mt-1">
              Enter Room
            </button>
          </form>

          <p className="mt-5 text-xs text-[#8ea3ca]">
            Tip: open two tabs using the same code to verify multiplayer sync instantly.
          </p>
        </SignedIn>
      </aside>
    </section>
  );
}
