// kyu: topic-header.js, section 1 of the Topic page: the header (dead badge, facts, last published, URL).

import { freshnessExports } from '/static/kp/dist/kp-themes.min.js';
import { $, num, plural } from './topic-util.js';
import { S, ORIGIN, TOPIC } from './topic-state.js';
const { setAgo } = freshnessExports;

const header = /** @type {HTMLElement} */ ($('[data-kt-header]'));
/** Draw the header from the page data. */
export function renderHeader() {
    const t = S.data.topic;
    const badge = $('[data-kt-dead-badge]', header);
    if (badge) {
        badge.hidden = !(t.dead > 0);
        badge.textContent = `${num(t.dead)} dead`;
    }
    const facts = $('[data-kt-facts]', header);
    if (facts) facts.textContent = `${num(t.messages)} messages retained · backlog ${num(t.backlog)} · ${plural(t.subscriptions, 'subscription', 'subscriptions')}`;
    const last = $('[data-kt-last]', header);
    if (last) {
        last.hidden = !t.last_published_ms;
        if (t.last_published_ms) setAgo(last, t.last_published_ms);
    }
    const url = $('[data-kt-topic-url]', header);
    if (url) url.textContent = `${ORIGIN}/t/${TOPIC}`;
}
