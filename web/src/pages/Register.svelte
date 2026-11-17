<script lang="ts">
  import { MailCheck, TriangleAlert, UserPlus } from '@lucide/svelte'
  import Icon from '../components/Icon.svelte'
  import { ApiError, api } from '../lib/api'
  import { link } from '../lib/router.svelte'

  let name = $state('')
  let email = $state('')
  let password = $state('')
  let error = $state<string | null>(null)
  let done = $state<string | null>(null)
  let busy = $state(false)

  const nameOk = $derived(/^[A-Za-z0-9_-]{1,20}$/.test(name))

  async function submit(e: SubmitEvent) {
    e.preventDefault()
    busy = true
    error = null
    try {
      done = (await api.register(name, email.trim(), password)).message
    } catch (err) {
      error = err instanceof ApiError ? err.message : "Creating your account didn't work. Try again in a moment."
    } finally {
      busy = false
    }
  }
</script>

<div class="auth-page">
  {#if done}
    <section class="block" data-testid="registered">
      <header><Icon of={MailCheck} /><h1 class="h">Check your email</h1></header>
      <div class="block-body">
        <p>{done}</p>
        <p class="muted">
          The link is valid for 48 hours. If it hasn't arrived in a few minutes, look in your spam
          folder. Once you've confirmed, <a href="/login" onclick={link}>log in</a> to start playing.
        </p>
      </div>
    </section>
  {:else}
    <form class="block" onsubmit={submit}>
      <header><Icon of={UserPlus} /><h1 class="h">Create an account</h1></header>
      <div class="block-body">
        <label>
          Display name
          <input
            type="text"
            autocomplete="username"
            required
            maxlength="20"
            pattern="[A-Za-z0-9_\-]{'{'}1,20{'}'}"
            bind:value={name}
          />
          <span class="hint" class:bad={name !== '' && !nameOk}>
            Up to 20 letters, digits, _ or -. Your opponents and the leaderboard show this.
          </span>
        </label>
        <label>
          Email
          <input type="email" autocomplete="email" required bind:value={email} />
          <span class="hint">Only for confirming your account and contacting prize winners. Never shown.</span>
        </label>
        <label>
          Password
          <input type="password" autocomplete="new-password" required minlength="8" maxlength="128" bind:value={password} />
          <span class="hint">At least 8 characters.</span>
        </label>
        {#if error}<div class="alert" role="alert"><Icon of={TriangleAlert} /><span>{error}</span></div>{/if}
        <button type="submit" class="primary big submit" disabled={busy || !nameOk}>
          {busy ? 'Creating your account...' : 'Create account'}
        </button>
        <p class="after">Already registered? <a href="/login" onclick={link}>Log in</a></p>
      </div>
    </form>
  {/if}

  <aside class="block">
    <header><h2 class="h">Before you sign up</h2></header>
    <div class="block-body small">
      <p><b>It's free.</b> There's no paid entry of any kind. Prizes, when seasons start, are paid by sponsors.</p>
      <p><b>One account each.</b> Matches will be checked for collusion before any prize is awarded.</p>
      <p>
        We keep a full record of every hand, including cards nobody saw. The
        <a href="/privacy" onclick={link}>privacy notice</a> explains what's stored and why.
      </p>
    </div>
  </aside>
</div>
