// kyu: topics-heat-list.js, the heat map at phone width: a list per day, one 44 px row per hour for the whole hub.

import { h, number } from './topics-util.js';

const clock = new Intl.DateTimeFormat('en-GB', { timeZone: 'Europe/Brussels', hour: '2-digit', hour12: false });
const dayKey = new Intl.DateTimeFormat('en-CA', { timeZone: 'Europe/Brussels' });
const dayName = new Intl.DateTimeFormat('en-GB', { timeZone: 'Europe/Brussels', weekday: 'long', day: 'numeric', month: 'long' });

/** @typedef {{ hours: number[], topics: { topic: string, published: number[], dead: number[] }[] }} HeatData */

/** "Today", "Yesterday" or the date. @param {number} ms @param {number} now */
function dayTitle(ms, now) {
    if (dayKey.format(ms) === dayKey.format(now)) return 'Today';
    if (dayKey.format(ms) === dayKey.format(now - 86_400_000)) return 'Yesterday';
    return dayName.format(ms);
}

/** The hub's count in hour `i`. @param {HeatData} data @param {'published' | 'dead'} metric @param {number} i */
export const hubAt = (data, metric, i) => data.topics.reduce((n, t) => n + (t[metric][i] ?? 0), 0);

/** One hour's row. @param {HeatData} data @param {'published' | 'dead'} metric @param {number} i @param {number} peak @param {boolean} picked */
function hourRow(data, metric, i, peak, picked) {
    const ms = data.hours[i];
    const n = hubAt(data, metric, i);
    const hour = clock.format(ms).replace(/^24$/, '00').padStart(2, '0');
    const words = metric === 'dead' ? (n === 0 ? 'none died' : `${number(n)} died`) : `${number(n)} published`;
    return h(
        'li',
        {},
        h(
            'button',
            {
                type: 'button',
                class: 'tp-heat-row',
                'aria-pressed': String(picked),
                'data-tp-hour': new Date(ms).toISOString(),
                'data-kp-tone': metric === 'dead' && n > 0 ? 'destructive' : undefined,
                style: `--tp-share: ${peak ? n / peak : 0}`,
            },
            h('span', { class: 'tp-heat-row__hour' }, `${hour}:00`),
            h('span', { class: 'tp-heat-row__bar', 'aria-hidden': 'true' }),
            h('span', { class: 'tp-heat-row__count' }, words),
        ),
    );
}

/** Draw the day lists, newest hour first. @param {HTMLElement} host @param {HeatData} data @param {'published' | 'dead'} metric @param {string | null} picked */
export function drawHeatDays(host, data, metric, picked) {
    const peak = Math.max(1, ...data.hours.map((_, i) => hubAt(data, metric, i)));
    const days = new Map();
    for (let i = data.hours.length - 1; i >= 0; i--) {
        const title = dayTitle(data.hours[i], data.hours[data.hours.length - 1]);
        if (!days.has(title)) days.set(title, []);
        days.get(title).push(hourRow(data, metric, i, peak, new Date(data.hours[i]).toISOString() === picked));
    }
    host.replaceChildren(
        ...[...days].map(([title, rows]) => h('section', { class: 'tp-heat-day' }, h('h3', { class: 'tp-heat-day__title' }, title), h('ul', { class: 'tp-heat-day__list' }, ...rows))),
    );
}
