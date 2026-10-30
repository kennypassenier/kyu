// kyu: topics-kpis.js, the Topics page's key-figure tiles: their markup, drawing, and the text they last showed.

import { $, $$, h, number, plural } from './topics-util.js';
import { ui, totals } from './topics-state.js';

export const kpis = $('[data-tp-kpis]');

/** @param {string} key @param {number} n */
function kpiValue(key, n) {
    const span = h('span', { 'data-kp-tick': true, 'data-tp-key': key });
    span.textContent = number(n);
    return h('span', { class: 'kp-kpi__value' }, span);
}

/** @param {string} key @param {string} text */
const kpiTrend = (key, text) => h('span', { class: 'kp-kpi__trend', 'data-tp-key': `${key}-words` }, text);

/** The "Filter / Filtering x" hint every toggle tile carries. */
const filterHint = () =>
    h(
        'span',
        { class: 'kp-kpi__hint', 'aria-hidden': 'true' },
        h('span', { 'data-kp-when': 'off' }, 'Filter'),
        h('span', { 'data-kp-when': 'on' }, 'Filtering ×'),
    );

/** A tile that is not a filter. @param {string} tile @param {string} label @param {string} key @param {number} n @param {string} words */
const plainTile = (tile, label, key, n, words) =>
    h('div', { class: 'kp-kpi', 'data-tp-tile': tile }, h('span', { class: 'kp-kpi__label' }, label), kpiValue(key, n), kpiTrend(key, words));

/** A tile that is a filter on its column. @param {string} key @param {string} label @param {string} tone @param {number} n @param {string} words @param {Node} hint */
const toggleTile = (key, label, tone, n, words, hint) =>
    h(
        'button',
        {
            type: 'button',
            class: 'kp-kpi kp-kpi--toggle',
            'aria-pressed': String(ui.pressed[/** @type {'backlog' | 'dead'} */ (key)]),
            'data-kp-tone': tone || undefined,
            'data-tp-filter': key,
        },
        h('span', { class: 'kp-kpi__label' }, label),
        hint.cloneNode(true),
        kpiValue(key, n),
        kpiTrend(key, words),
    );

/** The key-figure tiles as markup. */
export function kpiTiles() {
    const t = totals();
    const calm = t.backlog + t.dead === 0;
    const hint = filterHint();
    return [
        plainTile('topics', 'Topics', 'topics', t.topics, `${plural(t.subscriptions, 'subscription', 'subscriptions')} in all`),
        plainTile('messages', 'Messages', 'messages', t.messages, 'kept across all topics'),
        toggleTile('backlog', 'Backlog', calm ? '' : 'warning', t.backlog, calm ? 'nothing is waiting' : `${number(t.inflight)} more in flight`, hint),
        toggleTile('dead', 'Dead', calm ? '' : 'destructive', t.dead, t.dead === 0 ? 'nothing needs a decision' : `${number(t.lapsed)} lapsed, not dead yet`, hint),
    ];
}

/** The text each counting element shows, by key, so a swapped-in tile can count from what was shown. */
export const shown = new Map();
export function remember() {
    shown.clear();
    for (const el of $$('[data-tp-key]', kpis ?? document)) shown.set(el.getAttribute('data-tp-key'), el.textContent ?? '');
}

export function drawKpis() {
    kpis?.replaceChildren(...kpiTiles());
    remember();
}
