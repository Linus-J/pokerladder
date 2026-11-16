<script lang="ts">
  import { ApiError, api } from '../lib/api'
  import { link } from '../lib/router.svelte'

  let email = $state('')
  let error = $state<string | null>(null)
  let busy = $state(false)
  let sent = $state(false)

  async function submit(e: SubmitEvent) {
    e.preventDefault()
    busy = true
    error = null
    try {
      await api.requestPasswordReset(email.trim())
      sent = true
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Something went wrong.'
    } finally {
      busy = false
    }
  }
</script>

{#if sent}
  <div class="auth panel" data-testid="reset-sent">
    <h1>Check your email</h1>
    <p>If that address has an account, we've sent it a link to choose a new password. The link expires in an hour.</p>
    <p class="muted alt"><a href="/login" onclick={link}>Back to log in</a></p>
  </div>
{:else}
  <form class="auth panel" onsubmit={submit}>
    <h1>Reset your password</h1>
    <label>
      Email
      <input type="email" autocomplete="email" required maxlength="254" bind:value={email} />
    </label>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <button type="submit" class="primary wide" disabled={busy}>{busy ? 'Sending…' : 'Send reset link'}</button>
    <p class="muted alt"><a href="/login" onclick={link}>Back to log in</a></p>
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
