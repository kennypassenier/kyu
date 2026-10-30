// kyu: subscription-uptime.js, the 90-day delivery strip of the Subscription page and the daily statistics it shares with the drain bar.

import { setUptime } from '/static/kp/js/uptime.js';
import { $, getJson } from './subscription-util.js';
import { DATA, SUB, P } from './subscription-state.js';
import { drawGauges } from './subscription-gauges.js';

/** The uptime/daily statistics, fetched once and shared by the strip and the progress bar. @type {Promise<{ days: { day: string, delivered: number, dead: number }[] }> | undefined} */
let dailyAnswer;
const daily = () => (dailyAnswer ??= getJson(`${SUB}/daily?days=90`));

/** Days the strip covers. */
const STRIP_DAYS = 90;
/** A day on which fewer than this share of the messages was delivered counts as down. */
const DOWN_BELOW_SHARE = 0.5;

/** @param {string} key YYYY-MM-DD @param {number} back */
function dayBefore(key, back) {
    const [y, m, d] = key.split('-').map(Number);
    return new Date(Date.UTC(y, m - 1, d - back)).toISOString().slice(0, 10);
}

/** The strip's periods, oldest first, from the daily answer. @param {Map<string, { delivered: number, dead: number }>} byDay @param {string} newest */
function periodsOf(byDay, newest) {
    return Array.from({ length: STRIP_DAYS }, (_, i) => {
        const key = dayBefore(newest, STRIP_DAYS - 1 - i);
        const label = key.split('-').reverse().join('/');
        const day = byDay.get(key);
        if (!day || day.delivered + day.dead === 0) return { key, label, state: /** @type {any} */ ('none'), tip: `${label} · no messages` };
        const share = day.delivered / (day.delivered + day.dead);
        return {
            key,
            label,
            state: /** @type {any} */ (day.dead === 0 ? 'up' : share < DOWN_BELOW_SHARE ? 'down' : 'degraded'),
            uptime: share,
            incidents: day.dead,
            details: [`${day.delivered} delivered`, day.dead === 0 ? 'nothing dead' : `${day.dead} dead-lettered`],
        };
    });
}

export async function initUptime() {
    dailyAnswer = undefined;
    P.dailyState = 'loading';
    drawGauges();
    const el = $('#ky-uptime-strip');
    if (!el) return;
    const frame = { label: DATA.subscription, count: STRIP_DAYS, from: '90 days ago', to: 'Today' };
    setUptime(el, { ...frame, state: 'loading' });
    let answer;
    try {
        answer = await daily();
    } catch {
        P.dailyState = 'error';
        setUptime(el, { ...frame, state: 'error', error: 'Could not read the daily statistics', text: 'kyu did not answer.' });
        drawGauges();
        return;
    }
    /** @type {Map<string, { delivered: number, dead: number }>} */
    const byDay = new Map(answer.days.map((/** @type {any} */ d) => [d.day, d]));
    const newest = answer.days.length > 0 ? answer.days[answer.days.length - 1].day : new Date().toISOString().slice(0, 10);
    P.dailyState = 'ready';
    P.deliveredToday = byDay.get(newest)?.delivered ?? 0;
    const periods = periodsOf(byDay, newest);
    const any = periods.some((p) => p.state !== 'none');
    setUptime(el, { ...frame, state: any ? 'ready' : 'empty', open: 'info', periods: any ? periods : [] });
    drawGauges();
}

$('#ky-uptime-strip')?.addEventListener('kp-uptime-retry', () => void initUptime());
