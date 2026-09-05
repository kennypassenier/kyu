// The no-flash snippet [TH23].
//
// Runs before first paint, in the document head, before any stylesheet
// has painted a light background under a visitor who chose a dark theme.
// Six lines, and deliberately ignorant of which themes are dark: it only
// copies the stored name onto <html>, and the stylesheet does the rest.
// Knowing which themes are dark is the registry's job, and the registry
// is generated from the token source — a snippet that carried its own
// list is exactly how kyu came to believe in four dark themes.
//
// A consumer with a bundler imports this and calls applyStoredTheme()
// as early as their bundle runs. A consumer rendering HTML from a server
// inlines the snippet instead, because a module arrives too late to
// prevent the flash it exists to prevent.
//
// Pure since 3.0.0 [KT6]: importing this file does nothing. The first
// version read localStorage and wrote <html data-theme> on import, with
// the key and the attribute as literals in two places — a consumer who
// changed either got a snippet silently reading the wrong one.

import { STORAGE_KEY } from './theme-registry.js';

/** The attribute the stylesheet keys on. A contract value [TH26]. */
export const THEME_ATTRIBUTE = 'data-theme';

/**
 * The snippet to inline inside <script> in <head>, before the stylesheet
 * link. Plain ES5, no imports, no dependency on this package being loaded.
 *
 * @param {{ key?: string, attribute?: string }} [options]
 * @returns {string}
 */
export function noFlashSnippet({ key = STORAGE_KEY, attribute = THEME_ATTRIBUTE } = {}) {
    return `(function () {
    try {
        var t = localStorage.getItem(${JSON.stringify(key)});
        if (t) document.documentElement.setAttribute(${JSON.stringify(attribute)}, t);
    } catch (e) {}
})();`;
}

/** The snippet with the defaults, for the common case. */
export const NO_FLASH_SNIPPET = noFlashSnippet();

/**
 * What the snippet does, as a function: copy the stored theme onto the
 * document element. Returns the name applied, or null.
 *
 * @param {{ key?: string, attribute?: string, root?: Element }} [options]
 * @returns {string | null}
 */
export function applyStoredTheme({ key = STORAGE_KEY, attribute = THEME_ATTRIBUTE, root } = {}) {
    if (typeof document === 'undefined') return null;
    try {
        const stored = localStorage.getItem(key);
        if (!stored) return null;
        (root ?? document.documentElement).setAttribute(attribute, stored);
        return stored;
    } catch {
        // Blocked storage: the document keeps whatever theme it was served with.
        return null;
    }
}
