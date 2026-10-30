// kyu: topics-filters.js, the key figures filter the table, the table's filters light the tiles, and fix links land on their row.

import { VIEW_EVENT } from '/static/kp/js/datatable.js';
import { $ } from './topics-util.js';
import { ui, totals } from './topics-state.js';
import { table } from './topics-table.js';
import { sendTest } from './topics-test.js';

/* ---- the key figures filter the table (Backlog and Dead are range filters on their columns) */

const COLUMN = { backlog: 3, dead: 4 };
let applying = false;
function applyFilters() {
    const dt = table();
    if (!dt) return;
    applying = true;
    for (const key of /** @type {const} */ (['backlog', 'dead'])) dt.filter(COLUMN[key], ui.pressed[key] ? { from: '1' } : null);
    applying = false;
    syncTiles(dt.view());
}

document.addEventListener('kp-kpi-toggle', (event) => {
    const tile = /** @type {HTMLElement} */ (event.target).closest('[data-tp-filter]');
    if (!tile) return;
    ui.pressed[/** @type {'backlog' | 'dead'} */ (tile.getAttribute('data-tp-filter'))] = /** @type {CustomEvent} */ (event).detail.pressed;
    applyFilters();
});

/** The tiles follow the table's filters. @param {import('/static/kp/js/datatable.js').View} view */
function syncTiles(view) {
    const asks = (/** @type {number} */ column) => {
        const f = view.filters?.[column];
        return Boolean(f && !Array.isArray(f) && f.from === '1' && !f.to);
    };
    for (const key of /** @type {const} */ (['backlog', 'dead'])) {
        ui.pressed[key] = asks(COLUMN[key]);
        $(`[data-tp-filter="${key}"]`)?.setAttribute('aria-pressed', String(ui.pressed[key]));
    }
}

// A filter removed from its pill lets go of its tile, and a filter set in the panel lights it.
document.addEventListener(VIEW_EVENT, (event) => {
    if (!(event.target instanceof Element) || !event.target.matches('[data-tp-datatable]')) return;
    const view = /** @type {CustomEvent} */ (event).detail;
    if (!applying) syncTiles(view);
    const empty = $('[data-tp-nomatch]');
    if (empty && view.total > 0 && view.shown === 0) {
        const deadOnly = ui.pressed.dead && !ui.pressed.backlog && Object.keys(view.filters ?? {}).length === 1 && !view.query;
        const allClear = deadOnly && totals().dead === 0;
        empty.setAttribute('data-kp-art', allClear ? 'done' : 'nomatch');
        $('[data-tp-empty-title]')?.replaceChildren(allClear ? 'Nothing is dead' : 'No topic matches');
        $('[data-tp-empty-body]')?.replaceChildren(
            allClear
                ? 'No message has run out of attempts on any topic. Nothing needs a decision.'
                : 'Loosen the search or remove a filter.',
        );
    }
});

document.addEventListener('click', (event) => {
    const target = event.target instanceof Element ? event.target : null;
    if (target?.closest('[data-tp-show]')) {
        // The row must be on the table before the link lands on it.
        const dt = table();
        dt?.clearFilters();
        dt?.query('');
        for (const key of /** @type {const} */ (['backlog', 'dead'])) ui.pressed[key] = false;
    }
    const test = target?.closest('[data-tp-test]');
    if (test) void sendTest(/** @type {HTMLButtonElement} */ (test));
});
