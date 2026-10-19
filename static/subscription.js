// kyu: subscription.js — the Subscription page on kp-themes 11.0.0.
//
// The server renders the header, the policy and the backlog table (so the page
// reads without a script); this file brings the rest to life: the marks and
// freshness line, the gauges, bulk delete with Undo, the attempts timeline and
// request inspector for one message, and the 90-day strip. The data of the
// page is the JSON block `#ky-page-data`; the rest is fetched from the
// dashboard routes under `/t/{topic}/dashboard/subs/{name}`.
import { setIdmark, idmarkColours } from '/static/kp/js/idmark.js';
import { setAgo, agoMoment } from '/static/kp/js/freshness.js';
import { setBullets } from '/static/kp/js/bullets.js';
import { setProgress, setIndeterminate, setProgressFailed } from '/static/kp/js/progressbar.js';
import { dataTable } from '/static/kp/js/datatable.js';
import { undoToast } from '/static/kp/js/undo-toast.js';
import { setSelectionResult } from '/static/kp/js/selection-bar.js';
import { GANTT_STEPS, setGanttRuns, setGanttState, setGanttPoints, setGanttKinds, setGanttZones, setGanttWindow } from '/static/kp/js/gantt.js';
import { setRequests, setRequestsState } from '/static/kp/js/request-inspector.js';
import { setCardState, CARD_RETRY_EVENT } from '/static/kp/js/card-state.js';
import { setUptime } from '/static/kp/js/uptime.js';
import { setCode } from '/static/kp/js/code.js';
import { setStrings } from '/static/kp/js/strings.js';

/** @param {string} sel @param {ParentNode} [root] @returns {HTMLElement | null} */
const $ = (sel, root = document) => /** @type {HTMLElement | null} */ (root.querySelector(sel));
/** @param {string} sel @param {ParentNode} [root] @returns {HTMLElement[]} */
const $$ = (sel, root = document) => /** @type {HTMLElement[]} */ ([...root.querySelectorAll(sel)]);

const DATA = JSON.parse($('#ky-page-data')?.textContent ?? '{}');
const BASE = `/t/${encodeURIComponent(DATA.topic)}/dashboard`;
const SUB = `${BASE}/subs/${encodeURIComponent(DATA.subscription)}`;
const SEC = 1000;

/** Message id to what the backlog knows of it; shrinks as messages are deleted. @type {Map<string, { state: string, attempts: number, published_ms: number }>} */
const items = new Map(DATA.items.map((/** @type {any} */ item) => [item.id, item]));

// kyu's own words on the uptime strip: a day is judged by what was delivered, not by what was up.
const STATE_WORDS = /** @type {Record<string, string>} */ ({ up: 'all delivered', degraded: 'some dead', down: 'mostly dead' });
setStrings({
    uptimeIncidents: (n) => `${n} dead`,
    uptimeSummary: (percent, n) => `${percent}% delivered, ${n} dead`,
    uptimeStateWord: (state) => STATE_WORDS[state] ?? 'no messages',
    uptimeCell: (label, state, percent, n) =>
        [label, STATE_WORDS[state] ?? 'no messages', percent === null ? null : `${percent} delivered`, n === null ? null : `${n} dead`]
            .filter(Boolean)
            .join(' · '),
    uptimeInfoFigures: (percent, n) =>
        [percent === null ? null : `${percent}% delivered`, n === null ? null : `${n} dead`].filter(Boolean).join(' · '),
});

/** Run once the package's own boot (js/auto.js) has attached everything the page carries. @param {() => void} run */
function onReady(run) {
    const root = document.documentElement;
    if (root.hasAttribute('data-kp-auto-ready')) return run();
    new MutationObserver((_, obs) => {
        if (!root.hasAttribute('data-kp-auto-ready')) return;
        obs.disconnect();
        run();
    }).observe(root, { attributes: true });
}

/** @param {string} url @param {RequestInit} [init] */
async function getJson(url, init) {
    const response = await fetch(url, { credentials: 'same-origin', headers: { Accept: 'application/json', ...(init?.headers ?? {}) }, ...init });
    if (!response.ok) {
        /** @type {any} */
        let body = null;
        try {
            body = await response.json();
        } catch {
            /* an answer that is not JSON has no remedy to show */
        }
        throw new Error(body?.error ?? `kyu answered ${response.status}`);
    }
    return response.json();
}

