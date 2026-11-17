// The table as one player sees it, rebuilt from the server's messages. Pure functions
// (no Svelte, no I/O) so they can be unit-tested; the server stays authoritative and
// this only mirrors what it sends.

import type { MatchSnapshot, RatingChange, TurnInfo, WireEvent, WireStreet } from './protocol'

export type Pair<T> = [T, T]

export interface HandView {
  number: number
  level: number
  smallBlind: number
  bigBlind: number
  button: number
  street: WireStreet
  board: string[]
  /** Chips behind, by seat. */
  stacks: Pair<number>
  /** Chips each seat has put in on this street. */
  streetBets: Pair<number>
  /** Chips each seat has put in this hand (the pot is their sum while it runs). */
  committed: Pair<number>
  /** Hole cards this player may see: their own, plus anything tabled or shown. */
  cards: Pair<string[] | null>
  /** Hand descriptions from showdown. */
  shown: Pair<string | null>
  mucked: Pair<boolean>
  allIn: Pair<boolean>
  winnings: Pair<number>
  complete: boolean
}

export interface TableState {
  matchId: number
  mySeat: number
  names: Pair<string>
  ratings: Pair<number>
  /** False if the result won't change ratings (the pair's daily quota is used up). */
  rated: boolean
  handsPlayed: number
  /** Stacks between hands (the current hand's are in `hand`). */
  stacks: Pair<number>
  timeBanksMs: Pair<number>
  perActionMs: number
  hand: HandView | null
  turn: TurnInfo | null
  /** `performance.now()` when `turn` arrived, for the countdown. */
  turnAt: number
  log: string[]
  winner: number | null
  result: Pair<RatingChange> | null
  /** Whether the server has said how the match was recorded (`match_ended`). */
  resultSaved: boolean
  /** This player's auto-muck setting (takes effect from the next hand). */
  autoMuck: boolean
}

const LOG_LIMIT = 200

/** Chips in the middle: everything committed, less what has been paid out so far. */
export function pot(h: HandView): number {
  if (h.complete) return 0
  return Math.max(0, h.committed[0] + h.committed[1] - h.winnings[0] - h.winnings[1])
}

function set<T>(p: Pair<T>, seat: number, v: T): Pair<T> {
  const out: Pair<T> = [p[0], p[1]]
  out[seat] = v
  return out
}

export function fromSnapshot(s: MatchSnapshot, now: number): TableState {
  let t: TableState = {
    matchId: s.match_id,
    mySeat: s.seat,
    names: s.names,
    ratings: s.ratings,
    rated: s.rated,
    handsPlayed: s.hands_played,
    stacks: s.stacks,
    timeBanksMs: s.time_banks_ms,
    perActionMs: s.per_action_ms,
    hand: null,
    turn: null,
    turnAt: now,
    log: [],
    winner: s.winner,
    result: null,
    resultSaved: false,
    autoMuck: s.auto_muck,
  }
  if (s.hand) {
    for (const e of s.hand.events) t = applyEvent(t, e)
    // The server's public state is authoritative for the numbers.
    const p = s.hand.state
    if (t.hand) {
      t.hand = {
        ...t.hand,
        button: p.button,
        street: p.street,
        board: p.board,
        stacks: p.stacks,
        streetBets: p.street_bets,
        committed: p.committed,
        complete: p.complete,
      }
    }
  }
  t.turn = s.turn
  t.turnAt = now
  return t
}

function line(t: TableState, text: string): string[] {
  const log = [...t.log, text]
  return log.length > LOG_LIMIT ? log.slice(log.length - LOG_LIMIT) : log
}

const STREET_NAMES: Record<WireStreet, string> = {
  preflop: 'Preflop',
  flop: 'Flop',
  turn: 'Turn',
  river: 'River',
}

