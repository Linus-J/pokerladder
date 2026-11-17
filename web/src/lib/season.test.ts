import { describe, expect, it } from 'vitest'
import { seasonEnd, seasonName, seasonOf, timeLeft } from './season'

describe('seasons', () => {
  it('names seasons by month', () => {
    expect(seasonName('2026-11')).toBe('November 2026')
    expect(seasonName('2027-01')).toBe('January 2027')
    expect(seasonName('2026-13')).toBe('2026-13')
    expect(seasonName('nonsense')).toBe('nonsense')
  })

  it('uses UTC months', () => {
    // 23:30 on 31 Dec in UTC is still December, whatever the local time zone says.
    expect(seasonOf(new Date(Date.UTC(2026, 11, 31, 23, 30)))).toBe('2026-12')
    expect(seasonOf(new Date(Date.UTC(2027, 0, 1, 0, 0)))).toBe('2027-01')
  })

  it('ends at midnight UTC on the 1st of the next month', () => {
    expect(seasonEnd('2026-11').toISOString()).toBe('2026-12-01T00:00:00.000Z')
    expect(seasonEnd('2026-12').toISOString()).toBe('2027-01-01T00:00:00.000Z')
  })

  it('says how long is left', () => {
    const end = seasonEnd('2026-11').getTime()
    const at = (ms: number) => new Date(end - ms)
    expect(timeLeft('2026-11', at(13 * 86_400_000 + 5))).toBe('ends in 13 days')
    expect(timeLeft('2026-11', at(47 * 3_600_000 + 5))).toBe('ends in 47 hours')
    expect(timeLeft('2026-11', at(3_600_000))).toBe('ends in 1 hour')
    expect(timeLeft('2026-11', at(60_000))).toBe('ends within the hour')
    expect(timeLeft('2026-11', at(0))).toBe('ended')
  })
})
