// kyu: topic-dead-core.js, dead-letter data helpers: a row's key, the letters grouped by subscription, and the bulk calls.

import { S, T, call, sentence } from './topic-state.js';

export const deadKey = (/** @type {any} */ d) => `dead-${d.subscription}-${d.id}`;

/** The dead letters that are on the page, grouped by subscription. */
export function deadBySub() {
    /** @type {Map<string, any[]>} */
    const map = new Map();
    for (const d of S.data.dead_letters) map.set(d.subscription, [...(map.get(d.subscription) ?? []), d]);
    return map;
}

/** Run one bulk action over [{subscription, id}], one call per subscription.
 * @param {'requeue' | 'delete'} action @param {{ subscription: string, id: string }[]} items */
export async function bulk(action, items) {
    /** @type {Map<string, string[]>} */
    const groups = new Map();
    for (const it of items) groups.set(it.subscription, [...(groups.get(it.subscription) ?? []), it.id]);
    /** @type {{ subscription: string, id: string }[]} */
    const done = [];
    /** @type {{ subscription: string, id: string, reason: string }[]} */
    const refused = [];
    for (const [subscription, ids] of groups) {
        const r = await call(`${T}/bulk/${action}`, { json: { subscription, ids } });
        if (!r.ok) {
            const why = r.status === 404 ? 'Bulk actions are not available on this hub yet.' : sentence(/** @type {any} */ (r));
            for (const id of ids) refused.push({ subscription, id, reason: why });
            continue;
        }
        const answer = r.json ?? { done: ids, refused: [] };
        for (const id of answer.done ?? []) done.push({ subscription, id });
        for (const x of answer.refused ?? []) refused.push({ subscription, id: x.id, reason: x.reason });
    }
    return { done, refused };
}

/** The one plain sentence for a message that stopped being a dead letter while the page still listed it. */
export const NOT_DEAD_ANYMORE = 'That message is no longer a dead letter; the list is refreshed.';

/** The plain refusal of a single requeue the hub answered with a conflict (status 409), or null for any other answer.
 * @param {{ status?: number }} r */
export const plainRefusal = (r) => (r.status === 409 ? { error: NOT_DEAD_ANYMORE, remedy: '' } : null);

/** The title a JSON payload names itself with, quoted and cut, or null. @param {any} payload */
export function payloadTitle(payload) {
    const text = payload?.binary ? '' : payload?.text;
    try {
        const title = text ? JSON.parse(text)?.title : null;
        if (typeof title === 'string' && title.trim() !== '') return `"${title.trim().slice(0, 40)}"`;
    } catch {
        // Not JSON: the payload has no title to name.
    }
    return null;
}

/** What a dead letter is called in a sentence: its payload's title when it has one, else "1 dead letter". @param {any} d */
export const deadLabel = (d) => payloadTitle(d?.payload) ?? '1 dead letter';
