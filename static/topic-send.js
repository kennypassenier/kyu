// kyu: topic-send.js, section 2 of the Topic page: the send-and-receive commands, and the secret fields.

import { $, $$, plural } from './topic-util.js';
import { S, page, withOrigin } from './topic-state.js';

// Two copy buttons stand in every step: the command block's own "Copy" (without the token) and the secret field's,
// which puts the token in. The field's button is named for what it does.
const COPY_WITH_TOKEN = 'Copy with token';

const send = /** @type {HTMLElement} */ ($('[data-kt-send]'));
const ttl = Number(page.dataset.ktRevealSeconds ?? 10);
/** Name the field's button for what it does; kp-themes 12.2.0 puts this label back after each "Copied". @param {Element | null} copy */
function nameCopyButton(copy) {
    if (copy?.textContent === 'Copy') copy.textContent = COPY_WITH_TOKEN;
}

export function renderCommands() {
    for (const block of $$('[data-kt-command]', send)) {
        const code = $('code', block);
        if (code) code.textContent = withOrigin(S.data.commands.shown[block.dataset.ktCommand ?? ''] ?? '');
    }
    for (const field of $$('[data-kt-secret]', send)) {
        field.setAttribute('data-kp-secret-ttl', String(ttl));
        nameCopyButton($('[data-kp-secret-copy]', field));
        // Without a login token on this hub there is nothing to reveal or copy.
        if (S.data.commands.has_token) field.removeAttribute('data-kp-secret-disabled');
        else field.setAttribute('data-kp-secret-disabled', 'This hub has no login token to put in the command.');
    }
}
// The token is read from the page's data only when a field is asked to show or copy it.
// Reveal shows the token alone (the command is already on the page above, masked); Copy with token copies the whole
// working command. Copy of a field that is shown would copy what is shown, so it hides the field first.
send.addEventListener('kp-secret-request', (event) => {
    const { respond, purpose } = /** @type {CustomEvent} */ (event).detail;
    const field = /** @type {HTMLElement} */ (event.target);
    event.preventDefault();
    if (purpose === 'reveal') return respond(S.data.commands.token ?? '');
    respond(withOrigin(S.data.commands.working[field.dataset.ktSecret ?? ''] ?? ''));
});
send.addEventListener(
    'click',
    (event) => {
        const target = event.target instanceof Element ? event.target : null;
        const field = target?.closest('[data-kp-secret]');
        if (!field || field.getAttribute('data-kp-secret-state') !== 'shown') return;
        if (!target?.closest('[data-kp-secret-copy]')) return;
        $('[data-kp-secret-reveal]', /** @type {HTMLElement} */ (field))?.click();
    },
    true,
);
{
    const words = $('[data-kt-ttl-words]', send);
    if (words) words.textContent = ttl === 0 ? 'until you press Hide' : `for ${plural(ttl, 'second', 'seconds')}`;
}
