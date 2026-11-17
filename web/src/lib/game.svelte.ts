// App-wide reactive state: who is logged in, and the live WebSocket (queue + match).

import { api, ApiError } from './api'
import type { ClientMsg, ErrorCode, MeResponse, ServerMsg, WireAction } from './protocol'
import { router } from './router.svelte'
import { applyEvent, applyTurn, fromSnapshot, type TableState } from './table'

class Session {
  me = $state<MeResponse | null>(null)
  loaded = $state(false)

  async load() {
    try {
      this.me = await api.me()
    } catch (e) {
      if (!(e instanceof ApiError && e.status === 401)) throw e
      this.me = null
    } finally {
      this.loaded = true
    }
  }

  async login(login: string, password: string) {
    this.me = await api.login(login, password)
  }

  async logout() {
    game.stop()
    try {
      await api.logout()
    } finally {
      this.me = null
    }
  }
}

export const session = new Session()

const FRIENDLY: Partial<Record<ErrorCode, string>> = {
  email_not_verified: 'Confirm your email address first. The link is in your inbox.',
  already_in_match: "You're already in a match. Finish it before starting another.",
  rate_limited: 'Too many clicks at once. Wait a second and try again.',
  unavailable: "The server couldn't do that just now. Try again in a moment.",
}

export type Status = 'offline' | 'connecting' | 'online' | 'replaced'

class Game {
  status = $state<Status>('offline')
  queued = $state(false)
  queuedAt = $state(0)
  /** The current (or just-finished) match. */
  table = $state.raw<TableState | null>(null)
  notice = $state<string | null>(null)
  /** An action was sent and the server hasn't answered yet: don't send another. */
  pending = $state(false)
  /** The server is restarting once current matches end; no new matches meanwhile. */
  restarting = $state(false)
  /** Connected at least once since logging in (so a later 'connecting' is a reconnect). */
  everOnline = $state(false)

  private ws: WebSocket | null = null
  /** A match snapshot arrived on the current socket (so its `hello` is stale news). */
  private snapshotSeen = false
  private wanted = false
  private failures = 0
  private retryTimer: ReturnType<typeof setTimeout> | undefined
  private noticeTimer: ReturnType<typeof setTimeout> | undefined

  /** Opens the socket (and keeps it open) while logged in. */
  start() {
    this.wanted = true
    if (!this.ws) this.open()
  }

  stop() {
    this.wanted = false
    clearTimeout(this.retryTimer)
    this.ws?.close()
    this.ws = null
    this.status = 'offline'
    this.everOnline = false
    this.queued = false
    this.table = null
  }

  /** After another tab took over: take the connection back. */
  reclaim() {
    // The replaced socket may not have closed yet; forget it so a new one opens.
    const old = this.ws
    this.ws = null
    old?.close()
    this.start()
  }

  private open() {
    clearTimeout(this.retryTimer)
    this.status = 'connecting'
    const scheme = location.protocol === 'https:' ? 'wss' : 'ws'
    const ws = new WebSocket(`${scheme}://${location.host}/ws`)
    this.ws = ws
    this.snapshotSeen = false
    ws.onopen = () => {
      this.failures = 0
      this.status = 'online'
      this.everOnline = true
    }
    ws.onmessage = (ev) => {
      // A superseded socket can still deliver (e.g. its own "replaced"): ignore it.
      if (this.ws === ws) this.receive(JSON.parse(ev.data) as ServerMsg)
    }
    ws.onclose = () => {
      if (this.ws !== ws) return
      this.ws = null
      this.queued = false
      this.pending = false
      if (this.status === 'replaced' || !this.wanted) return
      this.status = 'connecting'
      this.failures += 1
      void this.retry()
    }
  }

  private async retry() {
    // The handshake fails without a valid session; after a few failures, check.
    if (this.failures % 3 === 0) {
      try {
        await session.load()
      } catch {
        /* offline: keep retrying */
      }
      if (!session.me) {
        this.stop()
        return
      }
    }
    const delay = Math.min(5000, 250 * 2 ** Math.min(this.failures, 5))
    this.retryTimer = setTimeout(() => {
      if (this.wanted && !this.ws) this.open()
    }, delay)
  }

  send(msg: ClientMsg) {
    if (this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(msg))
      return true
    }
    this.flash("That didn't send: you're not connected. Reconnecting now.")
    return false
  }

  joinQueue() {
    this.send({ type: 'queue_join' })
  }

  leaveQueue() {
    this.send({ type: 'queue_leave' })
  }

  act(action: WireAction) {
    if (this.pending) return
    if (this.send({ type: 'act', action })) this.pending = true
  }

  /** Returns whether the setting was sent (and so applied locally). */
  setAutoMuck(on: boolean) {
    if (!this.send({ type: 'set_auto_muck', on })) return false
    if (this.table) this.table = { ...this.table, autoMuck: on }
    return true
  }

  /** Leaves the result screen. */
  dismiss() {
    if (this.table?.winner !== null) this.table = null
  }

  private flash(text: string) {
    this.notice = text
    clearTimeout(this.noticeTimer)
    this.noticeTimer = setTimeout(() => (this.notice = null), 4000)
  }

  private receive(msg: ServerMsg) {
    const now = performance.now()
    // Any news about the match answers (or overtakes) an action in flight.
    if (msg.type !== 'queued' && msg.type !== 'queue_left' && msg.type !== 'restarting') {
      this.pending = false
    }
    switch (msg.type) {
      case 'hello':
        // A fresh server (or this one, if it is still draining: `restarting` follows).
        this.restarting = false
        if (session.me) {
          session.me = {
            ...session.me,
            rating: msg.rating,
            season_matches: msg.season_matches,
            email_verified: msg.email_verified,
          }
        }
        // The match ended while we were disconnected: its result will be in History.
        // (In a rare race a new match's snapshot can overtake the hello; keep it.)
        if (!msg.in_match && !this.snapshotSeen && this.table && this.table.winner === null) {
          this.table = null
          this.flash('Your match finished while you were away. The result is under Your matches.')
        }
        break
      case 'queued':
        this.queued = true
        this.queuedAt = Date.now()
        break
      case 'queue_left':
        this.queued = false
        break
      case 'match_state':
        this.snapshotSeen = true
        this.queued = false
        this.table = fromSnapshot(msg, now)
        if (router.path !== '/play') router.go('/play')
        break
      case 'event':
        if (this.table?.matchId === msg.match_id) this.table = applyEvent(this.table, msg.event)
        break
      case 'turn':
        if (this.table?.matchId === msg.match_id) this.table = applyTurn(this.table, msg.turn, now)
        break
      case 'match_ended':
        if (this.table?.matchId === msg.match_id) {
          this.table = {
            ...this.table,
            winner: msg.winner,
            rated: msg.rated,
            result: msg.ratings,
            resultSaved: true,
            turn: null,
          }
          const mine = msg.ratings?.[this.table.mySeat]
          if (mine && session.me) {
            session.me = { ...session.me, rating: mine.after }
            // The match count (and, across midnight on the 1st, the season) come from
            // the server rather than being worked out here.
            api.me().then(
              (me) => {
                if (session.me?.id === me.id) session.me = me
              },
              () => {},
            )
          }
        }
        break
      case 'error':
        this.flash(FRIENDLY[msg.code] ?? msg.message)
        break
      case 'restarting':
        this.restarting = true
        this.queued = false
        break
      case 'replaced':
        this.status = 'replaced'
        this.wanted = false
        break
    }
  }
}

export const game = new Game()
