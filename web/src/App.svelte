<script lang="ts">
  import { onMount } from 'svelte'
  import { game, session } from './lib/game.svelte'
  import { link, router } from './lib/router.svelte'
  import History from './pages/History.svelte'
  import Lobby from './pages/Lobby.svelte'
  import Login from './pages/Login.svelte'
  import Play from './pages/Play.svelte'
  import Privacy from './pages/Privacy.svelte'
  import Register from './pages/Register.svelte'
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

  async function logout() {
    await session.logout()
    router.go('/')
  }
</script>

<header class="top">
  <a class="brand" href="/" onclick={link}><span class="suit">♠</span> Pokerladder</a>
  <nav>
    {#if session.me}
      {#if game.table && game.table.winner === null}
        <a href="/play" onclick={link} class="live">● Live match</a>
      {/if}
      <a href="/history" onclick={link}>History</a>
      <span class="me">{session.me.name} <span class="num">{Math.round(session.me.rating)}</span></span>
      <button type="button" class="link" onclick={logout}>Log out</button>
    {:else if session.loaded}
      <a href="/login" onclick={link}>Log in</a>
      <a href="/register" onclick={link}>Sign up</a>
    {/if}
  </nav>
</header>

{#if game.status === 'replaced'}
  <div class="banner">
    You opened Pokerladder in another tab or device.
    <button type="button" class="link" onclick={() => game.reclaim()}>Use this one instead</button>
  </div>
{:else if session.me && game.status === 'connecting'}
  <div class="banner subtle">Reconnecting…</div>
{/if}
{#if game.notice}<div class="toast" role="status">{game.notice}</div>{/if}

<main>
  {#if loadError}
    <p class="error center">Can't reach the server. Reload to try again.</p>
  {:else if !session.loaded}
    <p class="muted center">Loading…</p>
  {:else if path === '/login'}
    <Login />
  {:else if path === '/register'}
    <Register />
  {:else if path === '/verify-email'}
    <VerifyEmail />
  {:else if path === '/privacy'}
    <Privacy />
  {:else if path === '/play' && session.me}
    <Play />
  {:else if path.startsWith('/history') && session.me}
    <History />
  {:else}
    <Lobby />
  {/if}
</main>

<footer>
  <a href="/privacy" onclick={link}>Privacy</a>
  <span class="muted">Free to enter · sponsor-funded prizes</span>
</footer>

<style>
  .top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.8rem 1.5rem;
    border-bottom: 1px solid var(--line);
    background: rgba(15, 20, 18, 0.9);
    position: sticky;
    top: 0;
    z-index: 5;
    backdrop-filter: blur(6px);
  }
  .brand {
    font-weight: 800;
    font-size: 1.15rem;
    color: var(--text);
    letter-spacing: -0.01em;
  }
  .brand:hover {
    text-decoration: none;
  }
  .suit {
    color: var(--accent);
  }
  nav {
    display: flex;
    gap: 1.2rem;
    align-items: center;
    font-size: 0.92rem;
  }
  nav a {
    color: var(--text);
  }
  .me {
    color: var(--muted);
  }
  .me .num {
    color: var(--accent);
    font-weight: 700;
  }
  .live {
    color: var(--good) !important;
    font-weight: 700;
  }
  main {
    max-width: 82rem;
    margin: 0 auto;
    padding: 1.25rem 1.5rem 3rem;
  }
  .banner {
    text-align: center;
    padding: 0.55rem 1rem;
    background: rgba(227, 179, 65, 0.15);
    border-bottom: 1px solid rgba(227, 179, 65, 0.35);
  }
  .banner.subtle {
    background: rgba(255, 255, 255, 0.04);
    color: var(--muted);
    border-color: var(--line);
  }
  .toast {
    position: fixed;
    bottom: 1.5rem;
    left: 50%;
    transform: translateX(-50%);
    background: #2b1c1c;
    border: 1px solid #6b3535;
    padding: 0.6rem 1.1rem;
    border-radius: 8px;
    z-index: 20;
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.5);
  }
  .center {
    text-align: center;
    margin-top: 4rem;
  }
  footer {
    display: flex;
    gap: 1.5rem;
    justify-content: center;
    padding: 1.5rem;
    font-size: 0.8rem;
    border-top: 1px solid var(--line);
  }
  @media (max-width: 640px) {
    .top {
      padding: 0.7rem 1rem;
    }
    nav {
      gap: 0.8rem;
    }
    .me {
      display: none;
    }
    main {
      padding: 1rem 0.75rem 2rem;
    }
  }
</style>
