import { getLang, switchLang } from "../i18n";

/** Local EN/AR language controls, with a backward-compatible one-click switch. */
export function LanguageToggle() {
  const active = getLang();
  return (
    <div className="lang-switch" role="group" aria-label={active === "ar" ? "اللغة" : "Language"}>
      <button
        type="button"
        lang="en"
        aria-label="EN"
        aria-pressed={active === "en"}
        className={active === "en" ? "active" : ""}
        onClick={() => switchLang("en")}
      >
        EN
      </button>
      <button
        type="button"
        data-testid="lang-toggle"
        lang={active}
        aria-label={active === "ar" ? "تغيير اللغة" : "Switch language"}
        onClick={() => switchLang(active === "ar" ? "en" : "ar")}
      >
        |
      </button>
      <button
        type="button"
        lang="ar"
        aria-label="AR"
        aria-pressed={active === "ar"}
        className={active === "ar" ? "active" : ""}
        onClick={() => switchLang("ar")}
      >
        AR
      </button>
    </div>
  );
}