export function applyEvent(t: TableState, e: WireEvent): TableState {
  const name = (seat: number) => t.names[seat]
  if (e.type === 'hand_started') {
    const hand: HandView = {
      number: e.number,
      level: e.level,
      smallBlind: e.small_blind,
      bigBlind: e.big_blind,
      button: e.button,
      street: 'preflop',
      board: [],
      stacks: e.stacks,
      streetBets: [0, 0],
      committed: [0, 0],
      cards: [null, null],
      shown: [null, null],
      mucked: [false, false],
      allIn: [false, false],
      winnings: [0, 0],
      complete: false,
    }
    return {
      ...t,
      hand,
      stacks: e.stacks,
      timeBanksMs: e.time_banks_ms,
      turn: null,
      log: line(t, `Hand #${e.number} · blinds ${e.small_blind}/${e.big_blind}`),
    }
  }
  if (e.type === 'match_over') {
    return { ...t, winner: e.winner, turn: null, log: line(t, `${name(e.winner)} wins the match`) }
  }
  if (e.type === 'timed_out') {
    return { ...t, log: line(t, `${name(e.seat)} ran out of time`) }
  }
  const h = t.hand
  if (!h) return t
  let hand: HandView = h
  let text: string | null = null
  switch (e.type) {
    case 'blind_posted':
      hand = {
        ...h,
        stacks: set(h.stacks, e.seat, h.stacks[e.seat] - e.amount),
        streetBets: set(h.streetBets, e.seat, h.streetBets[e.seat] + e.amount),
        committed: set(h.committed, e.seat, h.committed[e.seat] + e.amount),
        allIn: set(h.allIn, e.seat, e.all_in),
      }
      text = `${name(e.seat)} posts ${e.big ? 'big' : 'small'} blind ${e.amount}`
      break
    case 'hole_cards':
      hand = { ...h, cards: set(h.cards, e.seat, e.cards as string[]) }
      break
    case 'acted': {
      hand = {
        ...h,
        stacks: set(h.stacks, e.seat, h.stacks[e.seat] - e.added),
        streetBets: set(h.streetBets, e.seat, e.street_total),
        committed: set(h.committed, e.seat, h.committed[e.seat] + e.added),
        allIn: set(h.allIn, e.seat, e.all_in),
      }
      const a = e.action
      const verb =
        a.kind === 'fold'
          ? 'folds'
          : a.kind === 'check'
            ? 'checks'
            : a.kind === 'call'
              ? `calls ${e.added}`
              : e.facing === 0
                ? `bets ${e.street_total}`
                : `raises to ${e.street_total}`
      text = `${name(e.seat)} ${verb}${e.all_in ? ' (all-in)' : ''}`
      // That decision is made; the next one (if any) arrives as a new turn.
      if (t.turn?.seat === e.seat) {
        return { ...t, hand, turn: null, log: line(t, text) }
      }
      break
    }
    case 'hands_tabled':
      hand = { ...h, cards: [e.cards[0] as string[], e.cards[1] as string[]] }
      text = 'Hands are tabled'
      break
    case 'board':
      hand = { ...h, board: [...h.board, ...e.cards], street: e.street, streetBets: [0, 0] }
      text = `${STREET_NAMES[e.street]}: ${e.cards.join(' ')}`
      break
    case 'uncalled_returned':
      hand = {
        ...h,
        stacks: set(h.stacks, e.seat, h.stacks[e.seat] + e.amount),
        streetBets: set(h.streetBets, e.seat, Math.max(0, h.streetBets[e.seat] - e.amount)),
        committed: set(h.committed, e.seat, h.committed[e.seat] - e.amount),
      }
      text = `${e.amount} returned to ${name(e.seat)}`
      break
    case 'showdown':
      hand = {
        ...h,
        cards: set(h.cards, e.seat, e.cards as string[]),
        shown: set(h.shown, e.seat, e.description),
      }
      text = `${name(e.seat)} shows ${e.cards.join(' ')} (${e.description})`
      break
    case 'mucked':
      hand = { ...h, mucked: set(h.mucked, e.seat, true) }
      text = `${name(e.seat)} mucks`
      break
    case 'won':
      hand = {
        ...h,
        stacks: set(h.stacks, e.seat, h.stacks[e.seat] + e.amount),
        winnings: set(h.winnings, e.seat, h.winnings[e.seat] + e.amount),
      }
      text = `${name(e.seat)} wins ${e.amount}`
      break
    case 'ended':
      hand = { ...h, stacks: e.stacks, streetBets: [0, 0], complete: true }
      return { ...t, hand, stacks: e.stacks, handsPlayed: t.handsPlayed + 1, turn: null }
  }
  return { ...t, hand, log: text ? line(t, text) : t.log }
}

export function applyTurn(t: TableState, turn: TurnInfo, now: number): TableState {
  return { ...t, turn, turnAt: now }
}

/** Milliseconds left on the current decision. */
export function remainingMs(t: TableState, now: number): number {
  if (!t.turn) return 0
  return Math.max(0, t.turn.remaining_ms - (now - t.turnAt))
}

/** Bet-sizing presets (street totals), clamped to the legal range. */
export function raisePresets(t: TableState): { label: string; to: number }[] {
  const h = t.hand
  const l = t.turn?.legal
  if (!h || !l || l.raise_min_to === null || l.raise_max_to === null) return []
  const lo = l.raise_min_to
  const hi = l.raise_max_to
  const facing = Math.max(h.streetBets[0], h.streetBets[1])
  const toCall = l.call ?? 0
  // A pot-sized raise: call, then raise by the pot after calling.
  const potAfterCall = pot(h) + toCall
  const clamp = (x: number) => Math.min(hi, Math.max(lo, Math.round(x)))
  const out = [
    { label: 'Min', to: lo },
    { label: '½ pot', to: clamp(facing + potAfterCall / 2) },
    { label: 'Pot', to: clamp(facing + potAfterCall) },
    { label: 'All-in', to: hi },
  ]
  return out.filter((p, i) => i === 0 || p.to !== out[i - 1].to)
}
