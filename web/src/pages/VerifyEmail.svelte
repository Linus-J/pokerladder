<script lang="ts">
  import { Check, LoaderCircle, TriangleAlert } from '@lucide/svelte'
  import { onMount } from 'svelte'
  import Icon from '../components/Icon.svelte'
  import { ApiError, api } from '../lib/api'
  import { session } from '../lib/game.svelte'
  import { link, router } from '../lib/router.svelte'

  let status = $state<'working' | 'ok' | 'bad'>('working')
  let message = $state('')

  onMount(async () => {
    const token = router.query.get('token') ?? ''
    // Drop the token from the address bar and history.
    router.go('/verify-email', true)
    try {
      await api.verifyEmail(token)
      status = 'ok'
      if (session.me) session.me = { ...session.me, email_verified: true }
    } catch (e) {
      status = 'bad'
      message = e instanceof ApiError ? e.message : "We couldn't check that link just now."
    }
  })
</script>

<div class="auth-page">
  <section class="block">
    {#if status === 'working'}
      <header><Icon of={LoaderCircle} spin /><h1 class="h">Confirming your email...</h1></header>
      <div class="block-body"><span class="skeleton" style:width="70%"></span></div>
    {:else if status === 'ok'}
      <header><Icon of={Check} /><h1 class="h">Email confirmed</h1></header>
      <div class="block-body">
        <p>That's everything. You can play rated matches now.</p>
        {#if session.me}
          <p><a class="button primary" href="/" onclick={link}>Go to the lobby</a></p>
        {:else}
          <p><a class="button primary" href="/login" onclick={link}>Log in to play</a></p>
        {/if}
      </div>
    {:else}
      <header><Icon of={TriangleAlert} /><h1 class="h">That link didn't work</h1></header>
      <div class="block-body">
        <p class="bad">{message}</p>
        <p>Links last 48 hours and only work once. Log in and use "Send the link again" in the lobby to get a fresh one.</p>
      </div>
    {/if}
  </section>
</div>
