// The component contracts, framework-free [L7, DI4, DI10].
//
// Kenny's standing rule 31, and DI10's own wording: drive it from
// attributes rather than per-button code, so a new button gets the
// behaviour by declaring it instead of by someone remembering.
//
// Two contracts are enforced here, both of them the kind that a review
// catches once and then stops catching:
//
//   1. A destructive button must offer an undo or a confirmation
//      (SC 3.3.4 Error Prevention, AA). It is an OR, not an AND.
//   2. A badge or alert carrying a semantic colour must also say what it
//      means in text (DI4). Seven pale plates are one plate to someone
//      who cannot tell the colours apart.
//
// A violation is reported, loudly, and the offending control is disarmed
// rather than left to delete something. It is not thrown: one bad button
// on a dashboard should not take the page down with it.
//
// Recoverable since 3.0.0 [KT6, decision D7]. The first version disabled
// a consumer's button, forgot what the button had been, offered no way
// to detach and never looked again — so markup that arrived a moment too
// late stayed dead for the life of the page. Now enforcement records
// what it changed, returns a detach that puts it back, re-evaluates when
// called again, exempts what the consumer marks, and says what it says
// through the dictionary. The rule is the same; the ownership moved.

import { getStrings } from './strings.js';

/** Dispatched on the offending element, bubbling, with the Violation as detail. */
export const VIOLATION_EVENT = 'kp-contract-violation';
/** Dispatched on a destructive button when its first click armed it. */
export const ARM_EVENT = 'kp-confirm-arm';
/** Dispatched when an armed button disarms without acting: timeout, blur, or detach. */
export const DISARM_EVENT = 'kp-confirm-disarm';

/** @typedef {'DI10' | 'DI4'} Rule */
/** @typedef {{ rule: Rule, element: Element, message: string }} Violation */

/**
 * The confirmation obstacle.
 *
 * DI10's evidence, which is not the folklore: "undo beats confirmation"
 * has no controlled study behind it, while confirmations carrying a small
 * obstacle still worked for 44-74% of users after some twenty exposures,
 * against 20% or less for purely visual ones. So the first click does not
 * act — it arms, changes the label to the phrase the consumer chose, and
 * disarms itself again after a few seconds if nothing follows.
 *
 * Configurable rather than hard-coded, because it is an operational knob:
 * a dashboard whose users delete all day wants a longer window than a
 * settings page. Per element too, as `data-kp-confirm-ms`.
 */
export const CONFIRM_WINDOW_MS = 4000;

/** Markup the consumer excludes from enforcement: `data-kp-contract-ignore`. */
export const EXEMPT = '[data-kp-contract-ignore]';

/**
 * @param {ParentNode} root
 * @param {{ rules?: Rule[], exempt?: string }} [options]
 * @returns {Violation[]}
 */
export function findViolations(root = document, { rules = ['DI10', 'DI4'], exempt = EXEMPT } = {}) {
    /** @type {Violation[]} */
    const violations = [];
    const s = getStrings();

    if (rules.includes('DI10'))
        for (const el of root.querySelectorAll('[data-kp-destructive]')) {
            if (el.matches(exempt)) continue;
            if (!el.hasAttribute('data-kp-confirm') && !el.hasAttribute('data-kp-undo')) {
                violations.push({ rule: 'DI10', element: el, message: s.contractDestructive });
            }
        }

    if (rules.includes('DI4'))
        for (const el of root.querySelectorAll('[data-kp-semantic]')) {
            if (el.matches(exempt)) continue;
            // Text, or an image with an accessible name. An icon that is
            // aria-hidden carries nothing, which is the usual mistake.
            const text = (el.textContent ?? '').trim();
            const named = el.querySelector('[aria-label], [aria-labelledby], title');
            if (text === '' && named === null) {
                violations.push({ rule: 'DI4', element: el, message: s.contractSemantic });
            }
        }

    return violations;
}

/** What enforcement changed on an element, so detach can put it back. */
const changed = new WeakMap();

/**
 * Report the violations and disarm what they point at.
 *
 * Idempotent: calling it again first restores everything it changed
 * before and then looks afresh, so markup completed after the first
 * pass comes back to life. Returns a detach that restores without
 * re-evaluating. The list is also available on the return value, so a
 * test asserts on it rather than on console output.
 *
 * @param {ParentNode} root
 * @param {{ disable?: boolean, rules?: Rule[], exempt?: string, log?: ((message: string, element: Element) => void) | null }} [options]
 * @returns {(() => void) & { violations: Violation[] }}
 */
export function enforceContracts(
    root = document,
    { disable = true, rules, exempt, log = (message, element) => console.error(message, element) } = {},
) {
    // Restore first: a second pass over a repaired page must not carry
    // the marks of the first.
    for (const el of root.querySelectorAll('[data-kp-contract-error]')) restore(el);

    const violations = findViolations(root, { rules, exempt });
    for (const v of violations) {
        /** @type {{ disabled?: boolean }} */
        const before = {};
        if (disable && v.rule === 'DI10' && 'disabled' in v.element) {
            const button = /** @type {HTMLButtonElement} */ (v.element);
            before.disabled = button.disabled;
            button.disabled = true;
        }
        changed.set(v.element, before);
        v.element.setAttribute('data-kp-contract-error', v.rule);
        log?.(`[kp-themes ${v.rule}] ${v.message}`, v.element);
        v.element.dispatchEvent(new CustomEvent(VIOLATION_EVENT, { bubbles: true, detail: v }));
    }

    const detach = () => {
        for (const v of violations) restore(v.element);
    };
    return Object.assign(detach, { violations });
}

