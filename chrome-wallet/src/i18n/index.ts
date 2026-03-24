import i18n from 'i18next'
import { initReactI18next } from 'react-i18next'
import en from './locales/en'
import ko from './locales/ko'
import ja from './locales/ja'
import zh from './locales/zh'
import es from './locales/es'

export const LANGUAGES = [
  { code: 'en', label: 'English', flag: '🇺🇸' },
  { code: 'ko', label: '한국어',   flag: '🇰🇷' },
  { code: 'ja', label: '日本語',   flag: '🇯🇵' },
  { code: 'zh', label: '中文',     flag: '🇨🇳' },
  { code: 'es', label: 'Español',  flag: '🇪🇸' },
]

i18n
  .use(initReactI18next)
  .init({
    resources: {
      en: { translation: en },
      ko: { translation: ko },
      ja: { translation: ja },
      zh: { translation: zh },
      es: { translation: es },
    },
    lng: 'en',
    fallbackLng: 'en',
    interpolation: { escapeValue: false },
  })

/** Load persisted language from chrome.storage and apply it */
export async function loadSavedLanguage() {
  try {
    const data = await chrome.storage.local.get('language')
    if (data.language) {
      await i18n.changeLanguage(data.language)
    }
  } catch {
    // not in chrome context (e.g. tests)
  }
}

export default i18n
