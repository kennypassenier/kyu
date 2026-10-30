// kyu: subscription-attempts.js, the attempts of one message: its timeline, its inspector, and picking the message closest to dying.

import { agoMoment } from '/static/kp/js/freshness.js';
import { GANTT_STEPS, setGanttRuns, setGanttState, setGanttPoints, setGanttKinds, setGanttZones, setGanttWindow } from '/static/kp/js/gantt.js';
import { setRequests, setRequestsState } from '/static/kp/js/request-inspector.js';
import { $, getJson, words, withSeconds } from './subscription-util.js';
import { DATA, SUB, SEC, items } from './subscription-state.js';
import { readPage } from './subscription-page.js';

const OUTCOME_WORDS = /** @type {Record<string, string>} */ ({ expired: 'Lease expired', nacked: 'Nack', dead: 'Dead', acked: 'Acked', in_flight: 'Claimed, still held' });
const KIND = /** @type {Record<string, string>} */ ({ expired: 'expired', nacked: 'nack', dead: 'dead', acked: 'ack' });
const STATUS = /** @type {Record<string, number>} */ ({ expired: 409, nacked: 422, dead: 410, acked: 200 });

/** @typedef {{ n: number, claimed_at: number, ended_at: number | null, outcome: string, app?: string | null, reason?: string | null }} Attempt */

let currentMessage = '';


/** @param {string} id */
export async function loadAttempts(id) {
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

/** The timeline: one lane per attempt, the lease zone of each, the outcome marks, and a window that fits them.
 * @param {HTMLElement} gantt @param {Attempt[]} attempts */
function drawTimeline(gantt, attempts) {
    const count = attempts.length;
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
        return;
    }
    setGanttState(gantt, 'ready');
    const running = attempts.some((a) => a.ended_at === null);
    const first = Math.min(...attempts.map((a) => a.claimed_at));
    const last = running ? Date.now() : Math.max(...attempts.map((a) => a.ended_at ?? a.claimed_at));
    const need = Math.max(60, ((last - first) / SEC) * 1.2);
    const span = GANTT_STEPS.find((step) => step >= need) ?? GANTT_STEPS[GANTT_STEPS.length - 1];
    setGanttWindow(gantt, running ? { span, live: true } : { span, live: false, to: last + span * 0.1 * SEC });
}

/** The inspector's list: one pseudo-request per attempt, newest first. @param {string} id @param {Attempt[]} attempts */
function requestList(id, attempts) {
    return attempts
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
}

/** @param {HTMLElement} gantt @param {HTMLElement} requests @param {HTMLElement} title @param {string} id @param {Attempt[]} attempts */
function drawAttempts(gantt, requests, title, id, attempts) {
    const published = items.get(id)?.published_ms;
    const count = attempts.length;
    title.textContent = `Message ${id} on ${DATA.topic}${published ? `, published ${agoMoment(published)}` : ''}: ${count} ${count === 1 ? 'attempt' : 'attempts'}`;
    drawTimeline(gantt, attempts);
    setRequests(requests, requestList(id, attempts));
}

/** The timeline and the inspector are one read: both show the busy card until kyu has said which message to look at. */
export function holdAttempts() {
    const gantt = $('#ky-gantt');
    const requests = $('#ky-requests');
    if (gantt) setGanttState(gantt, 'loading', 'Reading the attempts');
    if (requests) setRequestsState(requests, 'loading');
}

export function wireAttempts() {
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
export async function startAttempts() {
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