/** A duration in words. @param {number} seconds */
function words(seconds) {
    const s = Math.round(seconds);
    if (s < 60) return `${s} s`;
    if (s < 3600) return `${Math.floor(s / 60)} min`;
    if (s < 86_400) return `${Math.floor(s / 3600)} h`;
    const d = Math.floor(s / 86_400);
    return `${d} ${d === 1 ? 'day' : 'days'}`;
}

/** dd/mm/yyyy HH:mm:ss in Europe/Brussels, for the lane names. @param {number} ms */
const withSeconds = (ms) => `${agoMoment(ms)}:${String(Math.floor(ms / 1000) % 60).padStart(2, '0')}`;

/** The uptime/daily statistics, fetched once and shared by the strip and the progress bar. @type {Promise<{ days: { day: string, delivered: number, dead: number }[] }>} */
let dailyAnswer;
const daily = () => (dailyAnswer ??= getJson(`${SUB}/daily?days=90`));

// ---------------------------------------------------------------- the header
function initHeader() {
    const names = [DATA.topic, DATA.subscription];
    const slots = idmarkColours(names);
    const topicMark = $('[data-ky-mark-topic]');
    const mark = $('[data-ky-mark]');
    if (topicMark) setIdmark(topicMark, names[0], { slot: slots[0] });
    if (mark) setIdmark(mark, names[1], { slot: slots[1] });
    const ago = $('[data-ky-ago]');
    if (ago) setAgo(ago, DATA.last_poll_ms ?? null);
    for (const el of $$('[data-ky-moment]')) el.textContent = agoMoment(Number(el.dataset.kyMoment));
}

// ---------------------------------------------------------------- the gauges
function counts() {
    let pending = 0;
    let claimed = 0;
    for (const item of items.values()) (item.state === 'claimed' ? (claimed += 1) : (pending += 1));
    return { pending, claimed };
}

/** Today's delivered count, once the daily statistics answered; null before and after a failure. @type {number | null} */
let deliveredToday = null;
/** Where the daily statistics are: the drain bar is indeterminate while they load. @type {'loading' | 'ready' | 'error'} */
let dailyState = 'loading';
/** Where the read of this page is (the policy, the gauges, the backlog): every one of them shows the same state. @type {'loading' | 'ready' | 'error'} */
let pageState = 'loading';

function drawGauges() {
    const age = $('#ky-bullets-age');
    const attempts = $('#ky-bullets-attempts');
    if (!age || !attempts) return;
    const lease = DATA.lease_ms / SEC;
    const max = DATA.max_attempts;
    const { pending, claimed } = counts();
    const held = [...items.values()].filter((item) => item.state === 'pending' || item.state === 'claimed');
    // Only the oldest message still on the page counts when the page lists fewer than exist.
    const oldest = held.length === 0 ? null : Math.min(...held.map((item) => item.published_ms));
    const common = { sort: /** @type {const} */ ('worst'), emptyTitle: 'Nothing is held', emptyText: `${DATA.subscription} has acknowledged everything it was handed.` };
    if (pageState !== 'ready') {
        // The page is being read, or kyu did not answer: both gauges say so with their own states (and Retry).
        const state = /** @type {'loading' | 'error'} */ (pageState);
        for (const el of [age, attempts]) setBullets(el, { ...common, state, message: 'Could not read the gauges', emptyText: 'kyu did not answer.', rows: [], threshold: 1, max: 2 });
        const bar = $('#ky-drain');
        const readout = $('#ky-drain-readout');
        if (bar && state === 'error') setProgressFailed(bar, { title: 'Delivery counts for today are not available' });
        else if (bar) setIndeterminate(bar, true);
        if (readout) readout.textContent = '';
        return;
    }
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
    const tried = held.filter((item) => item.attempts > 0);
    setBullets(attempts, {
        ...common,
        state: tried.length === 0 ? 'empty' : 'ready',
        emptyTitle: 'No message has been tried yet',
        emptyText: 'Every message in the backlog is waiting for its first attempt.',
        threshold: max,
        bad: max,
        max: max + 1,
        format: (n) => `${n} of ${max}`,
        rows: tried.map((item) => ({ key: item.id, label: `Published ${agoMoment(item.published_ms)}`, detail: `${item.id} · ${item.state}`, value: item.attempts, pick: true })),
    });
    const bar = $('#ky-drain');
    const readout = $('#ky-drain-readout');
    if (!bar || !readout) return;
    if (dailyState === 'loading') {
        // The package's own busy bar while the answer is on its way.
        setIndeterminate(bar, true);
        readout.textContent = '';
        return;
    }
    setIndeterminate(bar, false);
    if (deliveredToday === null) {
        setProgressFailed(bar, { title: 'Delivery counts for today are not available', text: 'kyu did not answer.' });
        readout.textContent = '';
        return;
    }
    const total = deliveredToday + pending + claimed;
    bar.setAttribute('aria-valuemax', String(Math.max(1, total)));
    setProgress(bar, deliveredToday);
    const percent = total === 0 ? 0 : Math.floor((deliveredToday / total) * 100);
    readout.textContent =
        pending + claimed === 0
            ? `${deliveredToday} of ${total} delivered today (${total === 0 ? 0 : 100}%), nothing left`
            : `${deliveredToday} of ${total} delivered today (${percent}%)`;
}

