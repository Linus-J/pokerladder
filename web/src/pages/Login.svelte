<script lang="ts">
  import { ApiError } from '../lib/api'
  import { session } from '../lib/game.svelte'
  import { link, router } from '../lib/router.svelte'

  let login = $state('')
  let password = $state('')
  let error = $state<string | null>(null)
  let busy = $state(false)

  async function submit(e: SubmitEvent) {
    e.preventDefault()
    busy = true
    error = null
    try {
      await session.login(login.trim(), password)
      router.go('/', true)
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Something went wrong.'
    } finally {
      busy = false
    }
  }
</script>

<form class="auth panel" onsubmit={submit}>
  <h1>Log in</h1>
  <label>
    Name or email
    <input type="text" autocomplete="username" required bind:value={login} />
  </label>
  <label>
    Password
    <input type="password" autocomplete="current-password" required bind:value={password} />
  </label>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  <button type="submit" class="primary wide" disabled={busy}>{busy ? 'Logging in…' : 'Log in'}</button>
  <p class="muted alt">New here? <a href="/register" onclick={link}>Create an account</a></p>
</form>

<style>
  .auth {
    max-width: 24rem;
    margin: 3rem auto;
    padding: 1.75rem;
  }
  .wide {
    width: 100%;
    margin-top: 0.4rem;
  }
  .alt {
    margin: 1.1rem 0 0;
    text-align: center;
    font-size: 0.9rem;
  }
</style>
