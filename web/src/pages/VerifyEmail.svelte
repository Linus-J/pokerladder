<script lang="ts">
  import { onMount } from 'svelte'
  import { ApiError, api } from '../lib/api'
  import { session } from '../lib/game.svelte'
  import { link, router } from '../lib/router.svelte'

  let status = $state<'working' | 'ok' | 'bad'>('working')
  let message = $state('')

  onMount(async () => {
    const token = router.query.get('token') ?? ''
    // Drop the token from the address bar and history.
    history.replaceState(null, '', '/verify-email')
    try {
      await api.verifyEmail(token)
      status = 'ok'
      if (session.me) session.me = { ...session.me, email_verified: true }
    } catch (e) {
      status = 'bad'
      message = e instanceof ApiError ? e.message : 'Something went wrong.'
    }
  })
</script>

<div class="box panel">
  {#if status === 'working'}
    <p>Confirming your email…</p>
  {:else if status === 'ok'}
    <h1>Email confirmed</h1>
    <p>You're ready to play.</p>
    {#if session.me}
      <a href="/" onclick={link}>Go to the lobby</a>
    {:else}
      <a href="/login" onclick={link}>Log in</a>
    {/if}
  {:else}
    <h1>That link didn't work</h1>
    <p class="error">{message}</p>
    <p class="muted">Log in and use "Send it again" in the lobby to get a new one.</p>
  {/if}
</div>

<style>
  .box {
    max-width: 26rem;
    margin: 3rem auto;
    text-align: center;
  }
</style>
