<script lang="ts">
  import { onMount } from 'svelte'
  import { ApiError, api } from '../lib/api'
  import { session } from '../lib/game.svelte'
  import { link, router } from '../lib/router.svelte'

  let token = ''
  let password = $state('')
  let confirm = $state('')
  let error = $state<string | null>(null)
  let busy = $state(false)
  let done = $state(false)

  onMount(() => {
    token = router.query.get('token') ?? ''
    // Keep the token out of the address bar and history.
    router.go('/reset-password', true)
  })

  async function submit(e: SubmitEvent) {
    e.preventDefault()
    if (password !== confirm) {
      error = "The passwords don't match."
      return
    }
    busy = true
    error = null
    try {
      await api.confirmPasswordReset(token, password)
      // Every session of the reset account has ended. This browser may have been
      // logged in as it (now logged out) or as someone else (still logged in).
      await session.load().catch(() => {})
      done = true
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Something went wrong.'
    } finally {
      busy = false
    }
  }
</script>

{#if done}
  <div class="auth panel" data-testid="reset-done">
    <h1>Password changed</h1>
    <p>That account has been logged out everywhere. Log in with your new password.</p>
    <p class="alt"><a href="/login" onclick={link}>Log in</a></p>
  </div>
{:else}
  <form class="auth panel" onsubmit={submit}>
    <h1>Choose a new password</h1>
    <label>
      New password
      <input type="password" autocomplete="new-password" required minlength="8" maxlength="128" bind:value={password} />
    </label>
    <label>
      Repeat it
      <input type="password" autocomplete="new-password" required minlength="8" maxlength="128" bind:value={confirm} />
    </label>
    {#if error}
      <p class="error" role="alert">{error}</p>
      <p class="muted"><a href="/forgot-password" onclick={link}>Get a new link</a></p>
    {/if}
    <button type="submit" class="primary wide" disabled={busy}>{busy ? 'Saving…' : 'Set password'}</button>
  </form>
{/if}

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
