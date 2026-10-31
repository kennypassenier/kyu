// kyu: topic-dead-actions.js, the actions on dead-letter rows: requeue, delete with Undo, and the selection bar.

import { undoToastExports, flashExports, selectionBarExports } from '/static/kp/dist/kp-themes.min.js';
import { $$, num, plural } from './topic-util.js';
import { S, T, call, refusal, sentence, holdPending } from './topic-state.js';
import { deadBody } from './topic-dead.js';
import { bulk, deadKey } from './topic-dead-core.js';
import { reload } from './topic-refresh.js';
const { undoToast } = undoToastExports;
const { runFlashButton } = flashExports;
const { setSelectionBusy, setSelectionResult } = selectionBarExports;

const selectionBar = /** @type {HTMLElement} */ (document.querySelector('[data-kt-dead] [data-kt-selection]'));

/** How long a requeued row stays on screen so the button's "Requeued" flash can be read. */
const FLASH_LINGER_MS = 500;
/** The Undo window of a deletion, unless the selection bar names another. */
const UNDO_MS = 6000;


// Rows: requeue, delete with Undo.
deadBody.addEventListener('click', (event) => {
    const button = event.target instanceof Element ? event.target.closest('[data-kt-row]') : null;
    const row = button?.closest('tr');
    if (!(button instanceof HTMLElement) || !row) return;
    if (button.dataset.ktRow === 'requeue') return requeueRow(button, row);
    deleteRowWithUndo(row);
});

/** Requeue one dead letter; on success the row leaves after the button's flash. @param {HTMLElement} button @param {HTMLElement} row */
async function requeueRow(button, row) {
    const sub = row.dataset.ktSub ?? '';
    const id = row.dataset.ktId ?? '';
    let ok = false;
    await runFlashButton(button, async () => {
        const r = await call(`${T}/requeue`, { form: { subscription: sub, id } });
        if (!r.ok) return refusal(/** @type {any} */ (r));
        ok = true;
        return 'Requeued';
    });
    if (!ok) return;
    await new Promise((resolve) => setTimeout(resolve, FLASH_LINGER_MS));
    row.remove();
    S.deadTable?.refresh();
    void reload();
}

/** Delete one dead letter: it leaves the table at once, the hub hears it when the Undo window closes. @param {HTMLElement} row */
function deleteRowWithUndo(row) {
    const sub = row.dataset.ktSub ?? '';
    const id = row.dataset.ktId ?? '';
    const next = row.nextElementSibling;
    row.remove();
    S.deadTable?.refresh();
    const settle = holdPending();
    const putBack = () => {
        deadBody.insertBefore(row, next && next.isConnected ? next : null);
        S.deadTable?.refresh();
    };
    undoToast(`Dead letter ${id} deleted`, {
        ms: UNDO_MS,
        max: 3,
        commit: async () => {
            const r = await call(`${T}/delivery/delete`, { form: { subscription: sub, id } });
            settle();
            if (!r.ok) throw new Error(sentence(/** @type {any} */ (r)));
            void reload();
        },
        undo: () => {
            settle();
            putBack();
        },
        restore: () => {
            settle();
            putBack();
        },
    });
}

// The selection bar: requeue or delete the ticked rows.
selectionBar.addEventListener('kp-selection-action', (event) => {
    const detail = /** @type {CustomEvent} */ (event).detail;
    /** @type {string[]} */
    const keys = detail.keys;
    const rows = $$('tr', deadBody).filter((r) => keys.includes(r.getAttribute('data-kp-row-key') ?? ''));
    const items = rows.map((r) => ({ subscription: r.dataset.ktSub ?? '', id: r.dataset.ktId ?? '' }));
    if (detail.action === 'delete') deleteSelected(rows, items, keys.length, detail.undoMs ?? UNDO_MS);
    else requeueSelected(rows, items, keys.length);
});

/** Untick the rows' boxes so the selection bar forgets them. @param {HTMLElement[]} rows */
function untick(rows) {
    for (const r of rows) {
        const box = /** @type {HTMLInputElement | null} */ (r.querySelector('[data-kp-select-row]'));
        if (box) {
            box.checked = false;
            box.dispatchEvent(new Event('change', { bubbles: true }));
        }
    }
}

/** Delete the ticked rows with one Undo; refused ones come back, the rest stay deleted.
 * @param {HTMLElement[]} rows @param {{ subscription: string, id: string }[]} items @param {number} count @param {number} undoMs */
function deleteSelected(rows, items, count, undoMs) {
    const places = rows.map((r) => r.nextElementSibling);
    untick(rows);
    for (const r of rows) r.remove();
    S.deadTable?.refresh();
    const settle = holdPending();
    const putBack = (/** @type {HTMLElement[]} */ list) => {
        for (const r of list) {
            const place = places[rows.indexOf(r)];
            deadBody.insertBefore(r, place && place.isConnected ? place : null);
        }
        S.deadTable?.refresh();
    };
    /** @type {HTMLElement[]} */
    let restoreOnly = rows;
    undoToast(`${plural(count, 'dead letter', 'dead letters')} deleted`, {
        ms: undoMs,
        max: 3,
        commit: async () => {
            const { refused } = await bulk('delete', items);
            settle();
            if (refused.length) {
                const gone = new Set(refused.map((x) => deadKey(x)));
                restoreOnly = rows.filter((r) => gone.has(r.getAttribute('data-kp-row-key') ?? ''));
                throw new Error(`${plural(refused.length, 'dead letter', 'dead letters')} could not be deleted: ${refused[0].reason}`);
            }
            void reload();
        },
        undo: () => {
            settle();
            putBack(rows);
        },
        restore: () => putBack(restoreOnly),
    });
}

/** Requeue the ticked rows through the selection bar's busy and result states.
 * @param {HTMLElement[]} rows @param {{ subscription: string, id: string }[]} items @param {number} count */
function requeueSelected(rows, items, count) {
    setSelectionBusy(selectionBar, `Requeueing ${num(count)}…`);
    void bulk('requeue', items).then(({ done, refused }) => {
        setSelectionBusy(selectionBar, '');
        setSelectionResult(selectionBar, {
            done: done.length,
            refused: refused.map((x) => ({ key: deadKey(x), reason: x.reason })),
        });
        const gone = new Set(done.map((x) => deadKey(x)));
        for (const r of rows) if (gone.has(r.getAttribute('data-kp-row-key') ?? '')) r.remove();
        S.deadTable?.refresh();
        void reload();
    });
}
