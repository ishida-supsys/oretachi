import { createI18n } from 'vue-i18n'
import en from './en'
import ja from './ja'

export const i18n = createI18n({
  legacy: false,
  locale: 'en',
  fallbackLocale: 'en',
  messages: { en, ja },
})

export function setLocale(locale: "en" | "ja") {
  i18n.global.locale.value = locale;
}

export default i18n
