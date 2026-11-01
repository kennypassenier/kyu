// kyu: subscription-test.js, the requests panel's test button: it publishes one known message to this
// subscription's topic, so its next delivery attempt shows up here.

import { overlaysExports } from '/static/kp/dist/kp-themes.min.js';
import { $, getJson } from './subscription-util.js';
import { BASE, DATA } from './subscription-state.js';
import { readPage } from './subscription-page.js';
const { toast } = overlaysExports;

/** The message the button puts on the topic. */
const TEST_PAYLOAD = JSON.stringify({ test: 'from the dashboard' });

/** Publish the test message, say what happened, and read the page again. */
async function publishTest() {
    try {
        await getJson(`${BASE}/publish`, {
            method: 'POST',
            headers: { Accept: 'application/json', 'Content-Type': 'application/x-www-form-urlencoded' },
            body: new URLSearchParams({ payload: TEST_PAYLOAD }).toString(),
        });
        toast(`Test message published to ${DATA.topic}. It reaches ${DATA.subscription} at its next poll.`);
        await readPage();
    } catch (error) {
        toast(`Test message not published to ${DATA.topic}. ${error instanceof Error ? error.message : ''}`.trim(), {
            sticky: true,
            live: 'assertive',
        });
    }
}

/** Wire the requests panel's test button (it fires `kp-requests-test`). */
export function initTestButton() {
    $('#ky-requests')?.addEventListener('kp-requests-test', () => void publishTest());
}
