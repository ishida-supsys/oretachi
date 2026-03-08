import { createI18n } from 'vue-i18n'
import en from './en'
import ja from './ja'

export const i18n = createI18n({
  legacy: false,
  locale: 'en',
  fallbackLocale: 'en',
  messages: { en, ja },
})

export default i18n
