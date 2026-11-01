// kyu: subscription-policy.js, changing one value of the policy in place: Change opens an input in the row,
// Enter saves behind an Undo toast, Esc cancels. An empty input returns the value to kyu's default.

import { undoToastExports, overlaysExports } from '/static/kp/dist/kp-themes.min.js';
import { $, getJson } from './subscription-util.js';
import { DATA, SUB } from './subscription-state.js';
import { readPage } from './subscription-page.js';
const { undoToast } = undoToastExports;
const { toast } = overlaysExports;

/** How long Undo is offered before the change is written. */
const UNDO_MS = 6000;
const UNIT_MS = /** @type {Record<string, number>} */ ({ ms: 1, s: 1000, sec: 1000, min: 60_000, m: 60_000, h: 3_600_000, d: 86_400_000, day: 86_400_000, days: 86_400_000 });

/** What each field is called in a sentence and how its value is typed. */
const FIELDS = /** @type {Record<string, { noun: string, duration: boolean, hint: string }>} */ ({
    lease_ms: { noun: 'lease', duration: true, hint: 'e.g. 30 s, 5 min; empty for the default' },
    max_attempts: { noun: 'attempts', duration: false, hint: 'a whole number; empty for the default' },
    backoff_ms: { noun: 'wait between attempts', duration: true, hint: 'e.g. 1 s, 0 for none; empty for the default' },
    ttl_ms: { noun: 'time to live', duration: true, hint: 'e.g. 10 min, 1 h; empty for the default' },
});

/** A typed value as the number kyu stores, null for "back to the default", or an Error saying what is wrong.
 * @param {string} field @param {string} typed @returns {number | null | Error} */
export function parseValue(field, typed) {
    const text = typed.trim().toLowerCase();
    if (text === '') return null;
    if (!FIELDS[field].duration) {
        return /^\d+$/.test(text) && Number(text) >= 1 ? Number(text) : new Error('Type a whole number of 1 or more.');
    }
    const match = /^(\d+(?:\.\d+)?)\s*([a-z]*)$/.exec(text);
    const unit = match ? UNIT_MS[match[2] || 's'] : undefined;
    if (!match || unit === undefined) return new Error('Type a duration such as 30 s, 5 min or 1 h.');
    return Math.round(Number(match[1]) * unit);
}

/** Swap the row's value for an input of the same height; resolves with the typed text, or null on Esc.
 * @param {HTMLElement} row @returns {Promise<string | null>} */
function askInRow(row) {
    const field = /** @type {string} */ (row.dataset.kyPolicy);
    const value = /** @type {HTMLElement} */ ($('.ky-policy__value', row));
    const input = document.createElement('input');
    input.className = 'kp-field__input ky-policy__input';
    input.setAttribute('aria-label', `New ${FIELDS[field].noun}`);
    input.placeholder = FIELDS[field].hint;
    input.value = /** @type {HTMLElement} */ ($('[data-ky-policy-text]', row)).textContent ?? '';
    value.hidden = true;
    value.after(input);
    input.focus();
    input.select();
    return new Promise((resolve) => {
        const finish = (/** @type {string | null} */ answer) => {
            input.remove();
            value.hidden = false;
            resolve(answer);
        };
        input.addEventListener('keydown', (event) => {
            if (event.key === 'Enter') finish(input.value);
            if (event.key === 'Escape') finish(null);
        });
        input.addEventListener('blur', () => finish(null), { once: true });
    });
}

/** Show the new value at once, write it when the Undo window closes, and read the policy back from kyu.
 * @param {HTMLElement} row @param {string} field @param {number | null} next @param {string} typed */
function stage(row, field, next, typed) {
    const text = /** @type {HTMLElement} */ ($('[data-ky-policy-text]', row));
    const marker = /** @type {HTMLElement} */ ($('[data-ky-policy-default]', row));
    const before = { text: text.textContent, isDefault: !marker.hidden };
    text.textContent = next === null ? "kyu's default" : typed;
    marker.hidden = true;
    undoToast(`The ${FIELDS[field].noun} of ${DATA.subscription} changes`, {
        ms: UNDO_MS,
        commit: async () => {
            await getJson(`${SUB}/policy`, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ field, value: next }),
            });
            await readPage();
        },
        undo: () => {
            text.textContent = before.text;
            marker.hidden = !before.isDefault;
        },
    });
}

/** Change on a policy row: ask, check, then stage the change behind Undo. @param {HTMLElement} row */
async function change(row) {
    const field = /** @type {string} */ (row.dataset.kyPolicy);
    const typed = await askInRow(row);
    if (typed === null) return;
    const next = parseValue(field, typed);
    if (next instanceof Error) {
        toast(`The ${FIELDS[field].noun} did not change. ${next.message}`, { live: 'assertive' });
        return;
    }
    stage(row, field, next, typed.trim());
}

/** Wire Change on every policy row; delegated, since a re-read of the page replaces the rows. */
export function initPolicy() {
    document.addEventListener('click', (event) => {
        const button = /** @type {HTMLElement | null} */ (/** @type {Element} */ (event.target).closest?.('[data-ky-policy-edit]'));
        const row = button?.closest('[data-ky-policy]');
        if (row) void change(/** @type {HTMLElement} */ (row));
    });
}
