// kyu: topic-publish.js, section 7 of the Topic page: the publish-a-test-message form.

import { runFlashButton } from '/static/kp/js/flash.js';
import { T, call, refusal } from './topic-state.js';
import { reload } from './topic-refresh.js';

const form = /** @type {HTMLFormElement} */ (document.getElementById('kt-publish-form'));
const area = /** @type {HTMLTextAreaElement} */ (document.getElementById('kt-publish-body'));
const delayField = /** @type {HTMLInputElement | null} */ (document.getElementById('kt-publish-delay'));
form.addEventListener('submit', (event) => event.preventDefault());
// The package's own form: data-kp-submit holds the button busy while the message goes out and `done()` hands it back;
// the same button is a flash button, so its word ("Published" / "Refused" / "Failed") and the line under it
// ("Published as <id>." / the refusal and what to do) are the package's too (flash.js).
const sendButton = /** @type {HTMLButtonElement} */ (document.getElementById('kt-publish-send'));
form.addEventListener('kp-form-valid', (event) => {
    const { done } = /** @type {CustomEvent<{ done: () => void }>} */ (event).detail;
    void (async () => {
        let published = false;
        await runFlashButton(sendButton, async () => {
            const r = await call(`${T}/publish`, { form: { payload: area.value, delay: delayField?.value ?? '' } });
            if (!r.ok) return refusal(/** @type {any} */ (r));
            published = true;
            return { word: 'Published', line: r.json?.id ? `Published as ${r.json.id}.` : 'Published.' };
        });
        done();
        if (published) void reload();
    })();
});
