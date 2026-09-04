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
// A consumer with a bundler can import this. A consumer rendering HTML
// from a server inlines NO_FLASH_SNIPPET instead, because a module
// arrives too late to prevent the flash it exists to prevent.

/**
 * Inline this inside <script> in <head>, before the stylesheet link.
 * Plain ES5, no imports, no dependency on this package being loaded.
 */
export const NO_FLASH_SNIPPET = `(function () {
    try {
        var t = localStorage.getItem('theme');
        if (t) document.documentElement.dataset.theme = t;
    } catch (e) {}
})();`;

if (typeof document !== 'undefined') {
    try {
        const stored = localStorage.getItem('theme');
        if (stored) document.documentElement.dataset.theme = stored;
    } catch {
        // Blocked storage: the document keeps whatever theme it was served with.
    }
}
