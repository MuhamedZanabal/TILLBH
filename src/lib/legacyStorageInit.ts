import { migrateLegacyBrowserStorage } from "./legacyStorage";

// Side-effect import from main.tsx. It must run before App/i18n modules so the
// initial language read sees a migrated preference on the very first TILLBH run.
migrateLegacyBrowserStorage();
