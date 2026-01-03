import type { Status, UpdatePayload } from './types'

function getBaseUrl(): string {
  const href = window.location.href
  if (href.includes('127.0.0.1') || href.includes('localhost')) {
    // Dev mode - adjust if your ESP32 is at a different IP
    return 'http://192.168.4.1/api/'
  }
  const sanitized = href.endsWith('/') ? href.slice(0, -1) : href
  return sanitized + '/api/'
}

const BASE_URL = getBaseUrl()

async function request<T>(endpoint: string, options?: RequestInit): Promise<T> {
  const res = await fetch(BASE_URL + endpoint, {
    ...options,
    headers: {
      'Content-Type': 'application/json',
      ...options?.headers,
    },
  })
  if (!res.ok) throw new Error(`HTTP ${res.status}`)
  // Some endpoints return 204 No Content
  if (res.status === 204) return {} as T
  return res.json()
}

export const api = {
  getStatus: () => request<Status>('status'),

  updatePower: (enabled: boolean) =>
    request<void>('power', {
      method: 'POST',
      body: JSON.stringify({ winderEnabled: enabled ? 1 : 0 }),
    }),

  updateTimer: (enabled: boolean) =>
    request<void>(`timer?timerEnabled=${enabled ? 1 : 0}`, { method: 'POST' }),

  updateSettings: (payload: UpdatePayload) =>
    request<void>('update', {
      method: 'POST',
      body: JSON.stringify(payload),
    }),

  reset: () => request<void>('reset'),
}
