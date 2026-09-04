// The theme state, owned by the document rather than by either channel
// [AR5, AR6, TH26].
//
// The React hook used to keep the current theme and its subscriber list in
// module state. A plain <script> cannot reach a React module's closure, so
// on the comparison page the two pickers would each set the theme
// correctly and each fail to update the other's selection mark — on the
// very surface built to compare them.
//
// So: the truth is `document.documentElement.dataset.theme`, and a change
// is announced as one DOM event both channels listen to. Cross-tab
// following rides the same bus, because a `storage` event is translated
// into the same announcement.
//
// This file is plain ESM with no dependencies. React sits on top of it,
// not underneath.

import { THEMES, DEFAULT_THEME, STORAGE_KEY } from './theme-registry.js';

/**
 * A theme name, as a union of the eleven that exist [KT4].
 *
 * Only the OUTPUTS of this module narrowed to it. What a function accepts
 * stays lenient — `storeTheme` and `initializeTheme` still take a plain
 * string — because narrowing an input breaks a consumer that reads a theme
 * out of config or a database, which is exactly what JobTracker and
 * kp-soft do. Narrowing a return value cannot break anyone.
 * The type below is the type itself:
 *
 * Re-exported from the generated registry so a consumer can name the type
 * without importing from two places. It was `string` until 1.1.0, which
 * meant `applyTheme('formeel')` type-checked and then silently fell back
 * to `formal` at runtime.
 *
 * @typedef {import('./theme-registry.js').ThemeName} ThemeName
 */

/**
 * The event both channels listen to. A contract value: a consumer may
 * listen for it too, so it does not get renamed casually [TH26].
 */
export const THEME_EVENT = 'kp-theme-change';

// Widened back to string on purpose: this array is what the runtime check
// searches, and `includes` on a ThemeName[] refuses the unknown string we
// are asking about. The narrowing happens in the guard's return type,
// where it is earned rather than assumed.
const NAMES = /** @type {readonly string[]} */ (THEMES.map((t) => t.name));
const DARK = new Set(THEMES.filter((t) => t.dark).map((t) => t.name));

/** @param {unknown} value @returns {value is ThemeName} */
export const isTheme = (value) => typeof value === 'string' && NAMES.includes(value);

/** @param {unknown} value @returns {ThemeName | null} */
const asTheme = (value) => (isTheme(value) ? value : null);

/** @returns {ThemeName} the theme the document is currently wearing */
export function currentTheme() {
    if (typeof document === 'undefined') return DEFAULT_THEME;
    return asTheme(document.documentElement.dataset.theme) ?? DEFAULT_THEME;
}

/**
 * Put a theme on <html> and tell everyone.
 *
 * Validation lives here rather than in each caller: this is the exported
 * entry point and was the only one that did not validate, which is how an
 * unknown value used to reach the DOM through `applyTheme` while the same
 * value was rejected by the hook (AR6, adopted from the critic).
 *
 * @param {unknown} theme
 * @returns {ThemeName} the theme actually applied — DEFAULT_THEME for anything unknown
 */
export function applyTheme(theme) {
    const next = asTheme(theme) ?? DEFAULT_THEME;
    const root = document.documentElement;
    const previous = asTheme(root.dataset.theme);
    root.dataset.theme = next;
    // The `dark` class is what a consumer's existing `dark:` variants key
    // on. Kept as a contract value [TH26], derived from the token source
    // rather than from a hand-kept list: kyu believed in four dark themes
    // where there are three.
    root.classList.toggle('dark', DARK.has(next));
    if (previous !== next) {
        document.dispatchEvent(new CustomEvent(THEME_EVENT, { detail: { theme: next, previous } }));
    }
    return next;
}

/**
 * Remember the choice. Returns false when storage refused — private mode,
 * blocked storage, a full quota. The caller shows that; it is not swallowed
 * [AR6], because in a server-rendered dashboard a preference that silently
 * fails to save is indistinguishable from a broken picker.
 *
 * @param {string} theme
 * @returns {boolean} whether the choice will survive a reload
 */
export function storeTheme(theme) {
    try {
        localStorage.setItem(STORAGE_KEY, theme);
        return true;
    } catch {
        return false;
    }
}

/** @returns {ThemeName | null} the stored choice, or null if there is none or storage is unreadable */
export function storedTheme() {
    try {
        return asTheme(localStorage.getItem(STORAGE_KEY));
    } catch {
        return null;
    }
}

/**
 * Before anything renders: wear the last known choice. Six lines in a
 * consumer's <head>, deliberately ignorant of which themes are dark —
 * that knowledge lives in the generated registry [TH23].
 *
 * @param {string} [fallback]
 * @returns {ThemeName}
 */
export function initializeTheme(fallback = DEFAULT_THEME) {
    return applyTheme(storedTheme() ?? fallback);
}

/**
 * Listen for theme changes, whoever made them: this tab's React picker,
 * this tab's framework-free picker, or another tab.
 *
 * @param {(theme: ThemeName) => void} listener
 * @returns {() => void} unsubscribe
 */
export function onThemeChange(listener) {
    if (typeof document === 'undefined') return () => {};
    /** @param {Event} e */
    const onEvent = (e) => listener(/** @type {CustomEvent} */ (e).detail.theme);
    /** @param {StorageEvent} e */
    const onStorage = (e) => {
        // Another tab changed the choice. Translate it into the same
        // announcement rather than a second mechanism, so a subscriber
        // never has to know which tab a change came from.
        if (e.key !== STORAGE_KEY) return;
        const next = asTheme(e.newValue);
        if (next && next !== currentTheme()) applyTheme(next);
    };
    document.addEventListener(THEME_EVENT, onEvent);
    window.addEventListener('storage', onStorage);
    return () => {
        document.removeEventListener(THEME_EVENT, onEvent);
        window.removeEventListener('storage', onStorage);
    };
}

export { THEMES, DEFAULT_THEME, STORAGE_KEY };
