// kyu: subscription-backlog.js, the backlog table of the Subscription page: bulk delete with Undo, the live row swap, and the Attempts buttons.

import { freshnessExports, datatableExports, undoToastExports, selectionBarExports } from '/static/kp/dist/kp-themes.min.js';
import { $, $$, getJson } from './subscription-util.js';
import { DATA, BASE, items } from './subscription-state.js';
import { drawGauges, drawCounts } from './subscription-gauges.js';
import { loadAttempts } from './subscription-attempts.js';
const { agoMoment } = freshnessExports;
const { dataTable } = datatableExports;
const { undoToast } = undoToastExports;
const { setSelectionResult } = selectionBarExports;

/** Set by initBacklog: take the rows of a freshly read page. @type {(doc: Document) => void} */
export let swapRows = () => undefined;

/** How long a deletion can be undone when the form button (not the selection bar) deletes, in ms. */
const FORM_UNDO_MS = 6000;

/**
 * Rows leave the table while an Undo window is open and come back on Undo. The table repaints `hidden` on every
 * refresh(), so a row that is gone for now is taken out of the DOM and put back on Undo.
 * @param {HTMLElement} body @param {() => ReturnType<typeof dataTable>} handle
 */
function parking(body, handle) {
    /** Rows taken out of the table while an Undo window is open, with the row each stood before. @type {Map<HTMLElement, Element | null>} */
    const parked = new Map();
    const park = (/** @type {HTMLElement[]} */ rows) => {
        for (const row of rows) {
            parked.set(row, row.nextElementSibling);
            row.remove();
        }
        handle()?.refresh();
    };
    const restore = (/** @type {HTMLElement[]} */ rows) => {
        for (const row of rows) {
            const before = parked.get(row) ?? null;
            body.insertBefore(row, before && before.parentNode === body ? before : null);
            parked.delete(row);
        }
        handle()?.refresh();
    };
    return { parked, park, restore };
}

/** @typedef {{ wrap: HTMLElement, bar: HTMLElement, byKey: Map<string, HTMLElement>, parked: Map<HTMLElement, Element | null>, park: (rows: HTMLElement[]) => void, restore: (rows: HTMLElement[]) => void }} Backlog */

/** Ask kyu to delete the messages once the Undo window closes; refused ones come back.
 * @param {Backlog} b @param {string[]} keys */
async function commitDelete(b, keys) {
    const answer = await getJson(`${BASE}/bulk/delete`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ subscription: DATA.subscription, ids: keys }),
    });
    const refused = /** @type {{ id: string, reason: string }[]} */ (answer.refused ?? []);
    const done = /** @type {string[]} */ (answer.done ?? []);
    if (done.length === 0 && refused.length > 0) throw new Error(refused[0].reason);
    b.restore(refused.flatMap((r) => (b.byKey.get(r.id) ? [/** @type {HTMLElement} */ (b.byKey.get(r.id))] : [])));
    for (const id of done) {
        const row = b.byKey.get(id);
        if (row) b.parked.delete(row);
        items.delete(id);
    }
    if (refused.length > 0) setSelectionResult(b.bar, { done: done.length, refused: refused.map((r) => ({ key: r.id, reason: r.reason })) });
    drawCounts();
    drawGauges();
}

/**
 * Delete the way the bulk route works: the rows go at once, Undo is offered for a few seconds, and kyu is asked when the window closes.
 * @param {Backlog} b @param {string[]} keys @param {number} ms
 */
function removeWithUndo(b, keys, ms) {
    const rows = keys.flatMap((k) => $$(`[data-kp-row-key="${CSS.escape(k)}"]`, b.wrap));
    for (const row of rows) {
        const box = /** @type {HTMLInputElement | null} */ ($('[data-kp-select-row]', row));
        if (box?.checked) {
            box.checked = false;
            box.dispatchEvent(new Event('change', { bubbles: true }));
        }
    }
    b.park(rows);
    const count = rows.length;
    undoToast(`${count} ${count === 1 ? 'message' : 'messages'} deleted from ${DATA.subscription}`, {
        ms,
        max: 3,
        commit: () => commitDelete(b, keys),
        undo: () => b.restore(rows),
    });
}

/** Wire the selection bar, the per-row delete form, the live row swap and the Attempts buttons of the backlog table. */
export function initBacklog() {
    const wrap = $('#ky-backlog-table');
    const bar = $('#ky-bar');
    if (!wrap || !bar) return;
    const handle = () => dataTable(wrap);
    /** @type {Map<string, HTMLElement>} */
    const byKey = new Map($$('[data-kp-row-key]', wrap).map((row) => [row.dataset.kpRowKey ?? '', row]));
    const body = /** @type {HTMLElement} */ ($('tbody', wrap));
    const { parked, park, restore } = parking(body, handle);
    /** @type {Backlog} */
    const b = { wrap, bar, byKey, parked, park, restore };
    document.addEventListener('kp-selection-action', (event) => {
        const e = /** @type {CustomEvent<{ action: string, keys: string[], matching?: boolean, undoMs: number }>} */ (event);
        if (e.target !== bar || e.detail.action !== 'delete') return;
        const keys = e.detail.matching ? $$('[data-kp-row-key]', wrap).map((row) => row.dataset.kpRowKey ?? '') : e.detail.keys;
        removeWithUndo(b, keys, e.detail.undoMs);
    });
    // The form post is the no-script path; with a script the same button deletes with Undo.
    wrap.addEventListener('submit', (event) => {
        const form = event.target instanceof Element ? event.target.closest('.ky-row-form') : null;
        if (!form) return;
        event.preventDefault();
        const id = /** @type {HTMLInputElement | null} */ (form.querySelector('input[name="id"]'))?.value;
        if (id) removeWithUndo(b, [id], FORM_UNDO_MS);
    });
    swapRows = (doc) => swapIn(b, body, handle, doc);
    wrap.addEventListener('click', (event) => {
        const button = event.target instanceof Element ? event.target.closest('[data-ky-attempts]') : null;
        const id = button?.getAttribute('data-ky-attempts');
        if (!id) return;
        void loadAttempts(id);
        $('#ky-attempts')?.scrollIntoView({ block: 'start', behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth' });
    });
}

/**
 * Put the rows of a freshly read page in the table (a read while an Undo window is open leaves them alone).
 * @param {Backlog} b @param {HTMLElement} body @param {() => ReturnType<typeof dataTable>} handle @param {Document} doc
 */
function swapIn(b, body, handle, doc) {
    // Rows waiting out an Undo window stay out until it closes.
    const waiting = new Set([...b.parked.keys()].map((row) => row.dataset.kpRowKey));
    const fresh = $$('#ky-backlog-table tbody tr[data-kp-row-key]', doc)
        .filter((row) => !waiting.has(row.dataset.kpRowKey))
        .map((row) => /** @type {HTMLElement} */ (document.importNode(row, true)));
    body.replaceChildren(...fresh);
    b.byKey.clear();
    for (const row of fresh) b.byKey.set(row.dataset.kpRowKey ?? '', row);
    for (const el of $$('[data-ky-moment]', body)) el.textContent = agoMoment(Number(el.dataset.kyMoment));
    void import('/static/kp/dist/kp-themes.min.js').then((kp) => kp.autoExports.attachAll(body));
    handle()?.refresh();
}
