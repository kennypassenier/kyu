// kyu: topic-problems.js, the problems of the dead letters: what died, why, and what now, plus the actions on those cards.

import { setProblems } from '/static/kp/js/problems.js';
import { runFlashButton } from '/static/kp/js/flash.js';
import { $, make, num, plural, dmy } from './topic-util.js';
import { S, SUBS, ORIGIN, TOPIC } from './topic-state.js';
import { stage } from './topic-stage.js';
import { deadKey, deadBySub, bulk } from './topic-dead-core.js';
import { reload } from './topic-refresh.js';

const deadSection = /** @type {HTMLElement} */ ($('[data-kt-dead]'));
const problemsHost = /** @type {HTMLElement} */ ($('[data-kt-stage="problems"]', deadSection));
const problemsView = stage(problemsHost);
const problemsList = /** @type {HTMLElement} */ ($('[data-kt-problems]', problemsHost));

const DAY_MS = 86_400_000;

/** The problems of the dead letters: what died, why, and what now. */
export async function renderProblems() {
    /** @type {any[]} */
    const items = [...deadBySub()].map(([name, list]) => subscriptionProblem(name, list));
    // The oldest dead letter against how long dead letters are kept.
    const oldest = oldestProblem();
    if (oldest) items.push(oldest);
    if (items.length === 0) {
        setProblems(problemsList, []);
        problemsHost.hidden = true;
        problemsView.ready();
        return;
    }
    problemsHost.hidden = false;
    problemsView.ready();
    setProblems(problemsList, items);
}

/** One problem card: the dead letters of one subscription, with its Requeue button. @param {string} name @param {any[]} list */
function subscriptionProblem(name, list) {
    const s = SUBS.get(name);
    const total = s ? Math.max(s.dead, list.length) : list.length;
    const archived = s?.state === 'archived';
    const last = Math.max(...list.map((d) => d.dead_ms ?? 0));
    const button = make('button', 'kp-button', `Requeue ${num(list.length)}`);
    button.type = 'button';
    button.setAttribute('data-kt-fix', name);
    button.setAttribute('data-kp-flash-words', 'Requeued,Refused,Failed');
    button.setAttribute('data-kp-refusal', 'toast');
    return {
        key: name,
        severity: archived ? 'info' : s?.state === 'flagged' ? 'critical' : 'warning',
        what: `${plural(total, 'message', 'messages')} died on ${name}`,
        why: `${name} did not acknowledge ${total === 1 ? 'it' : 'them'} in ${s ? plural(s.max_attempts, 'attempt', 'attempts') : 'the allowed number of attempts'}.${last ? ` The last one died on ${dmy(last)}.` : ''}`,
        remedy: archived
            ? `${name} is archived and no longer delivers. Delete these dead letters, or bring it back and requeue them:`
            : `Fix whatever reads ${name}, then requeue them.`,
        command: archived
            ? `curl -s -X POST -H "authorization: Bearer $KYU_TOKEN" "${ORIGIN}/api/t/${TOPIC}/subs/${name}/unarchive"`
            : undefined,
        action: button,
    };
}

/** The "oldest dead letter" card, once the retention is known and the letter is a day old; null otherwise. */
function oldestProblem() {
    const keptDays = S.retentionDays;
    if (!keptDays || !S.data.dead_letters.length) return null;
    const oldest = S.data.dead_letters.reduce((/** @type {any} */ a, /** @type {any} */ b) => ((a.dead_ms ?? a.published_ms) < (b.dead_ms ?? b.published_ms) ? a : b));
    const at = oldest.dead_ms ?? oldest.published_ms;
    const days = Math.floor((S.data.now_ms - at) / DAY_MS);
    if (days < 1) return null;
    const button = make('button', 'kp-button', 'Show it');
    button.type = 'button';
    button.setAttribute('data-kt-oldest', deadKey(oldest));
    return {
        key: 'oldest',
        severity: 'info',
        what: `The oldest dead letter is ${plural(days, 'day', 'days')} old`,
        why: `Dead letters are kept for ${plural(keptDays, 'day', 'days')} and then pruned.`,
        remedy: `Requeue or delete it before ${dmy(at + keptDays * DAY_MS).slice(0, 10)}.`,
        action: button,
    };
}

problemsList.addEventListener('click', async (event) => {
    const target = event.target instanceof Element ? event.target : null;
    const oldest = target?.closest('[data-kt-oldest]');
    if (oldest instanceof HTMLElement) {
        const key = oldest.dataset.ktOldest ?? '';
        if (location.hash === `#${key}`) location.hash = '';
        location.hash = key;
        return;
    }
    const button = target?.closest('[data-kt-fix]');
    if (!(button instanceof HTMLElement)) return;
    const name = button.dataset.ktFix ?? '';
    const list = deadBySub().get(name) ?? [];
    await runFlashButton(button, async () => {
        const { done, refused } = await bulk('requeue', list.map((d) => ({ subscription: name, id: d.id })));
        if (refused.length) return { error: `${plural(refused.length, 'message', 'messages')} not requeued.`, remedy: refused[0].reason };
        void done;
        return 'Requeued';
    });
    void reload();
});
