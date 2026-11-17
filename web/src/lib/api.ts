// JSON calls to the server's REST API. The session lives in an HttpOnly cookie, so
// there is no token handling here.

import type {
  HandHistory,
  Standings,
  MatchSummary,
  MeResponse,
  RegisterResponse,
} from './protocol'

export class ApiError extends Error {
  constructor(
    public status: number,
    public code: string,
    message: string,
  ) {
    super(message)
  }
}

async function call<T>(method: string, path: string, body?: unknown): Promise<T> {
  let res: Response
  try {
    res = await fetch(path, {
      method,
      credentials: 'same-origin',
      // Always JSON: the server requires it on POSTs (part of its CSRF defence).
      headers: { 'content-type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    })
  } catch {
    throw new ApiError(0, 'network', "Can't reach the server. Check your connection.")
  }
  if (res.status === 204) return undefined as T
  const data = await res.json().catch(() => null)
  if (!res.ok) {
    const code: string = data?.error ?? 'error'
    const message =
      FRIENDLY[code] ??
      (data?.message
        ? sentence(data.message)
        : `The server couldn't handle that (error ${res.status}). Try again in a moment.`)
    throw new ApiError(res.status, code, message)
  }
  return data as T
}

/** Clearer wording for some server errors (the rest are shown as sentences). */
const FRIENDLY: Record<string, string> = {
  bad_credentials: "That name or email and password don't match an account.",
  bad_token: 'This link has expired or has already been used.',
  internal: 'Something broke on our side. Try again in a moment.',
  rate_limited: 'Too many attempts. Wait a minute, then try again.',
}

/** "that name is taken" -> "That name is taken." */
function sentence(s: string): string {
  const t = s.trim()
  if (!t) return t
  return t[0].toUpperCase() + t.slice(1) + (/[.!?]$/.test(t) ? '' : '.')
}

export const api = {
  me: () => call<MeResponse>('GET', '/api/me'),
  login: (login: string, password: string) =>
    call<MeResponse>('POST', '/api/login', { login, password }),
  logout: () => call<void>('POST', '/api/logout', {}),
  register: (name: string, email: string, password: string) =>
    call<RegisterResponse>('POST', '/api/register', { name, email, password }),
  verifyEmail: (token: string) => call<void>('POST', '/api/verify-email', { token }),
  resendVerification: () => call<void>('POST', '/api/resend-verification', {}),
  requestPasswordReset: (email: string) =>
    call<void>('POST', '/api/password-reset/request', { email }),
  confirmPasswordReset: (token: string, password: string) =>
    call<void>('POST', '/api/password-reset/confirm', { token, password }),
  leaderboard: (season?: string) =>
    call<Standings>('GET', season ? `/api/leaderboard?season=${encodeURIComponent(season)}` : '/api/leaderboard'),
  seasons: () => call<string[]>('GET', '/api/seasons'),
  matches: () => call<MatchSummary[]>('GET', '/api/matches'),
  hands: (matchId: number) => call<HandHistory[]>('GET', `/api/matches/${matchId}/hands`),
}
