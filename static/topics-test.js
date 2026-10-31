// kyu: topics-test.js, Send test: publishes a known message to a topic from its row.

import { overlaysExports } from '/static/kp/dist/kp-themes.min.js';
const { toast } = overlaysExports;

/** @param {HTMLButtonElement} button */
export async function sendTest(button) {
    const name = button.getAttribute('data-tp-test') ?? '';
    if (button.disabled) return;
    button.disabled = true;
    try {
        const response = await fetch(`/t/${encodeURIComponent(name)}/dashboard/publish`, {
            method: 'POST',
            headers: { 'content-type': 'application/x-www-form-urlencoded', accept: 'application/json' },
            body: `payload=${encodeURIComponent(JSON.stringify({ test: 'hello from the kyu dashboard' }))}`,
        });
        if (!response.ok) {
            let words = `The hub refused it (${response.status}).`;
            try {
                const body = await response.json();
                if (body.error) words = `${body.error}${body.remedy ? ` ${body.remedy}` : ''}`;
            } catch {
                /* the answer was not JSON; the status says enough */
            }
            toast(`Test message not published to ${name}. ${words}`, { sticky: true, live: 'assertive' });
            return;
        }
        toast(`Test message published to ${name}. The figures pick it up at the next refresh.`);
    } catch {
        toast(`Test message not published to ${name}. The hub did not answer.`, { sticky: true, live: 'assertive' });
    } finally {
        button.disabled = false;
    }
}
