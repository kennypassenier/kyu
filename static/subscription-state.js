// kyu: subscription-state.js, the Subscription page's shared data: the page's JSON block, its routes,
// the backlog by message id, and the states of its reads.

import { $ } from './subscription-util.js';

export const DATA = JSON.parse($('#ky-page-data')?.textContent ?? '{}');
export const BASE = `/t/${encodeURIComponent(DATA.topic)}/dashboard`;
export const SUB = `${BASE}/subs/${encodeURIComponent(DATA.subscription)}`;
export const SEC = 1000;

/** Message id to what the backlog knows of it; shrinks as messages are deleted. @type {Map<string, { state: string, attempts: number, published_ms: number }>} */
export const items = new Map(DATA.items.map((/** @type {any} */ item) => [item.id, item]));

/** The states the modules share and reassign. */
export const P = {
    /** Today's delivered count, once the daily statistics answered; null before and after a failure. @type {number | null} */
    deliveredToday: null,
    /** Where the daily statistics are: the drain bar is indeterminate while they load. @type {'loading' | 'ready' | 'error'} */
    dailyState: 'loading',
    /** Where the read of this page is (the policy, the gauges, the backlog): every one of them shows the same state. @type {'loading' | 'ready' | 'error'} */
    pageState: 'loading',
};
