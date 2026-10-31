// kyu: topics-state.js, the Topics page's shared data (the model read from the page, mark colours) and its constants.

import { idmarkExports } from '/static/kp/dist/kp-themes.min.js';
import { $ } from './topics-util.js';
const { setIdmark, idmarkColours } = idmarkExports;

export const HOUR = 3_600_000;
/** A topic counts as quiet after this long, in seconds (the server's status word uses the same 24 h). */
export const QUIET_AFTER = 86_400;
export const STATUS_BADGE = { 'Dead letters': 'kp-badge--destructive', Backlog: 'kp-badge--warning', Quiet: 'kp-badge--info', Healthy: 'kp-badge--success' };

/**
 * @typedef {object} Topic
 * @property {string} name
 * @property {keyof typeof STATUS_BADGE} status
 * @property {number} messages
 * @property {string[]} subscriptions
 * @property {string[]} flagged
 * @property {number} backlog
 * @property {number} in_flight
 * @property {number} dead
 * @property {number} lapsed
 * @property {number | null} last_published_ms
 * @property {string | null} dead_note
 */

/** The page's data block, taken out of the DOM once read. @returns {{ now_ms: number, topics: Topic[] } | null} */
export function takeData() {
    const block = $('[data-tp-data]');
    if (!block) return null;
    block.remove();
    try {
        return JSON.parse(block.textContent ?? '');
    } catch {
        return null;
    }
}

/** What the Topics page shares and reassigns: the data, and the mark colours. */
export const M = {
    /** @type {{ now_ms: number, topics: Topic[] }} */
    model: { now_ms: Date.now(), topics: [] },
    slots: idmarkColours([]),
};
/** The key-figure filters that are pressed, and whether the first render and the kit boot are done. */
export const ui = { pressed: { backlog: false, dead: false }, ready: false, failed: false };

export const totals = () => ({
    topics: M.model.topics.length,
    subscriptions: M.model.topics.reduce((n, x) => n + x.subscriptions.length, 0),
    messages: M.model.topics.reduce((n, x) => n + x.messages, 0),
    backlog: M.model.topics.reduce((n, x) => n + x.backlog, 0),
    inflight: M.model.topics.reduce((n, x) => n + x.in_flight, 0),
    dead: M.model.topics.reduce((n, x) => n + x.dead, 0),
    lapsed: M.model.topics.reduce((n, x) => n + x.lapsed, 0),
});

/** The marks never collide while the palette has room: the slots are decided from the topics in order. */
export const decideSlots = () => {
    M.slots = idmarkColours(M.model.topics.map((t) => t.name));
};

/** @param {HTMLElement} el @param {string} name */
export function mark(el, name) {
    setIdmark(el, name, { slot: M.slots.get(name) });
}
