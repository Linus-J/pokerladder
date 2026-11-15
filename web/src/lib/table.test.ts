import { describe, expect, it } from 'vitest'
import type { MatchSnapshot, TurnInfo, WireEvent } from './protocol'
import { applyEvent, applyTurn, fromSnapshot, pot, raisePresets, remainingMs, type TableState } from './table'

const snapshot: MatchSnapshot = {
  match_id: 7,
  seat: 1,
  names: ['alice', 'bob'],
  ratings: [1500, 1520],
  hands_played: 0,
  stacks: [1500, 1500],
  time_banks_ms: [60000, 60000],
  per_action_ms: 15000,
  hand: null,
  turn: null,
  winner: null,
}

// Hand 1: alice (seat 0) has the button and posts the small blind.
const start: WireEvent[] = [
  { type: 'hand_started', number: 1, level: 1, button: 0, small_blind: 10, big_blind: 20, stacks: [1500, 1500], time_banks_ms: [60000, 60000] },
  { type: 'blind_posted', seat: 0, amount: 10, big: false, all_in: false },
  { type: 'blind_posted', seat: 1, amount: 20, big: true, all_in: false },
  { type: 'hole_cards', seat: 1, cards: ['As', 'Kd'] },
]

function play(events: WireEvent[], t: TableState = fromSnapshot(snapshot, 0)) {
  return events.reduce(applyEvent, t)
}

describe('table reducer', () => {
  it('tracks blinds, own hole cards and the pot', () => {
    const t = play(start)
    const h = t.hand!
    expect(h.stacks).toEqual([1490, 1480])
    expect(h.streetBets).toEqual([10, 20])
    expect(pot(h)).toBe(30)
    expect(h.cards).toEqual([null, ['As', 'Kd']])
    expect(h.button).toBe(0)
    expect(t.log.at(-1)).toBe('bob posts big blind 20')
  })

  it('plays a raised pot to a showdown and settles stacks', () => {
    const t = play([
      ...start,
      // alice raises to 60 (adds 50), bob calls 40.
      { type: 'acted', seat: 0, action: { kind: 'raise_to', amount: 60 }, facing: 20, added: 50, street_total: 60, all_in: false },
      { type: 'acted', seat: 1, action: { kind: 'call' }, facing: 60, added: 40, street_total: 60, all_in: false },
      { type: 'board', street: 'flop', cards: ['Ah', '7c', '2d'] },
    ])
    let h = t.hand!
    expect(h.streetBets).toEqual([0, 0])
    expect(h.committed).toEqual([60, 60])
    expect(pot(h)).toBe(120)
    expect(h.board).toEqual(['Ah', '7c', '2d'])
    expect(t.log).toContain('alice raises to 60')
    expect(t.log).toContain('bob calls 40')
    expect(t.log).toContain('Flop: Ah 7c 2d')

    const t2 = play(
      [
        // bob bets 100 into 120, alice folds: the bet comes back.
        { type: 'acted', seat: 1, action: { kind: 'raise_to', amount: 100 }, facing: 0, added: 100, street_total: 100, all_in: false },
        { type: 'acted', seat: 0, action: { kind: 'fold' }, facing: 100, added: 0, street_total: 0, all_in: false },
        { type: 'uncalled_returned', seat: 1, amount: 100 },
        { type: 'won', seat: 1, amount: 120 },
        { type: 'ended', stacks: [1440, 1560] },
      ],
      t,
    )
    h = t2.hand!
    expect(t2.log).toContain('bob bets 100')
    expect(h.complete).toBe(true)
    expect(h.stacks).toEqual([1440, 1560])
    expect(h.winnings).toEqual([0, 120])
    expect(pot(h)).toBe(0)
    expect(t2.stacks).toEqual([1440, 1560])
    expect(t2.handsPlayed).toBe(1)
    // Chips are conserved by the reducer's own bookkeeping before `ended` overrides it.
    const before = play([{ type: 'won', seat: 1, amount: 120 }], play([
      { type: 'acted', seat: 1, action: { kind: 'raise_to', amount: 100 }, facing: 0, added: 100, street_total: 100, all_in: false },
      { type: 'acted', seat: 0, action: { kind: 'fold' }, facing: 100, added: 0, street_total: 0, all_in: false },
      { type: 'uncalled_returned', seat: 1, amount: 100 },
    ], t)).hand!
    expect(before.stacks).toEqual([1440, 1560])
  })

  it('reveals tabled and shown hands, and remembers mucks', () => {
    const t = play([
      ...start,
      { type: 'acted', seat: 0, action: { kind: 'all_in' }, facing: 20, added: 1490, street_total: 1500, all_in: true },
      { type: 'acted', seat: 1, action: { kind: 'call' }, facing: 1500, added: 1480, street_total: 1500, all_in: true },
      { type: 'hands_tabled', cards: [['Qs', 'Qh'], ['As', 'Kd']] },
    ])
    expect(t.hand!.cards).toEqual([['Qs', 'Qh'], ['As', 'Kd']])
    expect(t.hand!.allIn).toEqual([true, true])
    expect(t.log).toContain('alice raises to 1500 (all-in)')
    const t2 = play([{ type: 'showdown', seat: 0, cards: ['Qs', 'Qh'], rank: 3000, description: 'a pair of Queens' }, { type: 'mucked', seat: 1 }], t)
    expect(t2.hand!.shown).toEqual(['a pair of Queens', null])
    expect(t2.hand!.mucked).toEqual([false, true])
  })

  it('ends the match and ignores hand events with no hand', () => {
    const t0 = fromSnapshot(snapshot, 0)
    expect(applyEvent(t0, { type: 'won', seat: 0, amount: 5 })).toBe(t0)
    const t = play([...start, { type: 'match_over', winner: 0 }])
    expect(t.winner).toBe(0)
    expect(t.log.at(-1)).toBe('alice wins the match')
  })

  it('rebuilds a reconnecting table from a snapshot', () => {
    const t = fromSnapshot(
      {
        ...snapshot,
        hand: {
          events: start,
          state: {
            button: 0,
            street: 'preflop',
            board: [],
            stacks: [1490, 1480],
            street_bets: [10, 20],
            committed: [10, 20],
            pot: 30,
            complete: false,
          },
        },
        turn: { seat: 0, remaining_ms: 9000, legal: { seat: 0, can_fold: true, can_check: false, call: 10, raise_min_to: 40, raise_max_to: 1500 } },
      },
      1000,
    )
    expect(t.hand!.cards[1]).toEqual(['As', 'Kd'])
    expect(t.hand!.stacks).toEqual([1490, 1480])
    expect(remainingMs(t, 1000)).toBe(9000)
    expect(remainingMs(t, 4000)).toBe(6000)
    expect(remainingMs(t, 99999)).toBe(0)
  })
})

