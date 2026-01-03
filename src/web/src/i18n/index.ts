import type { Language } from '../types'
import enUS from './en-US.json'
import deDE from './de-DE.json'
import esES from './es-ES.json'
import frFR from './fr-FR.json'
import ptBR from './pt-BR.json'

const translations: Record<Language, typeof enUS> = {
  'en-US': enUS,
  'de-DE': deDE,
  'es-ES': esES,
  'fr-FR': frFR,
  'pt-BR': ptBR,
}

export const languages: { code: Language; name: string }[] = [
  { code: 'en-US', name: 'English' },
  { code: 'de-DE', name: 'Deutsch' },
  { code: 'es-ES', name: 'Español' },
  { code: 'fr-FR', name: 'Français' },
  { code: 'pt-BR', name: 'Português' },
]

export function getTranslations(lang: Language) {
  return translations[lang] || translations['en-US']
}
