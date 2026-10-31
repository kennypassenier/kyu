// kyu: topics-table.js, the Topics table: finding it, filling its rows from the model, and syncing rows after a refresh.

import { datatableExports } from '/static/kp/dist/kp-themes.min.js';
import { $, $$ } from './topics-util.js';
import { M } from './topics-state.js';
import { buildRow, paintRow } from './topics-row.js';
const { dataTable } = datatableExports;

export const tableEl = () => $('[data-tp-datatable]');
export const table = () => {
    const el = tableEl();
    return el ? dataTable(el) : null;
};
export const rowsOf = () => $$('[data-tp-topic]');

export function fillRows() {
    const body = $('[data-tp-rows]');
    if (!body) return;
    body.replaceChildren(...M.model.topics.map(buildRow));
}

/** Rows for topics that appeared are added, rows for topics that went are removed, the rest are repainted. */
export function syncRows() {
    const body = $('[data-tp-rows]');
    if (!body) return;
    const names = new Set(M.model.topics.map((t) => t.name));
    for (const row of rowsOf()) if (!names.has(row.getAttribute('data-tp-topic') ?? '')) row.remove();
    for (const t of M.model.topics) {
        const row = rowsOf().find((r) => r.getAttribute('data-tp-topic') === t.name);
        if (row) paintRow(row, t, true);
        else body.append(buildRow(t));
    }
}