describe('raise presets', () => {
  const turn = (call: number | null, lo: number, hi: number): TurnInfo => ({
    seat: 1,
    remaining_ms: 15000,
    legal: { seat: 1, can_fold: call !== null, can_check: call === null, call, raise_min_to: lo, raise_max_to: hi },
  })

  it('sizes pot raises from the pot after calling', () => {
    // bob faces alice's raise to 60 (pot 80: 60 + 20); calling adds 40 → pot 120.
    let t = play([...start, { type: 'acted', seat: 0, action: { kind: 'raise_to', amount: 60 }, facing: 20, added: 50, street_total: 60, all_in: false }])
    t = applyTurn(t, turn(40, 100, 1500), 0)
    const p = Object.fromEntries(raisePresets(t).map((x) => [x.label, x.to]))
    expect(p).toEqual({ Min: 100, '½ pot': 120, Pot: 180, 'All-in': 1500 })
  })

  it('clamps to the legal range and drops duplicates', () => {
    let t = play(start)
    t = applyTurn(t, turn(10, 100, 120), 0)
    const p = raisePresets(t)
    expect(p.every((x) => x.to >= 100 && x.to <= 120)).toBe(true)
    expect(new Set(p.map((x) => x.to)).size).toBe(p.length)
    expect(raisePresets(applyTurn(t, { ...turn(10, 0, 0), legal: { ...turn(10, 0, 0).legal, raise_min_to: null, raise_max_to: null } }, 0))).toEqual([])
  })
})
