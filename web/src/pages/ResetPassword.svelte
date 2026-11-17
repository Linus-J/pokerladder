<script lang="ts">
  import { Check, KeyRound, TriangleAlert } from '@lucide/svelte'
  import { onMount } from 'svelte'
  import Icon from '../components/Icon.svelte'
  import { ApiError, api } from '../lib/api'
  import { session } from '../lib/game.svelte'
  import { link, router } from '../lib/router.svelte'

  let token = ''
  let password = $state('')
  let confirm = $state('')
  let error = $state<string | null>(null)
  /** The server rejected the link itself (expired or used). */
  let badLink = $state(false)
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
      error = "The two passwords don't match."
      return
    }
    busy = true
    error = null
    badLink = false
    try {
      await api.confirmPasswordReset(token, password)
      // Every session of the reset account has ended. This browser may have been
      // logged in as it (now logged out) or as someone else (still logged in).
      await session.load().catch(() => {})
      done = true
    } catch (err) {
      error = err instanceof ApiError ? err.message : "Saving your password didn't work. Try again in a moment."
      badLink = err instanceof ApiError && err.code === 'bad_token'
    } finally {
      busy = false
    }
  }
</script>

<div class="auth-page">
  {#if done}
    <section class="block" data-testid="reset-done">
      <header><Icon of={Check} /><h1 class="h">Password changed</h1></header>
      <div class="block-body">
        <p>That account is now logged out everywhere. Log in with your new password.</p>
        <p><a class="button primary" href="/login" onclick={link}>Log in</a></p>
      </div>
    </section>
  {:else}
    <form class="block" onsubmit={submit}>
      <header><Icon of={KeyRound} /><h1 class="h">Choose a new password</h1></header>
      <div class="block-body">
        <label>
          New password
          <input type="password" autocomplete="new-password" required minlength="8" maxlength="128" bind:value={password} />
          <span class="hint">At least 8 characters.</span>
        </label>
        <label>
          The same password again
          <input type="password" autocomplete="new-password" required minlength="8" maxlength="128" bind:value={confirm} />
        </label>
        {#if error}
          <div class="alert" role="alert">
            <Icon of={TriangleAlert} />
            <span>
              {error}
              {#if badLink}<a href="/forgot-password" onclick={link}>Ask for a new link</a>.{/if}
            </span>
          </div>
        {/if}
        <button type="submit" class="primary big submit" disabled={busy}>
          {busy ? 'Saving...' : 'Save new password'}
        </button>
      </div>
    </form>
  {/if}
</div>
