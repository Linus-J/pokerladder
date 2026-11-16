// JSON calls to the server's REST API. The session lives in an HttpOnly cookie, so
// there is no token handling here.

import type {
  HandHistory,
  LeaderboardRow,
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
    throw new ApiError(
      res.status,
      data?.error ?? 'error',
      data?.message ?? `Request failed (${res.status})`,
    )
  }
  return data as T
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
  leaderboard: () => call<LeaderboardRow[]>('GET', '/api/leaderboard'),
  matches: () => call<MatchSummary[]>('GET', '/api/matches'),
  hands: (matchId: number) => call<HandHistory[]>('GET', `/api/matches/${matchId}/hands`),
}
