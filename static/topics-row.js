// kyu: topics-row.js, one topic's table row: its markup and painting its cells from a topic.

import { stackbarExports, barcellExports, freshnessExports, tickExports, updateExports } from '/static/kp/dist/kp-themes.min.js';
import { h, number, plural } from './topics-util.js';
import { M, mark, STATUS_BADGE, QUIET_AFTER } from './topics-state.js';
const { setStackbar } = stackbarExports;
const { setBarCell } = barcellExports;
const { setAgo } = freshnessExports;
const { tickTo } = tickExports;
const { update } = updateExports;

/** @typedef {import('./topics-state.js').Topic} Topic */

/** @param {Topic} t */
function flowSpec(t) {
    const parts = [
        { key: 'backlog', label: 'Waiting', value: t.backlog },
        { key: 'inflight', label: 'In flight', value: t.in_flight },
        { key: 'dead', label: 'Dead', value: t.dead, tone: /** @type {const} */ ('destructive') },
        { key: 'lapsed', label: 'Lapsed', value: t.lapsed, tone: /** @type {const} */ ('warning') },
    ].filter((p) => p.value > 0);
    const sum = parts.reduce((n, p) => n + p.value, 0);
    return {
        parts,
        total: Math.max(sum, 1),
        legend: false,
        format: number,
        restLabel: sum === 0 ? 'Nothing waiting' : 'Rest',
        label: `${t.name}: ${sum === 0 ? 'nothing waiting' : parts.map((p) => `${number(p.value)} ${p.label.toLowerCase()}`).join(', ')}`,
    };
}

/** @param {Topic} t @returns {HTMLTableRowElement} */
export function buildRow(t) {
    const cell = (/** @type {string} */ label, /** @type {string} */ cls, /** @type {Node[]} */ kids) =>
        h('td', { 'data-label': label, class: cls || undefined }, ...kids);
    const markEl = h('span', { class: 'kp-idmark kp-idmark--sm', 'data-kp-idmark': t.name, 'aria-hidden': 'true' });
    mark(markEl, t.name);
    const href = `/t/${encodeURIComponent(t.name)}/dashboard`;
    const row = /** @type {HTMLTableRowElement} */ (
        h(
            'tr',
            { 'data-kp-row-key': t.name, 'data-kp-anchor': `topic-${t.name}`, 'data-tp-topic': t.name },
            cell('Topic', 'tp-name', [
                h('span', { class: 'tp-topic' }, markEl, h('a', { class: 'tp-topic__link', href }, t.name)),
            ]),
            cell('Status', '', [h('span', { class: 'kp-badge', 'data-tp-cell': 'status' })]),
            cell('Messages', 'kp-text-end kp-numeric', [h('span', { 'data-kp-tick': true, 'data-tp-cell': 'messages' })]),
            cell('Backlog', '', [h('span', { class: 'kp-bar-cell', 'data-tp-cell': 'backlog' })]),
            cell('Dead', '', [h('span', { class: 'kp-bar-cell tp-hover', tabindex: '0', 'data-tp-cell': 'dead' })]),
            cell('Flow', 'tp-flow', [h('figure', { class: 'kp-stackbar', 'data-kp-stackbar': true, 'data-tp-cell': 'flow' })]),
            cell('Last published', '', [
                h('time', { class: 'kp-ago tp-hover', 'data-kp-ago': true, 'data-kp-ago-verb': 'published', tabindex: '0', 'data-tp-cell': 'last' }),
            ]),
            cell('Actions', 'tp-actions', [
                h(
                    'div',
                    { class: 'kp-row-actions' },
                    h('button', { type: 'button', class: 'kp-button kp-button--sm', 'data-kp-action': 'test', 'data-tp-test': t.name }, 'Send test'),
                    h('a', { class: 'kp-button kp-button--sm kp-button--primary', href, 'data-kp-action': 'open' }, 'Open'),
                ),
            ]),
        )
    );
    paintRow(row, t);
    return row;
}

/** Write one topic into its row. The cells that carry a number tick when they change, the words change the theme's way. @param {HTMLElement} row @param {Topic} t */

