// Replays matches played by the real engine (crates/server/tests/client_fixture.rs)
// through the reducer and checks it agrees with the engine after every step.

import { describe, expect, it } from 'vitest'
import raw from './engine-fixture.json'
import type { WireEvent, WirePublicState } from './protocol'
import { applyEvent, fromSnapshot, pot, type TableState } from './table'

interface Step {
  events: WireEvent[]
  state: WirePublicState | null
  stacks: [number, number]
}
interface Game {
  seed: number
  seat: number
  names: [string, string]
  steps: Step[]
}

const games = raw as unknown as Game[]
const TOTAL = 3000 // MatchConfig::standard(): 1,500 each

function empty(g: Game): TableState {
  return fromSnapshot(
    {
      match_id: g.seed,
      seat: g.seat,
      names: g.names,
      ratings: [1500, 1500],
      hands_played: 0,
      stacks: [1500, 1500],
      time_banks_ms: [0, 0],
      per_action_ms: 0,
      hand: null,
      turn: null,
      winner: null,
      auto_muck: true,
    },
    0,
  )
}

describe('reducer vs engine', () => {
  it('has a fixture with real coverage', () => {
    const types = new Set(games.flatMap((g) => g.steps.flatMap((s) => s.events.map((e) => e.type))))
    for (const t of ['showdown', 'mucked', 'uncalled_returned', 'hands_tabled', 'timed_out', 'match_over']) {
      expect(types, t).toContain(t)
    }
  })

  for (const g of games) {
    it(`agrees on every step of match ${g.seed} (seat ${g.seat})`, () => {
      let t = empty(g)
      let hands = 0
      let tabled = false // this hand's hole cards are face up
      g.steps.forEach((step, i) => {
        const at = `step ${i}`
        for (const e of step.events) {
          if (e.type === 'hand_started') tabled = false
          if (e.type === 'hands_tabled') tabled = true
          // Each event is its own message (and may be rendered), so the reducer's own
          // bookkeeping must already reach the final stacks that `ended` announces.
          if (e.type === 'ended') {
            expect(t.hand?.stacks, `${at} before ended`).toEqual(e.stacks)
            hands += 1
          }
          t = applyEvent(t, e)
          if (t.hand) {
            const h = t.hand
            expect(h.stacks[0] + h.stacks[1] + pot(h), `${at} ${e.type}`).toBe(TOTAL)
          }
        }
        const h = t.hand
        const s = step.state
        if (s) {
          expect(h, at).not.toBeNull()
          expect(h!.button, at).toBe(s.button)
          expect(h!.street, at).toBe(s.street)
          expect(h!.board, at).toEqual(s.board)
          expect(h!.stacks, at).toEqual(s.stacks)
          // The engine keeps the last street's bets on a finished hand; the table
          // clears them (the chips have gone to the winner).
          expect(h!.streetBets, at).toEqual(s.complete ? [0, 0] : s.street_bets)
          expect(h!.committed, at).toEqual(s.committed)
          expect(h!.complete, at).toBe(s.complete)
          expect(pot(h!), at).toBe(s.pot)
          // Independent of both implementations: no chip appears or vanishes.
          expect(h!.stacks[0] + h!.stacks[1] + pot(h!), at).toBe(TOTAL)
        }
        if (!h || h.complete) expect(t.stacks, at).toEqual(step.stacks)
        // Only ever the viewer's own hole cards, until they are tabled or shown.
        if (h && !h.complete) {
          const opp = 1 - g.seat
          if (!tabled && h.shown[opp] === null) expect(h.cards[opp], at).toBeNull()
          expect(h.cards[g.seat], at).not.toBeNull()
        }
      })
      expect(t.handsPlayed).toBe(hands)
      const last = g.steps.at(-1)!
      if (last.events.some((e) => e.type === 'match_over')) {
        expect(t.winner).not.toBeNull()
        expect(last.stacks[t.winner!]).toBe(TOTAL)
      }
    })
  }
})
