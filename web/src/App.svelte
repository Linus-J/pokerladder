<script lang="ts">
  import { Info, LogOut, RotateCw, ScrollText, Spade, TriangleAlert, Trophy } from '@lucide/svelte'
  import { onMount } from 'svelte'
  import Icon from './components/Icon.svelte'
  import { game, session } from './lib/game.svelte'
  import { link, router } from './lib/router.svelte'
  import About from './pages/About.svelte'
  import ForgotPassword from './pages/ForgotPassword.svelte'
  import History from './pages/History.svelte'
  import Leaderboard from './pages/Leaderboard.svelte'
  import Lobby from './pages/Lobby.svelte'
  import Login from './pages/Login.svelte'
  import Play from './pages/Play.svelte'
  import Privacy from './pages/Privacy.svelte'
  import Register from './pages/Register.svelte'
  import ResetPassword from './pages/ResetPassword.svelte'
  import VerifyEmail from './pages/VerifyEmail.svelte'

  let loadError = $state(false)
  onMount(() => {
    session.load().catch(() => (loadError = true))
  })

  // Keep the game connection open while logged in.
  $effect(() => {
    if (session.me) game.start()
    else game.stop()
  })

  const PROTECTED = ['/play', '/history']
  const path = $derived(router.path)
  $effect(() => {
    if (!session.loaded) return
    const needsLogin = PROTECTED.some((p) => path === p || path.startsWith(p + '/'))
    if (needsLogin && !session.me) router.go('/login', true)
    if ((path === '/login' || path === '/register') && session.me) router.go('/', true)
  })

  const TITLES: Record<string, string> = {
    '/login': 'Log in',
    '/register': 'Create an account',
    '/about': 'About',
    '/leaderboard': 'Leaderboard',
    '/privacy': 'Privacy',
    '/play': 'Table',
    '/forgot-password': 'Reset your password',
    '/reset-password': 'Choose a new password',
    '/verify-email': 'Confirm your email',
  }
  $effect(() => {
    const t = path.startsWith('/history') ? 'Your matches' : TITLES[path]
    document.title = t ? `${t} | Pokerladder` : "Pokerladder | Heads-up hold'em ladder"
  })

  const inMatch = $derived(game.table !== null && game.table.winner === null)
  const here = (p: string) => (path === p || path.startsWith(p + '/') ? 'page' : undefined)

  async function logout() {
    await session.logout()
    router.go('/')
  }
</script>

