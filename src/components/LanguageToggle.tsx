import { getLang, switchLang } from "../i18n";

/** Compact locale selector. Language selection does not reload, navigate or mutate cart state. */
export function LanguageToggle() {
  const active = getLang();
  return (
    <div className="lang-switch" role="group" aria-label="Language" data-testid="lang-toggle">
      <button type="button" lang="en" aria-pressed={active === "en"} className={active === "en" ? "active" : ""} onClick={() => switchLang("en")}>EN</button>
      <span aria-hidden="true">|</span>
      <button type="button" lang="ar" aria-pressed={active === "ar"} className={active === "ar" ? "active" : ""} onClick={() => switchLang("ar")}>AR</button>
    </div>
  );
}
