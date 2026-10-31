// kyu: topics-heat.js, the Topics page's heat map: messages published or died per topic per hour, last 24 hours.

import { heatExports } from '/static/kp/dist/kp-themes.min.js';
import { $, $$, h, number, readJson } from './topics-util.js';
import { M } from './topics-state.js';
const { setHeat, HEAT_PICK_EVENT, HEAT_RETRY_EVENT } = heatExports;

/** A whole-hub hour with this many dead letters shows as completely full. */
const HUB_DEAD_FULL_AT = 3;

const brussels = new Intl.DateTimeFormat('en-GB', { timeZone: 'Europe/Brussels', hour: '2-digit', hour12: false });
const hourLabel = (/** @type {number} */ ms) => brussels.format(new Date(ms)).replace(/^24$/, '00').padStart(2, '0');

const heat = {
    /** @type {'loading' | 'ready' | 'empty' | 'error'} */
    state: 'loading',
    metric: /** @type {'published' | 'dead'} */ ('published'),
    picked: /** @type {string | null} */ (null),
    /** @type {{ hours: number[], topics: { topic: string, published: number[], dead: number[] }[] } | null} */
    data: null,
};

/** @typedef {{ hours: number[], topics: { topic: string, published: number[], dead: number[] }[] }} HeatData */

/** One topic's cell in the hour `ms` (index `i`). @param {HeatData['topics'][number]} topic @param {'published' | 'dead'} metric @param {number} ms @param {number} i @param {number} peak */
function topicCell(topic, metric, ms, i, peak) {
    const n = topic[metric][i] ?? 0;
    const when = `${hourLabel(ms)}:00`;
    if (metric === 'dead')
        return {
            tone: /** @type {const} */ (n === 0 ? 'ok' : n === 1 ? 'warn' : 'bad'),
            label: n === 0 ? 'no dead letters' : `${n} ${n === 1 ? 'dead letter' : 'dead letters'}`,
            aria: `${topic.topic} at ${when}: ${n === 0 ? 'no dead letters' : `${n} dead`}`,
            tip: `${topic.topic} at ${when}: ${n === 0 ? 'nothing died' : `${n} ${n === 1 ? 'message' : 'messages'} died`}`,
        };
    return {
        tone: /** @type {const} */ (n === 0 ? 'none' : 'level'),
        level: Math.sqrt(n / peak),
        label: `${number(n)} published`,
        aria: `${topic.topic} at ${when}: ${number(n)} published`,
        tip: `${topic.topic} at ${when}: ${number(n)} ${n === 1 ? 'message' : 'messages'} published`,
    };
}

/** The "whole hub" row: every topic summed per hour. @param {HeatData} data @param {'published' | 'dead'} metric */
function hubRow(data, metric) {
    const hours = data.hours;
    const hubAt = (/** @type {number} */ i) => data.topics.reduce((n, t) => n + (t[metric][i] ?? 0), 0);
    const hubPeak = Math.max(1, ...hours.map((_, i) => hubAt(i)));
    return {
        key: 'hub',
        label: 'Whole hub',
        cells: hours.map((ms, i) => {
            const total = hubAt(i);
            if (metric === 'dead')
                return {
                    tone: /** @type {const} */ (total === 0 ? 'ok' : total === 1 ? 'warn' : 'bad'),
                    share: total === 0 ? 1 : Math.min(1, total / HUB_DEAD_FULL_AT),
                    label: total === 0 ? 'no dead letters' : `${total} dead`,
                    tip: `Whole hub at ${hourLabel(ms)}:00: ${total === 0 ? 'nothing died' : `${total} died`}`,
                };
            return {
                tone: /** @type {const} */ ('level'),
                level: Math.sqrt(total / hubPeak),
                share: total / hubPeak,
                label: `${number(total)} published`,
                tip: `Whole hub at ${hourLabel(ms)}:00: ${number(total)} published`,
            };
        }),
    };
}

