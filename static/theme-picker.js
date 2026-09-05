// The framework-free theme picker [T1, TH8, TH63].
//
// No custom element and no rendering: the consumer's server writes the
// markup, this attaches the behaviour. kyu and almanac render HTML from a
// Rust binary and have no npm step, so a component that only exists after
// JavaScript runs would leave them with an empty box on first paint.
//
// The markup it expects:
//
//   <div data-kp-theme-picker>
//     <button type="button" data-kp-theme="formal">
//       <span class="kp-swatch" data-theme="formal"></span> Formal
//     </button>
//     ...
//   </div>
//   <p data-kp-theme-status hidden></p>
//
// Or the same thing inside a menu — an icon button with a dropdown, the
// shape the consuming projects preferred. `themeMenuMarkup()` below
// writes it for you; the behaviour is identical, because the attribute
// is what this module attaches to, not the layout.
//
// Every attribute above is a contract value [TH26]. The swatch previews a
// theme without activating it by wearing that theme's own token block —
// `data-theme` on any element, not only on <html> — so it reads the live
// colours instead of a copy kept in step by hand [AR11].
//
// Pure since 3.0.0 [KT6]: importing this file attaches nothing. Call
// attachThemePickers() when the markup is in the DOM, or load js/auto.js.

import { applyTheme, currentTheme, initializeTheme, onThemeChange, storeTheme, THEMES } from './theme-core.js';
import { getStrings } from './strings.js';

const PICKER = '[data-kp-theme-picker]';
const OPTION = '[data-kp-theme]';
const STATUS = '[data-kp-theme-status]';

/** Dispatched on the picker, bubbling, when a person chose a theme here: `{ theme, stored }`. */
export const PICK_EVENT = 'kp-theme-pick';

/**
 * Storage refused, so the choice will not survive a reload. Said out loud
 * rather than swallowed [AR6]: in a server-rendered dashboard every click
 * is a new page load, and a preference that quietly fails to save looks
 * exactly like a broken picker.
 */
const saveFailedText = () => getStrings().themeSaveFailed;

/** @param {ParentNode} root @param {boolean} failed */
function showSaveState(root, failed) {
    for (const el of root.querySelectorAll(STATUS)) {
        el.textContent = failed ? saveFailedText() : '';
        /** @type {HTMLElement} */ (el).hidden = !failed;
    }
}

/** @param {ParentNode} root @param {string} theme */
function markSelection(root, theme) {
    for (const el of root.querySelectorAll(OPTION)) {
        const button = /** @type {HTMLElement} */ (el);
        const selected = button.dataset.kpTheme === theme;
        button.setAttribute('aria-pressed', String(selected));
        // data-selected is the observable both channels share, so one
        // behaviour suite can assert against either mount [AR7]. The ARIA
        // state beside it is each channel's own idiom.
        button.dataset.selected = String(selected);
        // A class as well as the ARIA state: the state is for assistive
        // technology, the class is what CSS can style without relying on
        // an attribute selector a consumer may not expect.
        button.classList.toggle('is-selected', selected);
    }
}

/** @param {ParentNode} root */
function clearMarks(root) {
    for (const el of root.querySelectorAll(OPTION)) {
        el.removeAttribute('aria-pressed');
        el.removeAttribute('data-selected');
        el.classList.remove('is-selected');
    }
}

/**
 * Attach the behaviour to every picker under `root`, and keep them all in
 * step with each other and with any React picker on the same page — they
 * share one bus, so neither channel needs to know the other exists [AR5].
 *
 * Safe to call twice: a picker already attached is skipped. The returned
 * detach restores the marks it made; `refresh()` re-marks options added
 * after attach, which the idempotency guard would otherwise skip.
 *
 * @param {ParentNode} [root]
 * @param {{ persist?: boolean, closePopover?: boolean, status?: ParentNode | null }} [options]
 *   persist: store the choice (default true); closePopover: close an
 *   enclosing popover after a choice (default true); status: where the
 *   save-failed message lives (default: the picker's parent).
 * @returns {(() => void) & { refresh: () => void }} detach
 */
export function attachThemePickers(root = document, { persist = true, closePopover = true, status = null } = {}) {
    /** @type {(() => void)[]} */
    const cleanups = [];
    /** @type {HTMLElement[]} */
    const pickers = [];

    for (const el of root.querySelectorAll(PICKER)) {
        const picker = /** @type {HTMLElement} */ (el);
        if (picker.dataset.kpThemeAttached === '1') continue;
        picker.dataset.kpThemeAttached = '1';
        pickers.push(picker);

        /** @param {Event} event */
        const onClick = (event) => {
            const target = /** @type {HTMLElement} */ (event.target);
            const option = target.closest(OPTION);
            if (!option || !picker.contains(option)) return;
            const next = /** @type {HTMLElement} */ (option).dataset.kpTheme;
            if (!next) return;
            const applied = applyTheme(next);
            const stored = persist ? storeTheme(applied) : true;
            showSaveState(status ?? picker.parentNode ?? document, !stored);
            picker.dispatchEvent(new CustomEvent(PICK_EVENT, { bubbles: true, detail: { theme: applied, stored } }));
            // A picker inside a menu closes it: leaving the menu open
            // after a choice makes it look as though the click missed.
            /** @type {HTMLElement | null} */
            const popover = closePopover ? picker.closest('[popover]') : null;
            if (popover && popover.matches(':popover-open')) popover.hidePopover();
        };

        picker.addEventListener('click', onClick);
        // The no-flash snippet copies the stored name onto <html> without
        // reading it, because knowing the theme list is not its job
        // [TH23]. This is the first moment anything can check it, so an
        // unknown value becomes the default here rather than surviving as
        // a data-theme no stylesheet answers to.
        markSelection(picker, initializeTheme(currentTheme()));
        cleanups.push(() => {
            picker.removeEventListener('click', onClick);
            delete picker.dataset.kpThemeAttached;
            clearMarks(picker);
        });
    }

    // One subscription for the whole call, not one per picker: a change
    // announced on the bus updates every mark, including marks belonging
    // to a React picker's neighbour.
    const stop = onThemeChange((theme) => {
        for (const el of (root === document ? document : root).querySelectorAll(PICKER)) markSelection(el, theme);
    });
    cleanups.push(stop);

    const detach = () => {
        for (const c of cleanups) c();
    };
    return Object.assign(detach, {
        refresh: () => {
            for (const picker of pickers) markSelection(picker, currentTheme());
        },
    });
}