/** @param {HTMLElement} row @param {string} key */
const cellOf = (row, key) => /** @type {HTMLElement} */ (row.querySelector(`[data-tp-cell="${key}"]`));
/** @param {HTMLElement} row @param {string} key */
const tdOf = (row, key) => /** @type {HTMLElement} */ (cellOf(row, key).closest('td'));

/** @param {HTMLElement} row @param {Topic} t @param {boolean} animate */
function paintStatus(row, t, animate) {
    const badge = cellOf(row, 'status');
    if (badge.textContent !== t.status) {
        badge.className = `kp-badge ${STATUS_BADGE[t.status] ?? ''}`;
        if (animate && badge.textContent) update(badge, t.status);
        else badge.textContent = t.status;
    }
    tdOf(row, 'status').setAttribute('data-kp-filter-value', t.status);
    tdOf(row, 'status').setAttribute('data-kp-sort-value', t.status);
}

/** @param {HTMLElement} row @param {Topic} t @param {boolean} animate */
function paintMessages(row, t, animate) {
    const messages = cellOf(row, 'messages');
    if (messages.textContent !== number(t.messages)) {
        if (animate && messages.textContent) tickTo(messages, number(t.messages));
        else messages.textContent = number(t.messages);
    }
    tdOf(row, 'messages').setAttribute('data-kp-sort-value', String(t.messages));
    const link = /** @type {HTMLElement} */ (row.querySelector('.tp-topic__link'));
    link.setAttribute(
        'data-kp-hovercard',
        t.subscriptions.length
            ? `${t.name} is read by ${plural(t.subscriptions.length, 'subscription', 'subscriptions')}: ${t.subscriptions.join(', ')}`
            : `${t.name} has no subscriptions yet`,
    );
}

/** The backlog and dead bars, each against the largest of its column. @param {HTMLElement} row @param {Topic} t */
function paintBars(row, t) {
    const max = {
        backlog: Math.max(0, ...M.model.topics.map((x) => x.backlog)),
        dead: Math.max(0, ...M.model.topics.map((x) => x.dead)),
    };
    setBarCell(cellOf(row, 'backlog'), {
        value: max.backlog ? t.backlog / max.backlog : 0,
        text: number(t.backlog),
        sortValue: t.backlog,
        label: 'of the largest backlog',
    });
    setBarCell(cellOf(row, 'dead'), {
        value: max.dead ? t.dead / max.dead : 0,
        text: number(t.dead),
        sortValue: t.dead,
        label: 'of the most dead letters',
    });
    cellOf(row, 'dead').setAttribute(
        'data-kp-hovercard',
        t.dead > 0 ? `Latest dead letter on ${t.name}, ${t.dead_note ?? 'no detail kept'}` : `No dead letters on ${t.name}`,
    );
}

/** @param {HTMLElement} row @param {Topic} t */
function paintFlow(row, t) {
    const flow = flowSpec(t);
    setStackbar(cellOf(row, 'flow'), flow);
    tdOf(row, 'flow').setAttribute('data-kp-sort-value', String(flow.parts.reduce((n, p) => n + p.value, 0)));
}

/** @param {HTMLElement} row @param {Topic} t */
function paintLast(row, t) {
    const last = cellOf(row, 'last');
    last.setAttribute('data-kp-stale-after', String(QUIET_AFTER));
    if (t.last_published_ms === null) {
        last.removeAttribute('data-kp-ago');
        last.textContent = 'never';
        last.removeAttribute('datetime');
    } else {
        last.setAttribute('data-kp-ago', '');
        setAgo(last, t.last_published_ms);
    }
    last.setAttribute('data-kp-hovercard', `Last message on ${t.name}. Hover or focus shows the exact time; quiet after 24 h.`);
    tdOf(row, 'last').setAttribute('data-kp-sort-value', String(t.last_published_ms ?? 0));
}

/** Write one topic into its row. The cells that carry a number tick when they change, the words change the theme's way. @param {HTMLElement} row @param {Topic} t */
export function paintRow(row, t, animate = false) {
    paintStatus(row, t, animate);
    paintMessages(row, t, animate);
    paintBars(row, t);
    paintFlow(row, t);
    paintLast(row, t);
}
