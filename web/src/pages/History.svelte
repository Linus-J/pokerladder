<script lang="ts">
  import { Check, Copy, ScrollText } from '@lucide/svelte'
  import Icon from '../components/Icon.svelte'
  import { api } from '../lib/api'
  import type { HandHistory, MatchSummary } from '../lib/protocol'
  import { link, router } from '../lib/router.svelte'

  let matches = $state<MatchSummary[] | null>(null)
  let matchesFailed = $state(false)
  let hands = $state<HandHistory[] | null>(null)
  let handsFailed = $state(false)
  let copied = $state(false)

  const selected = $derived.by(() => {
    const m = router.path.match(/^\/history\/(\d+)$/)
    return m ? Number(m[1]) : null
  })
  const current = $derived(matches?.find((m) => m.id === selected) ?? null)

  $effect(() => {
    api.matches().then(
      (m) => (matches = m),
      () => (matchesFailed = true),
    )
  })

  $effect(() => {
    hands = null
    handsFailed = false
    copied = false
    if (selected === null) return
    const id = selected
    api.hands(id).then(
      (h) => {
        if (selected === id) hands = h
      },
      () => (handsFailed = true),
    )
  })

  function when(iso: string) {
    return new Date(iso).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' })
  }

  function outcome(m: MatchSummary) {
    if (m.status === 'in_progress') return { text: 'In progress', cls: 'muted' }
    if (m.status === 'abandoned') return { text: 'Abandoned', cls: 'muted' }
    return m.won ? { text: 'Won', cls: 'good' } : { text: 'Lost', cls: 'bad' }
  }

  let copyFailed = $state(false)
  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text)
      copied = true
      copyFailed = false
      setTimeout(() => (copied = false), 2000)
    } catch {
      copyFailed = true
    }
  }
</script>

<div class="page">
  <section class="block">
    <header><Icon of={ScrollText} /><h1 class="h">Your matches</h1></header>
    {#if matchesFailed}
      <p class="block-body">Your matches didn't load. Reload the page to try again.</p>
    {:else if !matches}
      <div class="block-body loading">
        {#each [80, 65, 72] as w, i (i)}<span class="skeleton" style:width="{w}%"></span>{/each}
      </div>
    {:else if matches.length === 0}
      <div class="block-body">
        <p><b>You haven't played a match yet.</b></p>
        <p class="muted">
          Each match you finish shows up here with its result and rating change, and you can read back
          every hand in it.
        </p>
        <p><a href="/" onclick={link}>Find a match in the lobby</a></p>
      </div>
    {:else}
      <table class="list">
        <thead><tr><th>Started</th><th>Opponent</th><th>Result</th><th class="r">Rating</th></tr></thead>
        <tbody>
          {#each matches as m (m.id)}
            {@const o = outcome(m)}
            {@const d = m.rating_after !== null && m.rating_before !== null ? m.rating_after - m.rating_before : null}
            <tr class:sel={m.id === selected}>
              <td><a href="/history/{m.id}" onclick={link} aria-current={m.id === selected ? 'page' : undefined}>{when(m.started_at)}</a></td>
              <td>{m.opponent}</td>
              <td class={o.cls}>{o.text}</td>
              <td class="r num">
                {#if d !== null}<span class={d >= 0 ? 'good' : 'bad'}>{d >= 0 ? '+' : ''}{d.toFixed(1)}</span>{/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </section>

  <section class="block">
    {#if selected === null}
      <header><h2 class="h">Hand histories</h2></header>
      <p class="block-body muted">
        Pick a match from the list to read its hands. They're written in the usual hand-history format, so
        you can paste them into a tracker or a forum post.
      </p>
    {:else}
      <header>
        <h2 class="h">
          {#if current}Hands vs {current.opponent}{:else}Hand histories{/if}
        </h2>
        {#if hands && hands.length > 0}
          <button type="button" class="aside copy" onclick={() => copy(hands!.map((h) => h.text).join('\n\n'))}>
            {#if copied}<Icon of={Check} />Copied{:else if copyFailed}<Icon of={Copy} />Couldn't copy. Select the text instead{:else}<Icon of={Copy} />{hands.length === 1 ? 'Copy the hand' : `Copy all ${hands.length} hands`}{/if}
          </button>
        {/if}
      </header>
      {#if handsFailed}
        <p class="block-body">That match's hands didn't load. Reload the page to try again.</p>
      {:else if !hands}
        <div class="block-body loading">
          {#each [90, 70, 85, 60] as w, i (i)}<span class="skeleton" style:width="{w}%"></span>{/each}
        </div>
      {:else if hands.length === 0}
        <p class="block-body muted">No hand in this match was finished, so there's nothing to show.</p>
      {:else}
        <div class="hands">
          {#each hands as h (h.number)}
            <details>
              <summary>Hand {h.number} <span class="muted small">{new Date(h.started_at).toLocaleTimeString()}</span></summary>
              <pre>{h.text}</pre>
            </details>
          {/each}
        </div>
      {/if}
    {/if}
  </section>
</div>

<style>
  .page {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    gap: var(--s5);
    align-items: start;
  }
  .loading {
    display: flex;
    flex-direction: column;
    gap: var(--s3);
  }
  tr.sel td {
    background: var(--note-bg);
  }
  .copy {
    margin-left: auto;
    padding: var(--s1) var(--s3);
    font-size: var(--t-sm);
  }
  .hands {
    padding: var(--s2) var(--s4) var(--s4);
  }
  details {
    border-bottom: 1px solid var(--line);
    padding: var(--s2) 0;
  }
  summary {
    cursor: pointer;
    font-weight: 700;
  }
  pre {
    font-family: var(--mono);
    font-size: var(--t-xs);
    line-height: 1.5;
    white-space: pre-wrap;
    background: var(--stripe);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: var(--s3);
    overflow-x: auto;
  }
  @media (max-width: 900px) {
    .page {
      grid-template-columns: 1fr;
    }
  }
</style>
