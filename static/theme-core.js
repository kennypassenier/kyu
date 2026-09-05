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
//
// Since 3.0.0 [KT6]: where the theme lives is a choice — the root
// element, the class that marks a dark theme and the storage key are
// settable once with configureTheme() or per call; a change can be
// refused by a listener on BEFORE_THEME_EVENT; the change event bubbles
// from the root so a scoped listener can catch it; a strict apply
// reports an unknown name instead of silently substituting the default;
// and the cross-tab subscription can be declined.

import { THEMES, DEFAULT_THEME, STORAGE_KEY } from './theme-registry.js';

/**
 * A theme name, as a union of the eleven that exist [KT4].
 *
 * Only the OUTPUTS of this module narrowed to it. What a function accepts
 * stays lenient — `storeTheme` and `initializeTheme` still take a plain
 * string — because narrowing an input breaks a consumer that reads a theme
 * out of config or a database, which is exactly what JobTracker and
 * kp-soft do. Narrowing a return value cannot break anyone.
 *
 * @typedef {import('./theme-registry.js').ThemeName} ThemeName
 */

/**
 * The event both channels listen to. A contract value: a consumer may
 * listen for it too, so it does not get renamed casually [TH26]. It
 * bubbles from the root, so `document.addEventListener` sees it as it
 * always did, and so does a listener on the root itself.
 */
export const THEME_EVENT = 'kp-theme-change';
/** Fired before a change, cancelable: `{ theme, previous }`. preventDefault() keeps the current theme. */
export const BEFORE_THEME_EVENT = 'kp-theme-before-change';

/** @typedef {{ root?: Element, darkClass?: string | null, storageKey?: string }} ThemeConfig */

/** The document-wide defaults, settable once by a consumer. */
const config = { root: /** @type {Element | null} */ (null), darkClass: /** @type {string | null} */ ('dark'), storageKey: STORAGE_KEY };

/**
 * Set the defaults once: which element wears the theme (default: the
 * document element), which class marks a dark theme (default `dark`; null
 * for none), and the storage key.
 *
 * @param {ThemeConfig} next
 */
export function configureTheme(next) {
    if (next.root !== undefined) config.root = next.root;
    if (next.darkClass !== undefined) config.darkClass = next.darkClass;
    if (next.storageKey !== undefined) config.storageKey = next.storageKey;
}

/** @param {Element | undefined} root */
const rootOf = (root) => root ?? config.root ?? document.documentElement;

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

/**
 * @param {{ root?: Element }} [options]
 * @returns {ThemeName} the theme the root is currently wearing
 */
export function currentTheme({ root } = {}) {
    if (typeof document === 'undefined') return DEFAULT_THEME;
    return asTheme(rootOf(root).getAttribute('data-theme')) ?? DEFAULT_THEME;
}

/**
 * Put a theme on the root and tell everyone.
 *
 * Validation lives here rather than in each caller: this is the exported
 * entry point and was the only one that did not validate, which is how an
 * unknown value used to reach the DOM through `applyTheme` while the same
 * value was rejected by the hook (AR6, adopted from the critic).
 *
 * @param {unknown} theme
 * @param {{ root?: Element, darkClass?: string | null, strict?: boolean, announce?: boolean }} [options]
 *   strict: throw on an unknown name instead of substituting the default; announce: dispatch the events (default true)
 * @returns {ThemeName} the theme actually applied — DEFAULT_THEME for anything unknown
 */
export function applyTheme(theme, { root, darkClass, strict = false, announce = true } = {}) {
    const known = asTheme(theme);
    if (known === null && strict) throw new RangeError(`kp-themes: "${String(theme)}" is not a theme`);
    const next = known ?? DEFAULT_THEME;
    const element = rootOf(root);
    const previous = asTheme(element.getAttribute('data-theme'));
    if (announce && previous !== next) {
        const ask = new CustomEvent(BEFORE_THEME_EVENT, { bubbles: true, cancelable: true, detail: { theme: next, previous } });
        if (!element.dispatchEvent(ask)) return previous ?? DEFAULT_THEME;
    }
    element.setAttribute('data-theme', next);
    // The `dark` class is what a consumer's existing `dark:` variants key
    // on. Kept as a contract value [TH26], derived from the token source
    // rather than from a hand-kept list: kyu believed in four dark themes
    // where there are three.
    const cls = darkClass === undefined ? config.darkClass : darkClass;
    if (cls) element.classList.toggle(cls, DARK.has(next));
    if (announce && previous !== next) {
        element.dispatchEvent(new CustomEvent(THEME_EVENT, { bubbles: true, detail: { theme: next, previous, root: element } }));
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
 * @param {{ key?: string, storage?: Storage }} [options]
 * @returns {boolean} whether the choice will survive a reload
 */
export function storeTheme(theme, { key, storage } = {}) {
    try {
        (storage ?? localStorage).setItem(key ?? config.storageKey, theme);
        return true;
    } catch {
        return false;
    }
}

/**
 * @param {{ key?: string, storage?: Storage }} [options]
 * @returns {ThemeName | null} the stored choice, or null if there is none or storage is unreadable
 */
export function storedTheme({ key, storage } = {}) {
    try {
        return asTheme((storage ?? localStorage).getItem(key ?? config.storageKey));
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
 * @param {{ root?: Element, key?: string }} [options]
 * @returns {ThemeName}
 */
export function initializeTheme(fallback = DEFAULT_THEME, { root, key } = {}) {
    return applyTheme(storedTheme({ key }) ?? fallback, { root });
}

/**
 * Listen for theme changes, whoever made them: this tab's React picker,
 * this tab's framework-free picker, or — unless declined — another tab.
 *
 * @param {(theme: ThemeName, detail: { previous: ThemeName | null, root: Element }) => void} listener
 * @param {{ crossTab?: boolean, root?: Element, key?: string }} [options]
 * @returns {() => void} unsubscribe
 */
export function onThemeChange(listener, { crossTab = true, root, key } = {}) {
    if (typeof document === 'undefined') return () => {};
    const target = root ?? document;
    /** @param {Event} e */
    const onEvent = (e) => {
        const detail = /** @type {CustomEvent} */ (e).detail;
        listener(detail.theme, { previous: detail.previous, root: detail.root });
    };
    /** @param {StorageEvent} e */
    const onStorage = (e) => {
        // Another tab changed the choice. Translate it into the same
        // announcement rather than a second mechanism, so a subscriber
        // never has to know which tab a change came from.
        if (e.key !== (key ?? config.storageKey)) return;
        const next = asTheme(e.newValue);
        if (next && next !== currentTheme({ root })) applyTheme(next, { root });
    };
    target.addEventListener(THEME_EVENT, onEvent);
    if (crossTab) window.addEventListener('storage', onStorage);
    return () => {
        target.removeEventListener(THEME_EVENT, onEvent);
        if (crossTab) window.removeEventListener('storage', onStorage);
    };
}

export { THEMES, DEFAULT_THEME, STORAGE_KEY };
