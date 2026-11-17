// Season helpers. Seasons are UTC calendar months, keyed 'YYYY-MM' by the server.

// Generated from the server's `rating` module.
export { PAIR_DAILY_LIMIT, QUALIFYING_MATCHES } from './protocol'

const MONTHS = [
  'January',
  'February',
  'March',
  'April',
  'May',
  'June',
  'July',
  'August',
  'September',
  'October',
  'November',
  'December',
]

/** '2026-11' -> 'November 2026'. Anything else is returned unchanged. */
export function seasonName(key: string): string {
  const m = /^(\d{4})-(\d{2})$/.exec(key)
  const month = m ? MONTHS[Number(m[2]) - 1] : undefined
  return m && month ? `${month} ${m[1]}` : key
}

/** The key of the season `at` falls in. */
export function seasonOf(at: Date): string {
  return `${at.getUTCFullYear()}-${String(at.getUTCMonth() + 1).padStart(2, '0')}`
}

/** When the season `key` ends (the start of the next UTC month). */
export function seasonEnd(key: string): Date {
  const [y, m] = key.split('-').map(Number)
  return new Date(Date.UTC(y, m, 1))
}

/** 'ends in 3 days' style text for how long is left of the season, from `now`. */
export function timeLeft(key: string, now: Date): string {
  const ms = seasonEnd(key).getTime() - now.getTime()
  if (ms <= 0) return 'ended'
  const hours = Math.floor(ms / 3_600_000)
  if (hours < 1) return 'ends within the hour'
  if (hours < 48) return `ends in ${hours} ${hours === 1 ? 'hour' : 'hours'}`
  return `ends in ${Math.floor(hours / 24)} days`
}