/**
 * The list a consumer's template needs to render the options. Exported so
 * a page built without a server-side theme list can still write the
 * markup, and so the showcase does not hand-type seven names.
 */
export { THEMES };

/** @param {string} text */
function escapeHtml(text) {
    return text.replace(/[&<>"']/g, (ch) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[ch] ?? ch);
}

/** The icon the menu button wears by default; pass your own SVG string to `icon`. */
export const THEME_MENU_ICON =
    `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">` +
    `<circle cx="13.5" cy="6.5" r=".5" fill="currentColor"/><circle cx="17.5" cy="10.5" r=".5" fill="currentColor"/>` +
    `<circle cx="8.5" cy="7.5" r=".5" fill="currentColor"/><circle cx="6.5" cy="12.5" r=".5" fill="currentColor"/>` +
    `<path d="M12 2C6.5 2 2 6.5 2 12s4.5 10 10 10c.9 0 1.6-.7 1.6-1.6 0-.4-.2-.8-.4-1.1-.3-.3-.4-.7-.4-1.1 0-.9.7-1.6 1.6-1.6H16c3.3 0 6-2.7 6-6 0-4.9-4.5-8.6-10-8.6z"/>` +
    `</svg>`;

/**
 * The options of a picker, as markup — grouped light and dark [TH63].
 *
 * Each group is a `<li role="presentation">` carrying a small heading
 * and its own list, so the menu reads "Light: Formal, Light, … — Dark:
 * Dark, Cyberpunk, …" rather than eleven names in one run. The grouping
 * comes from the registry's `dark` flag, which is generated from the
 * tokens, so this cannot disagree with the stylesheet about which is
 * which. Pass `grouped: false` for one flat list.
 *
 * @param {{ themes?: readonly import('./theme-registry.js').ThemeRecord[], labels?: Partial<Record<string, string>>, grouped?: boolean, groupLabels?: { light?: string, dark?: string } }} [options]
 * @returns {string}
 */
export function themeOptionsMarkup({ themes = THEMES, labels = {}, grouped = true, groupLabels = {} } = {}) {
    const s = getStrings();
    /** @param {import('./theme-registry.js').ThemeRecord} t */
    const option = (t) =>
        `<li><button type="button" data-kp-theme="${escapeHtml(t.name)}">` +
        `<span class="kp-swatch" data-theme="${escapeHtml(t.name)}"></span>${escapeHtml(labels[t.name] ?? t.label)}</button></li>`;
    if (!grouped) return themes.map(option).join('');
    const light = themes.filter((t) => !t.dark);
    const dark = themes.filter((t) => t.dark);
    /** @param {string} heading @param {typeof themes} list @param {'light' | 'dark'} kind */
    const group = (heading, list, kind) =>
        list.length === 0
            ? ''
            : `<li role="presentation" class="kp-theme-group" data-kp-theme-group="${kind}">` +
              `<span class="kp-theme-group__label" aria-hidden="true">${escapeHtml(heading)}</span>` +
              `<ul class="kp-theme-group__list" aria-label="${escapeHtml(heading)}">${list.map(option).join('')}</ul></li>`;
    return group(groupLabels.light ?? s.themeGroupLight, light, 'light') + group(groupLabels.dark ?? s.themeGroupDark, dark, 'dark');
}

/**
 * The icon button with a dropdown, as markup [S2].
 *
 * Returned as a string rather than rendered, so a server can print it
 * into a template and a page can insert it — the same choice T1 makes
 * everywhere else in this channel. The id must be unique on the page;
 * pass one when there are two menus. Everything a person reads is
 * escaped; the icon is trusted markup you pass.
 *
 * @param {{ id?: string, label?: string, icon?: string, themes?: readonly import('./theme-registry.js').ThemeRecord[], labels?: Partial<Record<string, string>>, grouped?: boolean, groupLabels?: { light?: string, dark?: string }, className?: string }} [options]
 * @returns {string}
 */
export function themeMenuMarkup({
    id = 'kp-theme-menu',
    label = getStrings().themePicker,
    icon = THEME_MENU_ICON,
    themes,
    labels,
    grouped,
    groupLabels,
    className = '',
} = {}) {
    const safeId = escapeHtml(id);
    const safeLabel = escapeHtml(label);
    const classes = `kp-theme-menu ${escapeHtml(className)}`.trim();
    return (
        `<span class="${classes}">` +
        `<button type="button" class="kp-icon-button" popovertarget="${safeId}" aria-label="${safeLabel}" style="anchor-name: --${safeId}">` +
        icon +
        `</button>` +
        `<div popover="auto" id="${safeId}" class="kp-popover" style="position-anchor: --${safeId}">` +
        `<ul class="kp-menu" data-kp-theme-picker aria-label="${safeLabel}">${themeOptionsMarkup({ themes, labels, grouped, groupLabels })}</ul>` +
        `</div></span>`
    );
}
