// kyu: topic-dead-state.js, keeps the page's own data in step with a dead-letter action the moment it is taken.
//
// A delete waits out its Undo window and a requeue waits for its flash, so the page data (S.data) still lists the
// letter while the table row is already gone. The alert ("N messages died", "Requeue N"), the header badge and the
// Dead column all draw from S.data, so they are updated here, from the same list the table was built from, and put
// back on Undo. Nothing here talks to the hub: the next read replaces S.data with the truth.

import { S } from './topic-state.js';
import { renderHeader } from './topic-header.js';
import { renderSubs } from './topic-subs.js';
import { renderDead } from './topic-dead.js';
import { deadKey } from './topic-dead-core.js';

/** @typedef {{ letter: any, index: number }} Taken */

/** Redraw everything that counts dead letters from S.data. */
function repaint() {
    renderHeader();
    renderSubs();
    S.subsTable?.refresh();
    // The table follows the same list (rows that left are dropped), then draws the alert and the note under it.
    renderDead();
}

/** Add `by` to a counter the page data keeps for a subscription and for the topic. @param {string} subscription @param {number} by */
function bumpCounts(subscription, by) {
    S.data.topic.dead = Math.max(0, S.data.topic.dead + by);
    const sub = S.data.subscriptions.find((/** @type {any} */ s) => s.name === subscription);
    if (sub) sub.dead = Math.max(0, sub.dead + by);
}

/**
 * Take dead letters out of the page data. Returns what was taken, in the order to give back.
 * @param {{ subscription: string, id: string }[]} items
 * @returns {Taken[]}
 */
export function takeDead(items) {
    const wanted = new Set(items.map((x) => deadKey(x)));
    /** @type {Taken[]} */
    const taken = [];
    S.data.dead_letters.forEach((/** @type {any} */ letter, /** @type {number} */ index) => {
        if (wanted.has(deadKey(letter))) taken.push({ letter, index });
    });
    const gone = new Set(taken.map((t) => t.letter));
    S.data.dead_letters = S.data.dead_letters.filter((/** @type {any} */ l) => !gone.has(l));
    for (const { letter } of taken) bumpCounts(letter.subscription, -1);
    repaint();
    return taken;
}

/** Put letters taken earlier back where they were. @param {Taken[]} taken */
export function giveBackDead(taken) {
    for (const { letter, index } of [...taken].sort((a, b) => a.index - b.index)) {
        if (S.data.dead_letters.some((/** @type {any} */ l) => deadKey(l) === deadKey(letter))) continue;
        S.data.dead_letters.splice(Math.min(index, S.data.dead_letters.length), 0, letter);
        bumpCounts(letter.subscription, 1);
    }
    repaint();
}
