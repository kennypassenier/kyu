// kyu: topic-subs-menu.js, the "More" menu of a subscription row: jump to its dead letters or oldest message, copy its commands.

import { toast } from '/static/kp/js/overlays.js';
import { S, ORIGIN, T, TOPIC } from './topic-state.js';
import { subsCard } from './topic-subs.js';

/** Copy text; say what happened in a toast. @param {string} text @param {string} said */
async function copyToast(text, said) {
    try {
        await navigator.clipboard.writeText(text);
        toast(said, { ms: 3000 });
    } catch {
        toast('Your browser blocked copying. Select the command on the page and press Ctrl+C.', { ms: 5000, live: 'assertive' });
    }
}
document.addEventListener('kp-menu-select', (event) => {
    const { value } = /** @type {CustomEvent} */ (event).detail;
    const name = /** @type {HTMLElement} */ (event.target).closest('[data-kt-menu]')?.getAttribute('data-kt-menu');
    if (!name || !subsCard.contains(/** @type {Node} */ (event.target))) return;
    const base = `curl -s -H "authorization: Bearer $KYU_TOKEN"`;
    if (value === 'dead') {
        S.deadTable?.filter(2, [name]);
        document.getElementById('kt-dead-title')?.scrollIntoView({ behavior: 'smooth', block: 'start' });
    } else if (value === 'oldest') location.href = `${T}/subs/${encodeURIComponent(name)}`;
    else if (value === 'copy-receive')
        void copyToast(`${base} -D- -o message.body "${ORIGIN}/t/${TOPIC}/next?as=${name}"`, `Copied the receive command for ${name}, without the token.`);
    else if (value === 'copy-ack')
        void copyToast(`${base} -X POST "${ORIGIN}/t/${TOPIC}/ack/<id>?as=${name}"`, `Copied the acknowledge command for ${name}, without the token.`);
});
