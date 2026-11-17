<script lang="ts">
  import { onMount } from 'svelte'
  import { link, scrollToHash } from '../lib/router.svelte'
  import { PAIR_DAILY_LIMIT, QUALIFYING_MATCHES as QUALIFYING } from '../lib/season'

  // A direct visit to /about#ratings: the page renders after the session loads, so
  // the browser's own jump to the anchor has already missed it.
  onMount(() => {
    if (location.hash) scrollToHash(location.hash)
  })

  // Mirrors MatchConfig::standard() in crates/engine/src/sng.rs.
  const BLINDS: [number, number][] = [
    [10, 20],
    [15, 30],
    [25, 50],
    [40, 80],
    [60, 120],
    [100, 200],
    [150, 300],
    [250, 500],
    [400, 800],
    [600, 1200],
    [1000, 2000],
    [1500, 3000],
  ]
  const HANDS_PER_LEVEL = 10
</script>

<div class="page">
  <article class="doc">
    <h1>About Pokerladder</h1>
    <p class="lead">
      Pokerladder is a ladder for heads-up No-Limit Hold'em. You play one opponent at a time, the
      server pairs you with someone near your rating, and every finished match moves both ratings.
      It is free to play.
    </p>

    <section id="matches">
      <h2>How a match works</h2>
      <p>
        Every match is a sit-and-go. Both players start with <b>1,500 chips</b> and play until one of
        them has all 3,000. The blinds go up every {HANDS_PER_LEVEL} hands, so no match goes on for ever.
      </p>
      <ul>
        <li>
          <b>The clock.</b> You have 15 seconds for each decision. When that runs out your time bank
          takes over: 60 seconds to start with, plus 10 more every 10 hands. If the bank runs dry too,
          you check if you can and fold if you can't.
        </li>
        <li>
          <b>Disconnecting.</b> The match carries on without you and your clock keeps running. If it
          runs out, you're sat out: from then on you check or fold a second after each decision reaches
          you, until you come back. Open the site again to pick up where you left off.
        </li>
        <li>
          <b>Finding an opponent.</b> You can't choose who you play. The queue pairs you with the
          closest-rated player who is waiting. At first it only accepts someone within 100 points of
          you, and that gap widens by 20 points for every second you wait.
        </li>
      </ul>

      <table class="list blinds">
        <caption>Blind levels</caption>
        <thead><tr><th>Level</th><th>Hands</th><th class="r">Small blind</th><th class="r">Big blind</th></tr></thead>
        <tbody>
          {#each BLINDS as [sb, bb], i (i)}
            {@const first = i * HANDS_PER_LEVEL + 1}
            <tr>
              <td class="num">{i + 1}</td>
              <td class="num">
                {#if i === BLINDS.length - 1}{first} onwards{:else}{first} to {first + HANDS_PER_LEVEL - 1}{/if}
              </td>
              <td class="r num">{sb.toLocaleString()}</td>
              <td class="r num">{bb.toLocaleString()}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </section>

    <section id="ratings">
      <h2>Ratings</h2>
      <p>
        Ratings use the Glicko-2 system. As well as a number, it keeps track of how sure it is of that
        number. A new rating starts at <b>1500</b> and very unsure, so your first few matches move it a
        lot. The more you play, the surer it gets and the smaller the steps become. Beating someone
        rated above you is worth more than beating someone below you. If you stop playing for a while,
        it slowly becomes unsure again.
      </p>
      <p>
        Only finished matches count; abandoned ones don't. And only the first {PAIR_DAILY_LIMIT}
        matches between the same two players in any 24 hours are rated. Later ones are played as
        normal but change nothing, and the table says so before the first hand.
      </p>
      <p>
        You have two ratings. Your <b>season rating</b> is the one you see everywhere: it starts again
        at 1500 every month. A longer-running rating, kept behind the scenes, is only used to find you
        opponents of the right strength, so a new season doesn't pair beginners with the best players.
        Beta ratings (from before seasons began) were reset when seasons started; your old matches
        still show the Elo points they moved.
      </p>
    </section>

    <section id="seasons">
      <h2>Seasons and prizes</h2>
      <p>
        A season is one calendar month, from midnight UTC on the 1st. To be ranked on the
        <a href="/leaderboard" onclick={link}>leaderboard</a> you need {QUALIFYING} rated matches in
        that season. Ranked players are ordered by their <b>score</b>: season rating minus twice its
        uncertainty. That rewards a rating you've shown over many matches rather than one lucky streak,
        and a player who stops playing slowly slips down. The standings at the end of the month are
        final, and earlier seasons stay on the leaderboard page.
      </p>
      <p>
        Prizes for the top of a season will be paid for by sponsors, and announced before the season
        starts. Entry is free: there is no paid entry of any kind, and buying something never improves
        your chances. Before any prize is paid, the winners' matches are checked for collusion and chip
        dumping.
      </p>
    </section>

    <section id="fair-play">
      <h2>Fair play</h2>
      <ul>
        <li>One account per person.</li>
        <li>
          The server shuffles, deals and enforces every rule. Your browser only ever receives the cards
          you are allowed to see.
        </li>
        <li>
          Every hand is recorded in full, including folded and mucked cards, so matches can be checked
          for collusion and chip dumping before any prize is paid. Other players never see cards you
          didn't show. The <a href="/privacy" onclick={link}>privacy notice</a> has the details.
        </li>
      </ul>
    </section>

    <section id="keys">
      <h2>Keyboard</h2>
      <p>At the table: <kbd>F</kbd> folds, <kbd>C</kbd> checks or calls, <kbd>R</kbd> bets or raises the amount you've set.</p>
    </section>
  </article>

  <nav class="block toc" aria-label="On this page">
    <header><h2>On this page</h2></header>
    <ul class="block-body">
      <li><a href="#matches">How a match works</a></li>
      <li><a href="#ratings">Ratings</a></li>
      <li><a href="#seasons">Seasons and prizes</a></li>
      <li><a href="#fair-play">Fair play</a></li>
      <li><a href="#keys">Keyboard</a></li>
    </ul>
  </nav>
</div>

<style>
  .page {
    display: grid;
    grid-template-columns: minmax(0, 44rem) 15rem;
    gap: var(--s6);
    align-items: start;
  }
  .doc section {
    margin-top: var(--s6);
  }
  .lead {
    font-size: var(--t-md);
  }
  ul {
    padding-left: var(--s5);
    max-width: 42rem;
  }
  li {
    margin-bottom: var(--s2);
  }
  .blinds {
    max-width: 30rem;
    border: 1px solid var(--line);
    margin-top: var(--s4);
  }
  caption {
    text-align: left;
    font-weight: 700;
    padding: var(--s2) 0;
  }
  .toc {
    position: sticky;
    top: var(--s4);
  }
  .toc ul {
    list-style: none;
    margin: 0;
    padding: var(--s3) var(--s4);
  }
  .toc li {
    margin: var(--s1) 0;
  }
  @media (max-width: 860px) {
    .page {
      grid-template-columns: 1fr;
    }
    .toc {
      display: none;
    }
  }
</style>
