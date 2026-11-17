<script lang="ts">
  import { KeyRound, MailCheck, TriangleAlert } from '@lucide/svelte'
  import Icon from '../components/Icon.svelte'
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
      error = err instanceof ApiError ? err.message : "That request didn't go through. Try again in a moment."
    } finally {
      busy = false
    }
  }
</script>

<div class="auth-page">
  {#if sent}
    <section class="block" data-testid="reset-sent">
      <header><Icon of={MailCheck} /><h1 class="h">Check your email</h1></header>
      <div class="block-body">
        <p>
          If that address belongs to an account, we've sent it a link to choose a new password. The
          link works for one hour.
        </p>
        <p class="after"><a href="/login" onclick={link}>Back to log in</a></p>
      </div>
    </section>
  {:else}
    <form class="block" onsubmit={submit}>
      <header><Icon of={KeyRound} /><h1 class="h">Reset your password</h1></header>
      <div class="block-body">
        <label>
          The email address you signed up with
          <input type="email" autocomplete="email" required maxlength="254" bind:value={email} />
        </label>
        {#if error}<div class="alert" role="alert"><Icon of={TriangleAlert} /><span>{error}</span></div>{/if}
        <button type="submit" class="primary big submit" disabled={busy}>
          {busy ? 'Sending...' : 'Email me a reset link'}
        </button>
        <p class="after"><a href="/login" onclick={link}>Back to log in</a></p>
      </div>
    </form>
  {/if}

  <aside class="block">
    <header><h2 class="h">What happens next</h2></header>
    <div class="block-body small">
      <p>You'll get an email with a link. Open it, pick a new password, and you're done.</p>
      <p>Changing your password logs your account out everywhere, so log in again afterwards.</p>
    </div>
  </aside>
</div>
