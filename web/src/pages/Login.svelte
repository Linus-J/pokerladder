<script lang="ts">
  import { LogIn, TriangleAlert } from '@lucide/svelte'
  import Icon from '../components/Icon.svelte'
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
      error = err instanceof ApiError ? err.message : "Logging in didn't work. Try again in a moment."
    } finally {
      busy = false
    }
  }
</script>

<div class="auth-page">
  <form class="block" onsubmit={submit}>
    <header><Icon of={LogIn} /><h1 class="h">Log in</h1></header>
    <div class="block-body">
      <label>
        Name or email
        <input type="text" autocomplete="username" required bind:value={login} />
      </label>
      <label>
        Password
        <input type="password" autocomplete="current-password" required bind:value={password} />
      </label>
      {#if error}<div class="alert" role="alert"><Icon of={TriangleAlert} /><span>{error}</span></div>{/if}
      <button type="submit" class="primary big submit" disabled={busy}>{busy ? 'Logging in...' : 'Log in'}</button>
      <p class="after"><a href="/forgot-password" onclick={link}>Forgotten your password?</a></p>
    </div>
  </form>

  <aside class="block">
    <header><h2 class="h">New here?</h2></header>
    <div class="block-body small">
      <p>An account takes a minute: a display name, an email address and a password.</p>
      <p>You'll need to confirm your email before your first match.</p>
      <p><a href="/register" onclick={link}>Create a free account</a></p>
    </div>
  </aside>
</div>
