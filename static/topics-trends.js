// kyu: topics-trends.js, the Topics page's key figures with their last 24 hours (sparkline tiles).

import { setTrendData, CHART_RETRY_EVENT } from '/static/kp/js/chart.js';
import { attachAll } from '/static/kp/js/auto.js';
import { $, h, number, readJson } from './topics-util.js';
import { totals } from './topics-state.js';

const SAMPLE_STEP = 300_000;
/** `key` is the field of a totals sample; `bad` is whether a rise is bad news. */
const TRENDS = [
    { key: 'messages', label: 'Messages', note: 'kept', bad: false, tone: '' },
    { key: 'backlog', label: 'Backlog', note: 'waiting', bad: true, tone: 'warning' },
    { key: 'in_flight', label: 'In flight', note: 'being worked', bad: false, tone: '' },
    { key: 'dead', label: 'Dead', note: 'dead letters', bad: true, tone: 'destructive' },
];
const trend = {
    /** @type {'loading' | 'ready' | 'empty' | 'error'} */
    state: 'loading',
    /** @type {{ at: number, messages: number, backlog: number, dead: number, in_flight: number }[]} */
    samples: [],
};

/** The strip of tiles, built once. @param {HTMLElement} strip */
function buildTiles(strip) {
    strip.replaceChildren(
        ...TRENDS.map((tr, i) =>
            h(
                'div',
                { class: 'kp-kpi kp-kpi--trend', 'data-tp-trend': tr.key, 'data-kp-tone': tr.tone || undefined },
                h('span', { class: 'kp-kpi__label' }, tr.label, ' ', h('span', { class: 'kp-kpi__label-note' }, tr.note)),
                h('span', { class: 'kp-kpi__value' }),
                h('span', { class: 'kp-kpi__trend' }),
                h('figure', {
                    class: 'kp-kpi__chart',
                    'data-kp-chart': 'spark',
                    'data-kp-spark-head': 'none',
                    'data-kp-spark-axis': 'relative',
                    'aria-label': `${tr.label}, last 24 hours`,
                    style: `--kp-chart-series: var(--chart-${(i % 5) + 1})`,
                }),
            ),
        ),
    );
    void attachAll(strip);
}

/** @param {HTMLElement} el @param {string} width */
const skeleton = (el, width) => el.replaceChildren(Object.assign(h('span', { class: 'kp-skeleton' }), { style: `inline-size: ${width}` }));

/** The change since the start of the window, as the tile's words. @param {typeof TRENDS[number]} tr @param {[number, number][]} series */
function deltaWords(tr, series) {
    const change = series[series.length - 1][1] - series[0][1];
    const peak = Math.max(...series.map((p) => p[1]));
    return [
        h(
            'span',
            {
                class: 'kp-kpi__delta',
                'data-kp-direction': change > 0 ? 'up' : change < 0 ? 'down' : undefined,
                'data-kp-tone': change === 0 ? undefined : tr.bad === change > 0 ? 'bad' : 'good',
            },
            number(Math.abs(change)),
        ),
        ` since yesterday · peak ${number(peak)}`,
    ];
}

/** Draw one tile in the current state. @param {HTMLElement} strip @param {typeof TRENDS[number]} tr */
function drawTile(strip, tr) {
    const tile = /** @type {HTMLElement} */ ($(`[data-tp-trend="${tr.key}"]`, strip));
    const value = /** @type {HTMLElement} */ ($('.kp-kpi__value', tile));
    const words = /** @type {HTMLElement} */ ($('.kp-kpi__trend', tile));
    const figure = /** @type {HTMLElement} */ ($('.kp-kpi__chart', tile));
    if (trend.state === 'error') {
        value.textContent = number(totalsNow(tr.key));
        words.textContent = '';
        setTrendData(figure, { error: 'Could not read the hourly history', text: 'The hub did not answer. The numbers on the Topics page are still current.' });
        return;
    }
    if (trend.state === 'loading') {
        skeleton(value, '4ch');
        skeleton(words, '80%');
        setTrendData(figure, null);
        return;
    }
    if (trend.state === 'empty') {
        value.textContent = number(totalsNow(tr.key));
        words.textContent = 'collecting the first 24 hours';
        setTrendData(figure, { points: [], step: SAMPLE_STEP, unit: '', digits: 0 });
        return;
    }
    const series = trend.samples.map((s) => /** @type {[number, number]} */ ([s.at, Number(s[/** @type {'messages'} */ (tr.key)] ?? 0)]));
    value.textContent = number(series[series.length - 1][1]);
    words.replaceChildren(...deltaWords(tr, series));
    setTrendData(figure, { points: series, step: SAMPLE_STEP, unit: '', digits: 0 });
}

export function drawTrends() {
    const strip = $('[data-tp-trends]');
    if (!strip) return;
    if (strip.children.length === 0) buildTiles(strip);
    for (const tr of TRENDS) drawTile(strip, tr);
}

/** The figure the page itself shows now, for a tile that has no history yet. @param {string} key */
function totalsNow(key) {
    const t = totals();
    return key === 'in_flight' ? t.inflight : key === 'messages' ? t.messages : key === 'backlog' ? t.backlog : t.dead;
}

export async function loadTrends() {
    trend.state = 'loading';
    drawTrends();
    try {
        const data = await readJson('/dashboard/stats/totals?hours=24');
        trend.samples = Array.isArray(data.samples) ? data.samples : [];
        trend.state = trend.samples.length >= 2 ? 'ready' : 'empty';
    } catch {
        trend.samples = [];
        trend.state = 'error';
    }
    drawTrends();
}
$('[data-tp-trends]')?.addEventListener(CHART_RETRY_EVENT, () => void loadTrends());
