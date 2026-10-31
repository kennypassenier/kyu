// kyu: topic-readers.js, section 3 of the Topic page: the warning shown while nobody has polled yet.

import { cardStateExports, problemsExports } from '/static/kp/dist/kp-themes.min.js';
import { $ } from './topic-util.js';
import { S, TOPIC, withOrigin } from './topic-state.js';
const { setCardState } = cardStateExports;
const { setProblems } = problemsExports;

const readers = $('[data-kt-readers]');
export function renderReaders() {
    if (!readers) return;
    const problems = /** @type {HTMLElement} */ ($('[data-kt-bootstrap]', readers));
    const active = S.data.subscriptions.filter((/** @type {any} */ s) => s.state !== 'archived');
    if (active.length > 0) {
        const first = active[0];
        setCardState(readers, 'empty', {
            title: `A reader has polled ${TOPIC}`,
            text: `${first.name} polled ${first.last_poll_at ?? 'recently'}, so there is nothing to warn about.`,
        });
        return;
    }
    const command = withOrigin(S.data.commands.shown.receive_raw).replace('?as=my-consumer"', '?as=my-consumer&from=beginning"');
    setProblems(problems, [
        {
            key: 'nobody-polled',
            what: `Nobody has polled ${TOPIC} yet`,
            why: 'A subscription only starts existing when it first polls, and it receives only what is published after that.',
            remedy: 'Run the receive command once before you publish the message you want it to see, or add &from=beginning to pick up what the topic still retains.',
            command,
            severity: 'warning',
        },
    ]);
    setCardState(readers, 'ready');
}