/** @param {Element} el */
function restore(el) {
    const before = changed.get(el);
    if (before !== undefined && 'disabled' in el) /** @type {HTMLButtonElement} */ (el).disabled = before.disabled ?? false;
    changed.delete(el);
    el.removeAttribute('data-kp-contract-error');
}

/**
 * Arm-then-act on every destructive button that asked for a confirmation.
 *
 * @param {ParentNode} root
 * @param {{ windowMs?: number, disarmOnBlur?: boolean }} [options]
 * @returns {() => void} detach
 */
export function attachConfirmations(root = document, { windowMs = CONFIRM_WINDOW_MS, disarmOnBlur = true } = {}) {
    /** @type {(() => void)[]} */
    const cleanups = [];

    for (const el of root.querySelectorAll('[data-kp-confirm]')) {
        const button = /** @type {HTMLButtonElement} */ (el);
        if (button.dataset.kpConfirmAttached === '1') continue;
        button.dataset.kpConfirmAttached = '1';

        const original = button.textContent ?? '';
        const phrase = button.dataset.kpConfirm || getStrings().confirm;
        // Per element beats per call: one page mixes a two-second window
        // on a list and a ten-second one on the account deletion.
        const window_ = Number(button.dataset.kpConfirmMs) || windowMs;
        let armed = false;
        let timer = 0;

        /** @param {boolean} announce */
        const disarm = (announce = true) => {
            const was = armed;
            armed = false;
            button.textContent = original;
            button.removeAttribute('data-kp-armed');
            clearTimeout(timer);
            if (was && announce) button.dispatchEvent(new CustomEvent(DISARM_EVENT, { bubbles: true }));
        };
        const onBlur = () => {
            if (disarmOnBlur) disarm();
        };

        /** @param {Event} event */
        const onClick = (event) => {
            if (armed) {
                disarm(false);
                return; // the real handler runs: this click is the deliberate one
            }
            // The first click is the obstacle, so it must not reach
            // anything else — capture and stop, rather than trust that no
            // other listener acts.
            event.preventDefault();
            event.stopImmediatePropagation();
            armed = true;
            button.textContent = phrase;
            button.setAttribute('data-kp-armed', 'true');
            button.dispatchEvent(new CustomEvent(ARM_EVENT, { bubbles: true, detail: { windowMs: window_ } }));
            timer = window.setTimeout(disarm, window_);
        };

        button.addEventListener('click', onClick, { capture: true });
        button.addEventListener('blur', onBlur);
        cleanups.push(() => {
            button.removeEventListener('click', onClick, { capture: true });
            button.removeEventListener('blur', onBlur);
            delete button.dataset.kpConfirmAttached;
            disarm();
        });
    }

    return () => {
        for (const c of cleanups) c();
    };
}

/**
 * Move focus to the target of a skip link, adding `tabindex="-1"` if the
 * target cannot take focus on its own [KT6].
 *
 * JobTracker found the half a skip link needs and nothing here provided:
 * without a focusable target the browser scrolls and the next Tab goes
 * back into the menu, so the link has done nothing for the person it
 * exists for. Returns whether a target was found and focused.
 *
 * @param {string} href `#main`, or any same-page hash
 * @param {Document | Element} [root]
 * @returns {boolean}
 */
export function skipTo(href, root = document) {
    const id = href.startsWith('#') ? href.slice(1) : href;
    if (id === '') return false;
    const target = /** @type {HTMLElement | null} */ (root.querySelector(`#${CSS.escape(id)}`));
    if (target === null) return false;
    if (!target.hasAttribute('tabindex')) target.setAttribute('tabindex', '-1');
    target.focus();
    return true;
}

/**
 * Make every `.kp-skip-link` (or `[data-kp-skip]`) move focus, not only
 * the scroll position.
 *
 * @param {ParentNode} root
 * @returns {() => void} detach
 */
export function attachSkipLinks(root = document) {
    /** @type {(() => void)[]} */
    const cleanups = [];
    for (const el of root.querySelectorAll('.kp-skip-link, [data-kp-skip]')) {
        const link = /** @type {HTMLAnchorElement} */ (el);
        if (link.dataset.kpSkipAttached !== undefined) continue;
        link.dataset.kpSkipAttached = '';
        /** @param {Event} event */
        const onClick = (event) => {
            if (skipTo(link.getAttribute('href') ?? '')) event.preventDefault();
        };
        link.addEventListener('click', onClick);
        cleanups.push(() => {
            link.removeEventListener('click', onClick);
            delete link.dataset.kpSkipAttached;
        });
    }
    return () => {
        for (const c of cleanups) c();
    };
}
