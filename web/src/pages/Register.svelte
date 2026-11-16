<script lang="ts">
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
      error = err instanceof ApiError ? err.message : 'Something went wrong.'
    } finally {
      busy = false
    }
  }
</script>

{#if done}
  <div class="auth panel" data-testid="registered">
    <h1>Check your email</h1>
    <p>{done}</p>
    <p class="muted">Once you've confirmed it, <a href="/login" onclick={link}>log in</a> to start playing.</p>
  </div>
{:else}
  <form class="auth panel" onsubmit={submit}>
    <h1>Create account</h1>
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
      <span class="hint" class:error={name !== '' && !nameOk}>Up to 20 letters, digits, _ or -. Shown to opponents.</span>
    </label>
    <label>
      Email
      <input type="email" autocomplete="email" required bind:value={email} />
      <span class="hint">For confirming your account and contacting prize winners. Never shown.</span>
    </label>
    <label>
      Password
      <input type="password" autocomplete="new-password" required minlength="8" maxlength="128" bind:value={password} />
      <span class="hint">At least 8 characters.</span>
    </label>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <button type="submit" class="primary wide" disabled={busy || !nameOk}>
      {busy ? 'Creating…' : 'Create account'}
    </button>
    <p class="muted fine">
      Free to play — no payment is ever needed to enter. See the <a href="/privacy" onclick={link}>privacy notice</a>.
    </p>
    <p class="muted alt">Have an account? <a href="/login" onclick={link}>Log in</a></p>
  </form>
{/if}

<style>
  .auth {
    max-width: 26rem;
    margin: 3rem auto;
    padding: 1.75rem;
  }
  .hint {
    display: block;
    font-size: 0.75rem;
    margin-top: 0.3em;
  }
  .wide {
    width: 100%;
    margin-top: 0.4rem;
  }
  .fine {
    font-size: 0.8rem;
    margin: 1rem 0 0;
  }
  .alt {
    margin: 0.6rem 0 0;
    text-align: center;
    font-size: 0.9rem;
  }
</style>
