import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import en from "./en.json";

// English is the only shipped locale. Strings still go through i18next so a
// locale can be added later without touching every call site.
export const i18nReady = i18n.use(initReactI18next).init({
  resources: {
    en: { translation: en },
  },
  lng: "en",
  fallbackLng: "en",
  interpolation: { escapeValue: false },
});

export default i18n;
