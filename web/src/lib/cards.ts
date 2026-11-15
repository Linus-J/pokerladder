// Card strings from the server look like "As", "Td", "9c".

const RANKS: Record<string, string> = {
  A: 'Ace',
  K: 'King',
  Q: 'Queen',
  J: 'Jack',
  T: 'Ten',
  '9': 'Nine',
  '8': 'Eight',
  '7': 'Seven',
  '6': 'Six',
  '5': 'Five',
  '4': 'Four',
  '3': 'Three',
  '2': 'Two',
}

export const SUITS: Record<string, { symbol: string; name: string }> = {
  s: { symbol: '♠', name: 'spades' },
  h: { symbol: '♥', name: 'hearts' },
  d: { symbol: '♦', name: 'diamonds' },
  c: { symbol: '♣', name: 'clubs' },
}

export interface CardFace {
  rank: string
  suit: string
  symbol: string
  label: string
}

export function face(card: string): CardFace {
  const r = card[0]
  const s = card[1]
  const suit = SUITS[s] ?? { symbol: '?', name: '?' }
  return {
    rank: r === 'T' ? '10' : r,
    suit: s,
    symbol: suit.symbol,
    label: `${RANKS[r] ?? r} of ${suit.name}`,
  }
}
