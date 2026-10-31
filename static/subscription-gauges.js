// kyu: subscription-gauges.js, the gauges of the Subscription page: oldest unacked, attempts, the drain bar, and the counts.

import { freshnessExports, bulletsExports, progressbarExports } from '/static/kp/dist/kp-themes.min.js';
import { $, $$, words } from './subscription-util.js';
import { DATA, SEC, items, P } from './subscription-state.js';
const { agoMoment } = freshnessExports;
const { setBullets } = bulletsExports;
const { setProgress, setIndeterminate, setProgressFailed } = progressbarExports;

function counts() {
    let pending = 0;
    let claimed = 0;
    for (const item of items.values()) (item.state === 'claimed' ? (claimed += 1) : (pending += 1));
    return { pending, claimed };
}

/** Both gauges and the drain bar say the page is being read, or that kyu did not answer.
 * @param {HTMLElement} age @param {HTMLElement} attempts @param {Record<string, any>} common @param {'loading' | 'error'} state */
function drawGaugesUnready(age, attempts, common, state) {
    // The page is being read, or kyu did not answer: both gauges say so with their own states (and Retry).
    for (const el of [age, attempts]) setBullets(el, { ...common, state, message: 'Could not read the gauges', emptyText: 'kyu did not answer.', rows: [], threshold: 1, max: 2 });
    const bar = $('#ky-drain');
    const readout = $('#ky-drain-readout');
    if (bar && state === 'error') setProgressFailed(bar, { title: 'Delivery counts for today are not available' });
    else if (bar) setIndeterminate(bar, true);
    if (readout) readout.textContent = '';
}

/** The oldest-unacked gauge against the lease. @param {HTMLElement} age @param {Record<string, any>} common @param {number | null} oldest */
function drawAgeGauge(age, common, oldest) {
    const lease = DATA.lease_ms / SEC;
    setBullets(age, {
        ...common,
        state: oldest === null ? 'empty' : 'ready',
        threshold: lease,
        bad: lease * 2,
        max: lease * 2,
        format: words,
        rows:
            oldest === null
                ? []
                : [{ key: 'oldest', lead: DATA.subscription, label: 'Oldest unacked message', detail: `published ${agoMoment(oldest)}`, value: Math.max(0, (Date.now() - oldest) / SEC) }],
    });
}

/** The attempts gauge: every held message that was tried, against the allowed attempts. @param {HTMLElement} attempts @param {Record<string, any>} common @param {any[]} held */
function drawAttemptsGauge(attempts, common, held) {
    const max = DATA.max_attempts;
    const tried = held.filter((item) => item.attempts > 0);
    setBullets(attempts, {
        ...common,
        state: tried.length === 0 ? 'empty' : 'ready',
        emptyTitle: 'No message has been tried yet',
        emptyText: 'Every message in the backlog is waiting for its first attempt.',
        threshold: max,
        bad: max,
        max: max + 1,
        format: (/** @type {number} */ n) => `${n} of ${max}`,
        rows: tried.map((item) => ({ key: item.id, label: `Published ${agoMoment(item.published_ms)}`, detail: `${item.id} · ${item.state}`, value: item.attempts, pick: true })),
    });
}

/** The drain bar: delivered today against delivered plus what is still held. @param {number} pending @param {number} claimed */
function drawDrain(pending, claimed) {
    const bar = $('#ky-drain');
    const readout = $('#ky-drain-readout');
    if (!bar || !readout) return;
    if (P.dailyState === 'loading') {
        // The package's own busy bar while the answer is on its way.
        setIndeterminate(bar, true);
        readout.textContent = '';
        return;
    }
    setIndeterminate(bar, false);
    if (P.deliveredToday === null) {
        setProgressFailed(bar, { title: 'Delivery counts for today are not available', text: 'kyu did not answer.' });
        readout.textContent = '';
        return;
    }
    const total = P.deliveredToday + pending + claimed;
    bar.setAttribute('aria-valuemax', String(Math.max(1, total)));
    setProgress(bar, P.deliveredToday);
    const percent = total === 0 ? 0 : Math.floor((P.deliveredToday / total) * 100);
    readout.textContent =
        pending + claimed === 0
            ? `${P.deliveredToday} of ${total} delivered today (${total === 0 ? 0 : 100}%), nothing left`
            : `${P.deliveredToday} of ${total} delivered today (${percent}%)`;
}

/** Draw both gauges and the drain bar from the backlog and the page states. */
export function drawGauges() {
    const age = $('#ky-bullets-age');
    const attempts = $('#ky-bullets-attempts');
    if (!age || !attempts) return;
    const { pending, claimed } = counts();
    const held = [...items.values()].filter((item) => item.state === 'pending' || item.state === 'claimed');
    // Only the oldest message still on the page counts when the page lists fewer than exist.
    const oldest = held.length === 0 ? null : Math.min(...held.map((item) => item.published_ms));
    const common = { sort: /** @type {const} */ ('worst'), emptyTitle: 'Nothing is held', emptyText: `${DATA.subscription} has acknowledged everything it was handed.` };
    if (P.pageState !== 'ready') return drawGaugesUnready(age, attempts, common, /** @type {'loading' | 'error'} */ (P.pageState));
    drawAgeGauge(age, common, oldest);
    drawAttemptsGauge(attempts, common, held);
    drawDrain(pending, claimed);
}

export function drawCounts() {
    const { pending, claimed } = counts();
    for (const el of $$('[data-ky-count]')) el.textContent = String(el.dataset.kyCount === 'pending' ? pending : claimed);
}
