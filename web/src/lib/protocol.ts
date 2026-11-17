// Generated from crates/server/src/protocol.rs by the `export_bindings` test.
// Do not edit by hand.

export type ClientMsg = { "type": "queue_join" } | { "type": "queue_leave" } | { "type": "act", action: WireAction, } | { "type": "set_auto_muck", on: boolean, };

export type ServerMsg = { "type": "hello", user_id: number, name: string, rating: number, email_verified: boolean, 
/**
 * Whether the user is seated in a live match (a `MatchState` follows if so).
 * A client still showing a table when this is false missed the match's end.
 */
in_match: boolean, } | { "type": "queued" } | { "type": "queue_left" } | { "type": "match_state" } & MatchSnapshot | { "type": "event", match_id: number, event: WireEvent, } | { "type": "turn", match_id: number, turn: TurnInfo, } | { "type": "match_ended", match_id: number, winner: number, 
/**
 * Indexed by seat; `None` if the result could not be recorded.
 */
ratings: [RatingChange, RatingChange] | null, } | { "type": "error", code: ErrorCode, message: string, } | { "type": "replaced" } | { "type": "restarting" };

export type ErrorCode = "bad_message" | "email_not_verified" | "already_in_match" | "not_in_match" | "illegal_action" | "unavailable" | "rate_limited";

export type MatchSnapshot = { match_id: number, 
/**
 * The receiving player's seat.
 */
seat: number, 
/**
 * Indexed by seat.
 */
names: [string, string], ratings: [number, number], hands_played: number, stacks: [number, number], time_banks_ms: [number, number], per_action_ms: number, 
/**
 * The hand in progress, if any.
 */
hand: HandSnapshot | null, turn: TurnInfo | null, winner: number | null, 
/**
 * The receiving player's auto-muck setting (takes effect from the next hand).
 */
auto_muck: boolean, };

export type HandSnapshot = { state: WirePublicState, 
/**
 * This hand's events so far, filtered for the receiving player (so it includes
 * their own hole cards and anything already tabled or shown).
 */
events: Array<WireEvent>, };

export type TurnInfo = { seat: number, legal: WireLegal, 
/**
 * Time left for this decision when the message was sent (free time + bank).
 */
remaining_ms: number, };

export type RatingChange = { before: number, after: number, };

export type WireAction = { "kind": "fold" } | { "kind": "check" } | { "kind": "call" } | { "kind": "raise_to", amount: number, } | { "kind": "all_in" };

export type WireStreet = "preflop" | "flop" | "turn" | "river";

export type WireLegal = { seat: number, can_fold: boolean, can_check: boolean, call: number | null, raise_min_to: number | null, raise_max_to: number | null, };

export type WirePublicState = { button: number, street: WireStreet, board: Array<string>, stacks: [number, number], street_bets: [number, number], committed: [number, number], pot: number, complete: boolean, };

export type WireEvent = { "type": "hand_started", number: number, level: number, button: number, small_blind: number, big_blind: number, stacks: [number, number], time_banks_ms: [number, number], } | { "type": "blind_posted", seat: number, amount: number, big: boolean, all_in: boolean, } | { "type": "hole_cards", seat: number, cards: [string, string], } | { "type": "acted", seat: number, action: WireAction, facing: number, added: number, street_total: number, all_in: boolean, } | { "type": "hands_tabled", cards: [[string, string], [string, string]], } | { "type": "board", street: WireStreet, cards: Array<string>, } | { "type": "uncalled_returned", seat: number, amount: number, } | { "type": "showdown", seat: number, cards: [string, string], 
/**
 * 1..=7462, higher wins.
 */
rank: number, description: string, } | { "type": "mucked", seat: number, } | { "type": "won", seat: number, amount: number, } | { "type": "ended", stacks: [number, number], } | { "type": "timed_out", seat: number, } | { "type": "match_over", winner: number, };

export type MeResponse = { id: number, name: string, rating: number, email_verified: boolean, 
/**
 * Whether this server only lets verified accounts queue.
 */
verification_required: boolean, };

export type RegisterResponse = { message: string, };

export type LeaderboardRow = { name: string, rating: number, rated_matches: number, };

export type MatchSummary = { id: number, opponent: string, 
/**
 * `in_progress`, `finished` or `abandoned`.
 */
status: string, 
/**
 * `None` unless finished.
 */
won: boolean | null, rating_before: number | null, rating_after: number | null, hands_played: number | null, started_at: string, };

export type HandHistory = { number: number, started_at: string, 
/**
 * PokerStars-style text from this player's seat.
 */
text: string, };