function drawCounts() {
    const { pending, claimed } = counts();
    for (const el of $$('[data-ky-count]')) el.textContent = String(el.dataset.kyCount === 'pending' ? pending : claimed);
}

// ---------------------------------------------------------------- the backlog
/** Set by initBacklog: take the rows of a freshly read page. @type {(doc: Document) => void} */
let swapRows = () => undefined;
function initBacklog() {
    const wrap = $('#ky-backlog-table');
    const bar = $('#ky-bar');
    if (!wrap || !bar) return;
    const handle = () => dataTable(wrap);
    /** @param {string[]} keys */
    const rowsOf = (keys) => keys.flatMap((k) => $$(`[data-kp-row-key="${CSS.escape(k)}"]`, wrap));
    /** @type {Map<string, HTMLElement>} */
    const byKey = new Map($$('[data-kp-row-key]', wrap).map((row) => [row.dataset.kpRowKey ?? '', row]));
    /** Rows taken out of the table while an Undo window is open, with the row each stood before. @type {Map<HTMLElement, Element | null>} */
    const parked = new Map();
    const body = /** @type {HTMLElement} */ ($('tbody', wrap));
    // The table repaints `hidden` on every refresh(), so a row that is gone for now is taken out of the DOM and put back on Undo.
    const park = (/** @type {HTMLElement[]} */ rows) => {
        for (const row of rows) {
            parked.set(row, row.nextElementSibling);
            row.remove();
        }
        handle()?.refresh();
    };
    const restore = (/** @type {HTMLElement[]} */ rows) => {
        for (const row of rows) {
            const before = parked.get(row) ?? null;
            body.insertBefore(row, before && before.parentNode === body ? before : null);
            parked.delete(row);
        }
        handle()?.refresh();
    };
    /**
     * Delete the way the bulk route works: the rows go at once, Undo is offered for a few seconds, and kyu is asked when the window closes.
     * @param {string[]} keys @param {number} ms
     */
    const remove = (keys, ms) => {
        const rows = rowsOf(keys);
        for (const row of rows) {
            const box = /** @type {HTMLInputElement | null} */ ($('[data-kp-select-row]', row));
            if (box?.checked) {
                box.checked = false;
                box.dispatchEvent(new Event('change', { bubbles: true }));
            }
        }
        park(rows);
        const count = rows.length;
        undoToast(`${count} ${count === 1 ? 'message' : 'messages'} deleted from ${DATA.subscription}`, {
            ms,
            max: 3,
            commit: async () => {
                const answer = await getJson(`${BASE}/bulk/delete`, {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ subscription: DATA.subscription, ids: keys }),
                });
                const refused = /** @type {{ id: string, reason: string }[]} */ (answer.refused ?? []);
                const done = /** @type {string[]} */ (answer.done ?? []);
                if (done.length === 0 && refused.length > 0) throw new Error(refused[0].reason);
                restore(refused.flatMap((r) => (byKey.get(r.id) ? [/** @type {HTMLElement} */ (byKey.get(r.id))] : [])));
                for (const id of done) {
                    const row = byKey.get(id);
                    if (row) parked.delete(row);
                    items.delete(id);
                }
                if (refused.length > 0) setSelectionResult(bar, { done: done.length, refused: refused.map((r) => ({ key: r.id, reason: r.reason })) });
                drawCounts();
                drawGauges();
            },
            undo: () => restore(rows),
        });
    };
    document.addEventListener('kp-selection-action', (event) => {
        const e = /** @type {CustomEvent<{ action: string, keys: string[], matching?: boolean, undoMs: number }>} */ (event);
        if (e.target !== bar || e.detail.action !== 'delete') return;
        const keys = e.detail.matching ? $$('[data-kp-row-key]', wrap).map((row) => row.dataset.kpRowKey ?? '') : e.detail.keys;
        remove(keys, e.detail.undoMs);
    });
    // The form post is the no-script path; with a script the same button deletes with Undo.
    wrap.addEventListener('submit', (event) => {
        const form = event.target instanceof Element ? event.target.closest('.ky-row-form') : null;
        if (!form) return;
        event.preventDefault();
        const id = /** @type {HTMLInputElement | null} */ (form.querySelector('input[name="id"]'))?.value;
        if (id) remove([id], 6000);
    });
    /**
     * Put the rows of a freshly read page in the table (a read while an Undo window is open leaves them alone).
     * @param {Document} doc
     */
    swapRows = (doc) => {
        // Rows waiting out an Undo window stay out until it closes.
        const waiting = new Set([...parked.keys()].map((row) => row.dataset.kpRowKey));
        const fresh = $$('#ky-backlog-table tbody tr[data-kp-row-key]', doc)
            .filter((row) => !waiting.has(row.dataset.kpRowKey))
            .map((row) => /** @type {HTMLElement} */ (document.importNode(row, true)));
        body.replaceChildren(...fresh);
        byKey.clear();
        for (const row of fresh) byKey.set(row.dataset.kpRowKey ?? '', row);
        for (const el of $$('[data-ky-moment]', body)) el.textContent = agoMoment(Number(el.dataset.kyMoment));
        void import('/static/kp/js/auto.js').then((auto) => auto.attachAll(body));
        handle()?.refresh();
    };
    wrap.addEventListener('click', (event) => {
        const button = event.target instanceof Element ? event.target.closest('[data-ky-attempts]') : null;
        const id = button?.getAttribute('data-ky-attempts');
        if (!id) return;
        void loadAttempts(id);
        $('#ky-attempts')?.scrollIntoView({ block: 'start', behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth' });
    });
}

// ---------------------------------------------------------------- the attempts of one message
const OUTCOME_WORDS = /** @type {Record<string, string>} */ ({ expired: 'Lease expired', nacked: 'Nack', dead: 'Dead', acked: 'Acked', in_flight: 'Claimed, still held' });
const KIND = /** @type {Record<string, string>} */ ({ expired: 'expired', nacked: 'nack', dead: 'dead', acked: 'ack' });
const STATUS = /** @type {Record<string, number>} */ ({ expired: 409, nacked: 422, dead: 410, acked: 200 });

/** @typedef {{ n: number, claimed_at: number, ended_at: number | null, outcome: string, app?: string | null, reason?: string | null }} Attempt */

let currentMessage = '';

/** @param {string} id */
async function loadAttempts(id) {
    const gantt = $('#ky-gantt');
    const requests = $('#ky-requests');
    const title = $('#ky-attempts-title');
    if (!gantt || !requests || !title) return;
    currentMessage = id;
    title.textContent = `Message ${id}`;
    setGanttState(gantt, 'loading', 'Reading the attempts');
    setRequestsState(requests, 'loading');
    let answer;
    try {
        answer = await getJson(`${SUB}/attempts?id=${encodeURIComponent(id)}`);
    } catch {
        if (currentMessage !== id) return;
        setGanttState(gantt, 'error', 'Could not read the attempts', { text: 'kyu did not answer for this message.' });
        setRequestsState(requests, 'error', { message: 'kyu could not read the attempts of this message.' });
        return;
    }
    if (currentMessage !== id) return;
    drawAttempts(gantt, requests, title, id, /** @type {Attempt[]} */ (answer.attempts ?? []));
}

/** @param {HTMLElement} gantt @param {HTMLElement} requests @param {HTMLElement} title @param {string} id @param {Attempt[]} attempts */
function drawAttempts(gantt, requests, title, id, attempts) {
    const published = items.get(id)?.published_ms;
    const count = attempts.length;
    title.textContent = `Message ${id} on ${DATA.topic}${published ? `, published ${agoMoment(published)}` : ''}: ${count} ${count === 1 ? 'attempt' : 'attempts'}`;
    const lanes = attempts.map((a) => ({ id: `a${a.n}`, label: `Attempt ${a.n}`, sub: withSeconds(a.claimed_at) }));
    const runs = attempts.map((a) => ({
        id: `r${a.n}`,
        lane: `a${a.n}`,
        label: OUTCOME_WORDS[a.outcome] ?? a.outcome,
        start: a.claimed_at,
        end: a.ended_at,
        state: /** @type {any} */ (a.outcome === 'acked' ? 'ok' : a.outcome === 'in_flight' ? 'running' : 'failed'),
        detail: a.reason ?? (a.outcome === 'acked' ? 'acknowledged' : a.app ? `held by ${a.app}` : ''),
    }));
    const points = attempts
        .filter((a) => a.ended_at !== null)
        .map((a) => ({ id: `p${a.n}`, lane: `a${a.n}`, at: /** @type {number} */ (a.ended_at), kind: KIND[a.outcome] ?? a.outcome, label: OUTCOME_WORDS[a.outcome] ?? a.outcome, detail: a.reason ?? 'acknowledged' }));
    const zones = attempts.map((a) => ({ id: `z${a.n}`, lane: `a${a.n}`, from: a.claimed_at, to: a.claimed_at + DATA.lease_ms, label: `Lease (${words(DATA.lease_ms / SEC)})`, tone: /** @type {any} */ ('info') }));
    setGanttRuns(gantt, runs, lanes);
    setGanttPoints(gantt, points, lanes);
    setGanttZones(gantt, zones);
    if (count === 0) {
        setGanttState(gantt, 'empty', `Not claimed yet: nothing to show until ${DATA.subscription} reads it.`);
    } else {
        setGanttState(gantt, 'ready');
        const running = attempts.some((a) => a.ended_at === null);
        const first = Math.min(...attempts.map((a) => a.claimed_at));
        const last = running ? Date.now() : Math.max(...attempts.map((a) => a.ended_at ?? a.claimed_at));
        const need = Math.max(60, ((last - first) / SEC) * 1.2);
        const span = GANTT_STEPS.find((step) => step >= need) ?? GANTT_STEPS[GANTT_STEPS.length - 1];
        setGanttWindow(gantt, running ? { span, live: true } : { span, live: false, to: last + span * 0.1 * SEC });
    }
    const list = attempts
        .map((a) => {
            const body = JSON.stringify(
                { attempt: a.n, of: DATA.max_attempts, claimed_at: new Date(a.claimed_at).toISOString(), ended_at: a.ended_at === null ? null : new Date(a.ended_at).toISOString(), outcome: a.outcome, reason: a.reason ?? null },
                null,
                2,
            );
            /** @type {Record<string, string>} */
            const headers = { 'x-kyu-subscription': DATA.subscription, 'x-kyu-outcome': a.outcome };
            if (a.app) headers['x-kyu-app'] = a.app;
            return {
                at: a.ended_at ?? a.claimed_at,
                method: 'ATTEMPT',
                path: `/t/${DATA.topic}/messages/${id}/attempts/${a.n}`,
                status: STATUS[a.outcome],
                headers,
                body,
                bodyBytes: new TextEncoder().encode(body).length,
            };
        })
        .reverse();
    setRequests(requests, list);
}

/** The timeline and the inspector are one read: both show the busy card until kyu has said which message to look at. */
function holdAttempts() {
    const gantt = $('#ky-gantt');
    const requests = $('#ky-requests');
    if (gantt) setGanttState(gantt, 'loading', 'Reading the attempts');
    if (requests) setRequestsState(requests, 'loading');
}

function wireAttempts() {
    const gantt = $('#ky-gantt');
    const requests = $('#ky-requests');
    const title = $('#ky-attempts-title');
    if (!gantt || !requests || !title) return;
    setGanttKinds(gantt, [
        { kind: 'expired', label: 'Lease expired', shape: 'ring', tone: 'warning' },
        { kind: 'nack', label: 'Nack', shape: 'triangle', tone: 'destructive' },
        { kind: 'dead', label: 'Dead', shape: 'square', tone: 'destructive' },
        { kind: 'ack', label: 'Acked', shape: 'diamond', tone: 'success' },
    ]);
    // Before a message is picked there is nothing to read again but the page itself.
    const retry = () => void (currentMessage ? loadAttempts(currentMessage) : startAttempts());
    requests.addEventListener('kp-requests-retry', retry);
    gantt.addEventListener('kp-gantt-retry', retry);
    document.addEventListener('kp-bullets-pick', (event) => {
        const key = /** @type {CustomEvent<{ key: string }>} */ (event).detail?.key;
        if (key && items.has(key)) void loadAttempts(key);
    });
}

/** Read the page, then show the attempts of the message that is closest to dying (the oldest when none was tried yet). */
async function startAttempts() {
    const gantt = $('#ky-gantt');
    const requests = $('#ky-requests');
    const title = $('#ky-attempts-title');
    if (!gantt || !requests || !title) return;
    holdAttempts();
    if (!(await readPage())) {
        setGanttState(gantt, 'error', 'Could not read the attempts', { text: 'kyu did not answer.' });
        setRequestsState(requests, 'error', { message: 'kyu could not read the backlog, so no message is picked.' });
        return;
    }
    // Someone pressed Attempts on a row while the page was being read: that message stays.
    if (currentMessage) return;
    const held = [...items.entries()];
    if (held.length === 0) {
        title.textContent = `${DATA.subscription} holds no message right now.`;
        setGanttState(gantt, 'empty', 'Nothing in the backlog, so no attempts to show.');
        setRequests(requests, []);
        return;
    }
    const [pick] = held.reduce((best, entry) => (entry[1].attempts > best[1].attempts ? entry : best));
    void loadAttempts(pick);
}

// ---------------------------------------------------------------- the 90 days
/** @param {string} key YYYY-MM-DD @param {number} back */
function dayBefore(key, back) {
    const [y, m, d] = key.split('-').map(Number);
    return new Date(Date.UTC(y, m - 1, d - back)).toISOString().slice(0, 10);
}

async function initUptime() {
    dailyAnswer = undefined;
    dailyState = 'loading';
    drawGauges();
    const el = $('#ky-uptime-strip');
    if (!el) return;
    setUptime(el, { label: DATA.subscription, state: 'loading', count: 90, from: '90 days ago', to: 'Today' });
    let answer;
    try {
        answer = await daily();
    } catch {
        dailyState = 'error';
        setUptime(el, { label: DATA.subscription, state: 'error', error: 'Could not read the daily statistics', text: 'kyu did not answer.', count: 90, from: '90 days ago', to: 'Today' });
        drawGauges();
        return;
    }
    /** @type {Map<string, { delivered: number, dead: number }>} */
    const byDay = new Map(answer.days.map((/** @type {any} */ d) => [d.day, d]));
    const newest = answer.days.length > 0 ? answer.days[answer.days.length - 1].day : new Date().toISOString().slice(0, 10);
    dailyState = 'ready';
    deliveredToday = byDay.get(newest)?.delivered ?? 0;
    const periods = Array.from({ length: 90 }, (_, i) => {
        const key = dayBefore(newest, 89 - i);
        const label = key.split('-').reverse().join('/');
        const day = byDay.get(key);
        if (!day || day.delivered + day.dead === 0) return { key, label, state: /** @type {any} */ ('none'), tip: `${label} · no messages` };
        const share = day.delivered / (day.delivered + day.dead);
        return {
            key,
            label,
            state: /** @type {any} */ (day.dead === 0 ? 'up' : share < 0.5 ? 'down' : 'degraded'),
            uptime: share,
            incidents: day.dead,
            details: [`${day.delivered} delivered`, day.dead === 0 ? 'nothing dead' : `${day.dead} dead-lettered`],
        };
    });
    const any = periods.some((p) => p.state !== 'none');
    setUptime(el, { label: DATA.subscription, from: '90 days ago', to: 'Today', state: any ? 'ready' : 'empty', open: 'info', periods: any ? periods : [], count: 90 });
    drawGauges();
}

$('#ky-uptime-strip')?.addEventListener('kp-uptime-retry', () => void initUptime());
$('#ky-drain')?.addEventListener('kp-progressbar-retry', () => void (pageState === 'error' ? readPage() : initUptime()));

// ---------------------------------------------------------------- reading the page itself
const policyCard = () => /** @type {HTMLElement | null} */ ($('#ky-policy'));
const backlogTable = () => {
    const el = $('#ky-backlog-table');
    return el ? dataTable(el) : null;
};
const PAGE_FAILED = 'kyu did not answer, so this page may be out of date.';

/** The policy card, the gauges and the backlog table are one read of the page: they show its state together. @param {'loading' | 'ready' | 'error'} state */
function showPageState(state) {
    pageState = state;
    const card = policyCard();
    if (card) {
        if (state === 'error') setCardState(card, 'error', { title: 'Could not read the policy', text: 'kyu did not answer.' });
        else setCardState(card, state);
    }
    const table = backlogTable();
    // The payload cells wait with the rows (a cell code view is skeleton while it loads).
    if (state !== 'ready') for (const fig of $$('#ky-backlog-table [data-kp-code]')) setCode(fig, { cell: true, state, message: state === 'error' ? PAGE_FAILED : undefined });
    if (state === 'loading') {
        table?.state('loading');
    } else if (state === 'error') table?.fail(PAGE_FAILED);
    else table?.state('ready');
    drawGauges();
}

/** Read this page again: its data block, the policy and the backlog rows. @returns {Promise<boolean>} whether kyu answered */
async function readPage() {
    showPageState('loading');
    try {
        const response = await fetch(location.pathname + location.search, { credentials: 'same-origin', headers: { Accept: 'text/html' } });
        if (!response.ok) throw new Error(`the page answered ${response.status}`);
        const doc = new DOMParser().parseFromString(await response.text(), 'text/html');
        const block = doc.getElementById('ky-page-data');
        if (!block) throw new Error('the page had no data');
        const fresh = JSON.parse(block.textContent ?? '{}');
        items.clear();
        for (const item of fresh.items ?? []) items.set(item.id, item);
        const policy = $('#ky-policy .kp-kv', doc);
        const here = $('#ky-policy .kp-kv');
        if (policy && here) {
            here.replaceWith(document.importNode(policy, true));
            for (const el of $$('#ky-policy [data-ky-moment]')) el.textContent = agoMoment(Number(el.dataset.kyMoment));
        }
        swapRows(doc);
        drawCounts();
    } catch {
        showPageState('error');
        return false;
    }
    showPageState('ready');
    return true;
}
for (const id of ['ky-bullets-age', 'ky-bullets-attempts']) document.getElementById(id)?.addEventListener('kp-bullets-retry', () => void readPage());
policyCard()?.addEventListener(CARD_RETRY_EVENT, () => void readPage());
$('#ky-backlog-table')?.addEventListener('kp-datatable-retry', () => void readPage());

onReady(() => {
    initHeader();
    pageState = 'loading';
    drawGauges();
    initBacklog();
    wireAttempts();
    holdAttempts();
    void initUptime();
    // The first read is a real one with the components' own states: the server-drawn page stands behind the busy
    // cards until kyu has answered; the attempts wait for it, since the backlog it reads says which message to show.
    void startAttempts();
});
