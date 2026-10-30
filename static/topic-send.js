// kyu: topic-send.js, section 2 of the Topic page: the send-and-receive commands, the secret fields and the journey steps.

import { setStep } from '/static/kp/js/steplist.js';
import { $, $$, dmy } from './topic-util.js';
import { S, page, withOrigin } from './topic-state.js';

const send = /** @type {HTMLElement} */ ($('[data-kt-send]'));
const ttl = Number(page.dataset.ktRevealSeconds ?? 10);
export function renderCommands() {
    for (const block of $$('[data-kt-command]', send)) {
        const code = $('code', block);
        if (code) code.textContent = withOrigin(S.data.commands.shown[block.dataset.ktCommand ?? ''] ?? '');
    }
    for (const field of $$('[data-kt-secret]', send)) {
        field.setAttribute('data-kp-secret-ttl', String(ttl));
        // Without a login token on this hub there is nothing to reveal or copy.
        if (S.data.commands.has_token) field.removeAttribute('data-kp-secret-disabled');
        else field.setAttribute('data-kp-secret-disabled', 'This hub has no login token to put in the command.');
    }
}
// The token is read from the page's data only when a field is asked to show or copy it.
send.addEventListener('kp-secret-request', (event) => {
    const { respond } = /** @type {CustomEvent} */ (event).detail;
    const field = /** @type {HTMLElement} */ (event.target);
    event.preventDefault();
    respond(withOrigin(S.data.commands.working[field.dataset.ktSecret ?? ''] ?? ''));
});
const journey = /** @type {HTMLElement} */ ($('[data-kt-journey]', send));
const STEPS = ['publish', 'receive', 'ack'];
export const startOver = () => STEPS.forEach((id, i) => setStep(journey, id, { state: i === 0 ? 'current' : 'S.pending', time: '' }));
send.addEventListener('kp-copy', (event) => {
    const step = /** @type {HTMLElement} */ (event.target).closest('[data-kt-step]')?.getAttribute('data-kt-step');
    if (!step) return;
    setStep(journey, step, { state: 'done', time: dmy(Date.now()) });
    const next = STEPS.find((id) => $(`[data-kp-step-id="${id}"]`, journey)?.getAttribute('data-state') !== 'done');
    if (next) setStep(journey, next, { state: 'current' });
});
$('[data-kt-start-over]', send)?.addEventListener('click', startOver);
{
    const words = $('[data-kt-ttl-words]', send);
    if (words) words.textContent = ttl === 0 ? 'until you press Hide' : `for ${ttl} seconds`;
}