<header class="masthead">
  <div class="wrap bar">
    <a class="brand" href="/" onclick={link}>
      <Icon of={Spade} />
      <span>Pokerladder</span>
    </a>
    <nav aria-label="Main">
      <a href="/" onclick={link} aria-current={path === '/' ? 'page' : undefined}>Lobby</a>
      <a href="/leaderboard" onclick={link} aria-current={here('/leaderboard')}>
        <Icon of={Trophy} />Leaderboard
      </a>
      {#if session.me}
        <a href="/history" onclick={link} aria-current={here('/history')}><Icon of={ScrollText} />Your matches</a>
      {/if}
      <a href="/about" onclick={link} aria-current={here('/about')}><Icon of={Info} />About</a>
    </nav>
  </div>
  <div class="userbar">
    <div class="wrap userbar-inner">
      {#if session.me}
        <span>
          Logged in as <b>{session.me.name}</b>, rating <b class="num">{Math.round(session.me.rating)}</b>
        </span>
        {#if inMatch}
          <a class="live" href="/play" onclick={link}>You're in a match: back to the table</a>
        {/if}
        <button type="button" class="link logout" onclick={logout}><Icon of={LogOut} />Log out</button>
      {:else if session.loaded}
        <span class="muted">You're not logged in.</span>
        <span class="auth-links">
          <a href="/login" onclick={link}>Log in</a>
          <span class="muted">or</span>
          <a href="/register" onclick={link}>create an account</a>
        </span>
      {/if}
    </div>
  </div>
</header>

{#if game.status === 'replaced'}
  <div class="banner">
    <div class="wrap">
      <Icon of={TriangleAlert} />
      <span>
        Pokerladder is open in another tab or on another device, so this one has stopped updating.
        <button type="button" class="link" onclick={() => game.reclaim()}>Play here instead</button>
      </span>
    </div>
  </div>
{:else if session.me && game.status === 'connecting'}
  <div class="banner quiet">
    <div class="wrap">
      <Icon of={RotateCw} spin />
      {game.everOnline ? 'Lost the connection to the game server. Reconnecting...' : 'Connecting to the game server...'}
    </div>
  </div>
{:else if game.restarting}
  <div class="banner" data-testid="restarting">
    <div class="wrap">
      <Icon of={RotateCw} />
      <span>
        The server is restarting for an update. Matches already running will finish as normal. New
        matches can start again in a few minutes.
      </span>
    </div>
  </div>
{/if}
{#if game.notice}<div class="toast" role="status">{game.notice}</div>{/if}

<main class="wrap">
  {#if loadError}
    <div class="alert" role="alert">
      <Icon of={TriangleAlert} />
      <span>Pokerladder can't reach its server right now. Check your connection, then reload the page.</span>
    </div>
  {:else if !session.loaded}
    <p class="muted loading">Loading Pokerladder...</p>
  {:else if path === '/login'}
    <Login />
  {:else if path === '/register'}
    <Register />
  {:else if path === '/verify-email'}
    <VerifyEmail />
  {:else if path === '/forgot-password'}
    <ForgotPassword />
  {:else if path === '/reset-password'}
    <ResetPassword />
  {:else if path === '/privacy'}
    <Privacy />
  {:else if path === '/about'}
    <About />
  {:else if path === '/leaderboard'}
    <Leaderboard />
  {:else if path === '/play' && session.me}
    <Play />
  {:else if path.startsWith('/history') && session.me}
    <History />
  {:else}
    <Lobby />
  {/if}
</main>

<footer>
  <div class="wrap foot">
    <span>Pokerladder. Heads-up No-Limit Hold'em, free to enter, prizes paid by sponsors.</span>
    <span class="foot-links">
      <a href="/about" onclick={link}>About</a>
      <a href="/about#fair-play" onclick={link}>Fair play</a>
      <a href="/privacy" onclick={link}>Privacy</a>
    </span>
  </div>
</footer>

<style>
  :global(.wrap) {
    max-width: 72rem;
    margin: 0 auto;
    padding-left: var(--s5);
    padding-right: var(--s5);
  }
  .masthead {
    background: var(--bar);
    color: var(--bar-ink);
  }
  .bar {
    display: flex;
    align-items: center;
    gap: var(--s5);
    min-height: 3.25rem;
  }
  .brand {
    display: inline-flex;
    align-items: center;
    gap: var(--s2);
    font-family: var(--serif);
    font-size: var(--t-xl);
    font-weight: 700;
    color: var(--bar-ink);
    text-decoration: none;
  }
  .brand:hover {
    color: var(--bar-ink);
  }
  nav {
    display: flex;
    flex-wrap: wrap;
    margin-left: auto;
    align-self: stretch;
  }
  nav a {
    display: inline-flex;
    align-items: center;
    gap: var(--s2);
    padding: 0 var(--s3);
    color: var(--bar-muted);
    text-decoration: none;
    font-weight: 700;
    border-bottom: 3px solid transparent;
  }
  nav a:hover {
    color: var(--bar-ink);
    border-bottom-color: var(--bar-muted);
  }
  nav a[aria-current='page'] {
    color: var(--bar-ink);
    border-bottom-color: var(--bar-ink);
  }
  .userbar {
    background: var(--sunk);
    color: var(--ink);
    border-bottom: 1px solid var(--line);
    font-size: var(--t-sm);
  }
  .userbar-inner {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s2) var(--s5);
    min-height: 2.25rem;
  }
  .live {
    font-weight: 700;
  }
  .logout,
  .auth-links {
    margin-left: auto;
  }
  .logout {
    display: inline-flex;
    align-items: center;
    gap: var(--s1);
  }
  main {
    padding-top: var(--s5);
    padding-bottom: var(--s7);
    min-height: 60vh;
  }
  .banner {
    background: var(--note-bg);
    border-bottom: 1px solid var(--note-line);
    font-size: var(--t-sm);
  }
  .banner .wrap {
    display: flex;
    align-items: center;
    gap: var(--s2);
    padding-top: var(--s2);
    padding-bottom: var(--s2);
  }
  .banner.quiet {
    background: var(--sunk);
    border-color: var(--line);
    color: var(--muted);
  }
  .toast {
    position: fixed;
    bottom: var(--s5);
    left: var(--s5);
    max-width: 26rem;
    background: var(--surface);
    border: 1px solid var(--line-strong);
    border-left: 4px solid var(--brand);
    padding: var(--s3) var(--s4);
    border-radius: var(--radius);
    z-index: 20;
  }
  .loading {
    padding: var(--s6) 0;
  }
  footer {
    border-top: 1px solid var(--line);
    background: var(--sunk);
    font-size: var(--t-sm);
    color: var(--muted);
  }
  .foot {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s2) var(--s5);
    padding-top: var(--s4);
    padding-bottom: var(--s4);
  }
  .foot-links {
    display: flex;
    gap: var(--s4);
    margin-left: auto;
  }
  @media (max-width: 720px) {
    :global(.wrap) {
      padding-left: var(--s4);
      padding-right: var(--s4);
    }
    .bar {
      flex-wrap: wrap;
      gap: 0;
      padding-top: var(--s2);
    }
    nav {
      margin-left: 0;
      width: 100%;
    }
    nav a {
      padding: var(--s2) var(--s4) var(--s2) 0;
    }
    /* Text alone fits on one line at phone width. */
    nav a :global(svg) {
      display: none;
    }
    .logout,
    .auth-links,
    .foot-links {
      margin-left: 0;
    }
  }
</style>
