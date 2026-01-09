import { createContext } from 'preact'
import { useContext, useState, useEffect, useCallback } from 'preact/hooks'
import type { Status, Language } from '../types'
import { api } from '../api'
import { getTranslations } from '../i18n'

function toNumber(value: unknown, fallback = 0): number {
  if (typeof value === 'number') return Number.isFinite(value) ? value : fallback
  if (typeof value === 'string') {
    const n = Number(value)
    return Number.isFinite(n) ? n : fallback
  }
  if (typeof value === 'boolean') return value ? 1 : 0
  return fallback
}

function toBoolean(value: unknown, fallback = false): boolean {
  if (typeof value === 'boolean') return value
  if (typeof value === 'number') return value !== 0
  if (typeof value === 'string') {
    const s = value.trim().toLowerCase()
    if (s === '1' || s === 'true' || s === 'yes' || s === 'on') return true
    if (s === '0' || s === 'false' || s === 'no' || s === 'off') return false
    const n = Number(value)
    if (Number.isFinite(n)) return n !== 0
  }
  return fallback
}

interface Store {
  status: Status | null
  loading: boolean
  error: string | null
  language: Language
  translations: ReturnType<typeof getTranslations>
  setLanguage: (lang: Language) => void
  refresh: () => Promise<void>
  setPower: (enabled: boolean) => Promise<void>
}

const StoreContext = createContext<Store | null>(null)

export function StoreProvider({ children }: { children: preact.ComponentChildren }) {
  const [status, setStatus] = useState<Status | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [language, setLanguageState] = useState<Language>(() => {
    const saved = localStorage.getItem('selectedLanguage') as Language
    return saved || 'en-US'
  })

  const translations = getTranslations(language)

  const setLanguage = useCallback((lang: Language) => {
    setLanguageState(lang)
    localStorage.setItem('selectedLanguage', lang)
  }, [])

  const refresh = useCallback(async () => {
    try {
      setLoading(true)
      setError(null)
      const raw = (await api.getStatus()) as unknown as Record<string, unknown>

      // Firmware API intentionally serializes many numbers as strings (for backward-compat).
      // Normalize to the strongly-typed `Status` we use in the UI.
      const data: Status = {
        ...(raw as unknown as Status),
        batteryLevel: raw.batteryLevel == null ? null : toNumber(raw.batteryLevel),
        db: Math.abs(toNumber(raw.db)),
        rotationsPerDay: toNumber(raw.rotationsPerDay),
        startTimeEpoch: toNumber(raw.startTimeEpoch),
        currentTimeEpoch: toNumber(raw.currentTimeEpoch),
        estimatedRoutineFinishEpoch: toNumber(raw.estimatedRoutineFinishEpoch),
        winderEnabled: toNumber(raw.winderEnabled),
        timerEnabled: toNumber(raw.timerEnabled),
        screenSleep: toBoolean(raw.screenSleep),
        screenScheduleEnabled: toBoolean(raw.screenScheduleEnabled),
        screenEquipped: toBoolean(raw.screenEquipped),
        customWindDuration: toNumber(raw.customWindDuration, 180),
        customWindPauseDuration: toNumber(raw.customWindPauseDuration, 15),
        customDurationInSecondsToCompleteOneRevolution: toNumber(raw.customDurationInSecondsToCompleteOneRevolution, 8),
        gmtOffset: toNumber(raw.gmtOffset),
        dst: toBoolean(raw.dst),
      }

      setStatus(data)
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to fetch status')
    } finally {
      setLoading(false)
    }
  }, [])

  const setPower = useCallback(async (enabled: boolean) => {
    // Optimistic update - immediately show the new state
    setStatus((prev) => prev ? { ...prev, winderEnabled: enabled ? 1 : 0 } : prev)
    try {
      await api.updatePower(enabled)
      // Small delay to let the controller process the command before refresh
      await new Promise((r) => setTimeout(r, 200))
      await refresh()
    } catch (e) {
      // Revert on error
      setStatus((prev) => prev ? { ...prev, winderEnabled: enabled ? 0 : 1 } : prev)
      setError(e instanceof Error ? e.message : 'Failed to update power')
    }
  }, [refresh])

  useEffect(() => {
    refresh()
  }, [refresh])

  return (
    <StoreContext.Provider
      value={{ status, loading, error, language, translations, setLanguage, refresh, setPower }}
    >
      {children}
    </StoreContext.Provider>
  )
}

export function useStore() {
  const ctx = useContext(StoreContext)
  if (!ctx) throw new Error('useStore must be used within StoreProvider')
  return ctx
}
