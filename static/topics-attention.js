// kyu: topics-attention.js, the Topics page's attention band: what needs a look, most urgent first.

import { setAttention } from '/static/kp/js/attention.js';
import { $, h, number, plural } from './topics-util.js';
import { M, HOUR } from './topics-state.js';

const band = $('[data-tp-attention]');
/** A topic's backlog counts as worth a warning from this many messages. */
const BACKLOG_WARN_AT = 50;

/** @param {string} name @param {string} label */
const show = (name, label) => h('a', { class: 'kp-button kp-button--sm', href: `#topic-${name}`, 'data-tp-show': name }, label);

/** @typedef {import('/static/kp/js/attention.js').AttentionItem} AttentionItem */

/** @returns {AttentionItem[]} */
function deadItems() {
    return M.model.topics
        .filter((t) => t.dead > 0)
        .map((t) => ({
            key: `dead-${t.name}`,
            severity: /** @type {const} */ ('critical'),
            title: `${t.name}: ${plural(t.dead, 'dead letter', 'dead letters')}`,
            text: `A subscription gave up on ${t.dead === 1 ? 'a message' : 'these messages'} after its last attempt. Open the topic to requeue or delete.`,
            action: show(t.name, 'Show the topic'),
        }));
}

/** @returns {AttentionItem[]} */
function backlogItems() {
    return M.model.topics
        .filter((t) => t.backlog >= BACKLOG_WARN_AT)
        .map((t) => ({
            key: `backlog-${t.name}`,
            severity: /** @type {const} */ ('warning'),
            title: `${t.name}: ${number(t.backlog)} messages waiting`,
            text: 'No subscription has taken them yet; they are safe and will be delivered in order.',
            action: show(t.name, 'Show the topic'),
        }));
}

/** @returns {AttentionItem[]} */
function flaggedItems() {
    return M.model.topics.flatMap((t) =>
        t.flagged.map((sub) => ({
            key: `flagged-${t.name}-${sub}`,
            severity: /** @type {const} */ ('warning'),
            title: `The subscription ${sub} is flagged`,
            text: `It has stopped polling ${t.name}; a flagged subscription is worth a look before it is archived.`,
            action: show(t.name, `Show ${t.name}`),
        })),
    );
}

/** @returns {AttentionItem[]} */
function quietItems() {
    return M.model.topics
        .filter((t) => t.status === 'Quiet' && t.last_published_ms !== null)
        .map((t) => ({
            key: `quiet-${t.name}`,
            severity: /** @type {const} */ ('info'),
            title: `${t.name} has been quiet for ${number(Math.round((M.model.now_ms - /** @type {number} */ (t.last_published_ms)) / HOUR))} h`,
            text: 'Nothing is wrong with it; the publisher may simply have nothing to say.',
            action: show(t.name, 'Show the topic'),
        }));
}

/** @returns {AttentionItem[]} */
function attentionItems() {
    return [...deadItems(), ...backlogItems(), ...flaggedItems(), ...quietItems()];
}

export function drawBand() {
    if (band) setAttention(band, attentionItems());
}
