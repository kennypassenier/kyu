// kyu: subscription-header.js, the header of the Subscription page: the marks, the freshness line and the moments.

import { setIdmark, idmarkColours } from '/static/kp/js/idmark.js';
import { setAgo, agoMoment } from '/static/kp/js/freshness.js';
import { $, $$ } from './subscription-util.js';
import { DATA } from './subscription-state.js';

export function initHeader() {
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
