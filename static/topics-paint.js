// kyu: topics-paint.js, drawing the Topics page from the model: hub bar, update time, and everything at once.

import { setStackbar } from '/static/kp/js/stackbar.js';
import { setAgo } from '/static/kp/js/freshness.js';
import { $, h, number, plural } from './topics-util.js';
import { M, totals } from './topics-state.js';
import { drawKpis } from './topics-kpis.js';
import { drawBand } from './topics-attention.js';
import { paintRow } from './topics-row.js';
import { table, rowsOf } from './topics-table.js';

export function drawHubBar() {
    const bar = $('[data-tp-hubbar]');
    if (!bar) return;
    const t = totals();
    const sum = t.backlog + t.inflight + t.dead + t.lapsed;
    const state = sum === 0 ? 'empty' : 'ready';
    setStackbar(bar, {
        title: 'Where the undelivered messages are, across the hub',
        parts: [
            { key: 'backlog', label: 'Waiting', value: t.backlog },
            { key: 'inflight', label: 'In flight', value: t.inflight },
            { key: 'dead', label: 'Dead', value: t.dead, tone: 'destructive' },
            { key: 'lapsed', label: 'Lapsed', value: t.lapsed, tone: 'warning' },
        ].filter((p) => p.value > 0),
        total: Math.max(sum, 1),
        restLabel: 'Nothing waiting',
        format: number,
        state: /** @type {any} */ (state),
        message: t.topics === 0 ? 'No topics yet.' : 'Nothing is waiting anywhere on the hub.',
        retry: false,
    });
}


export function drawMeta() {
    const meta = $('[data-tp-meta]');
    if (!meta) return;
    const time = h('time', { class: 'kp-ago', 'data-kp-ago': true, 'data-kp-ago-verb': 'updated', 'data-kp-stale-after': '180' });
    meta.replaceChildren(h('span', {}, `${plural(M.model.topics.length, 'topic', 'topics')}\u00a0\u00b7\u00a0`), time);
    setAgo(time, M.model.now_ms);
}


/** Everything the data changes: the figures, the band, the bar and the rows. @param {boolean} [animate] */
export function paintAll(animate = false) {
    drawKpis();
    drawBand();
    drawHubBar();
    drawMeta();
    for (const row of rowsOf()) {
        const t = M.model.topics.find((x) => x.name === row.getAttribute('data-tp-topic'));
        if (t) paintRow(row, t, animate);
    }
    table()?.refresh();
}