function heatSpec() {
    const data = heat.data;
    if (!data) return { columns: [], rows: [] };
    const metric = heat.metric;
    const hours = data.hours;
    const peak = Math.max(1, ...data.topics.flatMap((t) => t.published));
    const columns = hours.map((ms, i) => ({ key: new Date(ms).toISOString(), label: i % 3 === 0 || i === hours.length - 1 ? hourLabel(ms) : '' }));
    const rows = data.topics.map((topic) => ({
        key: topic.topic,
        label: topic.topic,
        mark: `var(--chart-${(M.slots.get(topic.topic) ?? 0) + 1})`,
        cells: hours.map((ms, i) => topicCell(topic, metric, ms, i, peak)),
    }));
    return {
        columns,
        rows,
        summary: hubRow(data, metric),
        every: 3,
        label: `Messages ${metric === 'dead' ? 'that died' : 'published'} per topic per hour, last 24 hours`,
        toggle: true,
        picked: heat.picked,
    };
}

export function drawHeat() {
    const grid = $('[data-tp-heat]');
    if (!grid) return;
    setHeat(grid, {
        ...heatSpec(),
        state: /** @type {any} */ (heat.state),
        skeleton: { rows: Math.max(M.model.topics.length, 3), columns: 24 },
        message: heat.state === 'empty' ? 'No history yet. kyu has not counted a full hour.' : 'Could not read the hourly counts',
        text: heat.state === 'error' ? 'The hub did not answer.' : undefined,
    });
    const legend = $('[data-tp-heat-legend]');
    if (legend) {
        const item = (/** @type {string} */ tone, /** @type {string} */ text, /** @type {string} */ level = '') =>
            h('li', {}, h('span', { class: 'kp-heat__swatch', 'data-kp-tone': tone, style: level ? `--kp-heat-level: ${level}` : undefined }), ` ${text}`);
        legend.replaceChildren(
            ...(heat.metric === 'dead'
                ? [item('ok', 'nothing died'), item('warn', 'one died'), item('bad', 'two or more died')]
                : [item('none', 'nothing published'), item('level', 'a few', '0.2'), item('level', 'some', '0.55'), item('level', 'the most', '1')]),
        );
    }
}

export async function loadHeat() {
    heat.state = 'loading';
    drawHeat();
    try {
        const data = await readJson('/dashboard/stats/hourly?hours=24');
        heat.data = data;
        heat.state = data.topics?.length && data.hours?.length ? 'ready' : 'empty';
    } catch {
        heat.data = null;
        heat.state = 'error';
    }
    drawHeat();
}

document.addEventListener(HEAT_PICK_EVENT, (event) => {
    const detail = /** @type {CustomEvent} */ (event).detail;
    const readout = $('[data-tp-heat-readout]');
    if (!readout || !(event.target instanceof Element) || !event.target.matches('[data-tp-heat]')) return;
    heat.picked = detail.picked ? detail.colKey : null;
    const data = heat.data;
    const i = data ? data.hours.findIndex((ms) => new Date(ms).toISOString() === detail.colKey) : -1;
    if (!detail.picked || !data || i < 0) {
        readout.textContent = 'No hour pinned. Press a cell to pin its hour.';
        return;
    }
    const published = data.topics.reduce((n, t) => n + (t.published[i] ?? 0), 0);
    const dead = data.topics.reduce((n, t) => n + (t.dead[i] ?? 0), 0);
    readout.textContent = `${hourLabel(data.hours[i])}:00 pinned: ${number(published)} published across the hub, ${dead === 0 ? 'none died' : `${dead} died`}.`;
});
for (const b of $$('[data-tp-heat-metric]')) {
    b.addEventListener('click', () => {
        heat.metric = /** @type {any} */ (b.getAttribute('data-tp-heat-metric'));
        for (const other of $$('[data-tp-heat-metric]')) other.setAttribute('aria-pressed', String(other === b));
        drawHeat();
    });
}
$('[data-tp-heat]')?.addEventListener(HEAT_RETRY_EVENT, () => void loadHeat());
