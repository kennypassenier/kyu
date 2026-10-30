// kyu: topic-recent.js, section 6 of the Topic page: the recent-messages list and its master-detail pane wiring.

import { masterDetail, setPaneState } from '/static/kp/js/master-detail.js';
import { statePlate } from '/static/kp/js/state-plate.js';
import { $, make, num, iso, bytesText } from './topic-util.js';
import { S, MESSAGES, TOPIC } from './topic-state.js';
import { stage } from './topic-stage.js';
import { recent, mdRoot, fillDetail, resetDetail } from './topic-detail.js';
import { reload } from './topic-refresh.js';

/** How long a new arrival keeps its data-new mark, in ms. */
const NEW_MARK_MS = 2500;
const recentStage = stage(/** @type {HTMLElement} */ ($('[data-kt-stage="recent"]', recent)));
const items = /** @type {HTMLElement} */ ($('[data-kt-items]', recent));

/** @param {any} m */
const snippet = (m) => (m.payload.binary ? `binary, ${bytesText(m.payload.bytes)}` : m.payload.text === '' ? 'empty payload' : m.payload.text);

/** @param {any} m */
function messageItem(m) {
    const li = make('li');
    li.setAttribute('role', 'option');
    li.setAttribute('data-kp-pane-key', m.id);
    li.dataset.kpKey = m.id;
    const wrap = make('span', 'kt-msg');
    const ago = make('time', 'kp-ago');
    ago.setAttribute('data-kp-ago', '');
    ago.setAttribute('data-kp-ago-verb', 'published');
    ago.setAttribute('datetime', iso(m.published_ms));
    ago.textContent = m.published_at;
    const small = make('small');
    small.append(ago);
    wrap.append(make('b', '', m.id), small, make('code', '', snippet(m)));
    li.append(wrap);
    return li;
}

/** Redraw the list from the page data: new arrivals at the head, gone ones removed, then the detail pane. */
export function renderRecent() {
    const list = S.data.messages;
    MESSAGES.clear();
    for (const m of list) MESSAGES.set(m.id, m);
    resetDetail();
    const count = $('[data-kt-recent-count]', recent);
    if (count) count.textContent = list.length ? `The newest ${num(list.length)} of ${num(S.data.topic.messages)} messages retained on ${TOPIC}.` : '';
    if (list.length === 0) {
        recentStage.plate(statePlate(document, { kind: 'empty', title: 'No messages yet', text: `Messages published to ${TOPIC} show here.` }), 'none');
        return;
    }
    recentStage.ready();
    const known = new Set([...items.children].map((li) => /** @type {HTMLElement} */ (li).dataset.kpKey));
    const fresh = list.filter((/** @type {any} */ m) => !known.has(m.id));
    const wanted = new Set(list.map((/** @type {any} */ m) => m.id));
    for (const li of [...items.children]) if (!wanted.has(/** @type {HTMLElement} */ (li).dataset.kpKey)) li.remove();
    const first = known.size === 0;
    // Newest first: new arrivals go to the head.
    const lis = fresh.map(messageItem);
    items.prepend(...lis);
    if (!first) for (const li of lis) li.setAttribute('data-new', '');
    setTimeout(() => lis.forEach((li) => li.removeAttribute('data-new')), NEW_MARK_MS);
    if (S.booted) {
        for (const li of lis) S.attachAll?.(li);
        const md = masterDetail(mdRoot);
        if (md && md.key() === null) md.select(list[0].id);
        else if (md) fillDetail(String(md.key()));
    }
}

mdRoot.addEventListener('kp-pane-select', (event) => {
    const key = /** @type {CustomEvent} */ (event).detail?.key;
    if (key == null) return;
    fillDetail(String(key));
    setPaneState(mdRoot, 'ready');
});
mdRoot.addEventListener('kp-pane-open', (event) => {
    const key = /** @type {CustomEvent} */ (event).detail?.key;
    if (key == null) return;
    fillDetail(String(key));
    setPaneState(mdRoot, 'ready');
});
mdRoot.addEventListener('kp-pane-retry', () => void reload(true));
// The payload block's own failed plate: its Retry fires kp-code-retry (code.js), which reads the messages again.
mdRoot.addEventListener('kp-code-retry', () => void reload(true));
