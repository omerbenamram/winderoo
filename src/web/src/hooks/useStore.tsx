import { createContext } from 'preact'
import { useContext, useState, useEffect, useCallback } from 'preact/hooks'
import type { Status, Language } from '../types'
import { api } from '../api'
import { getTranslations } from '../i18n'

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
      const data = await api.getStatus()
      // Normalize the db value (comes as negative from ESP)
      data.db = Math.abs(data.db)
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
