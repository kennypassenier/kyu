// kyu: common-palette.js, the command palette (Ctrl+K): Publish and Prune run here, and the confirmed prune of dead letters.

import { palette, overlays, components } from './common-kit.js';
import { ON_TOPIC_PAGE, CONTEXT_TOPIC, one, counted } from './common-context.js';
import { dialog, sheet, prune } from './common-shell.js';

/** The publish box of the topic page, wherever the page builder put it. */
export const publishField = () =>
    one('form[action$="/dashboard/publish"] [name="payload"]') ?? one('[data-kyu-publish]') ?? null;

if (dialog) {
    if (CONTEXT_TOPIC) dialog.setAttribute('data-kp-context', CONTEXT_TOPIC);

    const publish = one('[data-value="publish"]', dialog);
    const publishWords = one('.kp-palette__description', publish ?? dialog);
    if (publish && publishWords) {
        publishWords.textContent = ON_TOPIC_PAGE ? (publish.dataset.hintTopic ?? '') : (publish.dataset.hintElsewhere ?? '');
    }

    const dead = Number(one('[data-value="prune"]', dialog)?.dataset.dead ?? 0);
    const pruneWords = one('[data-value="prune"] .kp-palette__description', dialog);
    if (pruneWords) {
        pruneWords.textContent = dead > 0
            ? `Delete the ${counted(dead, 'dead letter')} on this hub, after you type prune`
            : 'Nothing is dead, so there is nothing to prune';
    }

    palette.attachPalettes(document.body);
    if (sheet) sheet.setAttribute('data-kp-primary', '');

    dialog.addEventListener('kp-palette-run', (event) => {
        const value = String(/** @type {CustomEvent} */ (event).detail?.value ?? '');
        // After the palette has closed and handed the focus back to where it came from.
        const after = (act) => {
            if (!dialog.open) return act();
            dialog.addEventListener('close', () => setTimeout(act, 0), { once: true });
        };
        if (value === 'publish') after(() => (publishField() ? goPublish() : location.assign('/topics')));
        else if (value === 'prune') after(() => askPrune(dead));
    });
}

export function goPublish() {
    const field = publishField();
    if (!field) return false;
    field.scrollIntoView({ block: 'center' });
    field.focus();
    return true;
}

function askPrune(dead) {
    if (dead === 0) {
        overlays.toast('Nothing is dead on this hub.');
        return;
    }
    prune?.click();
}

if (prune) {
    components.attachConfirmations(document.body);
    prune.addEventListener('click', async () => {
        try {
            const response = await fetch('/dashboard/dead-letters/prune', { method: 'POST', headers: { accept: 'application/json' } });
            if (!response.ok) throw new Error(String(response.status));
            const { pruned } = await response.json();
            overlays.toast(`Pruned ${counted(Number(pruned), 'dead letter')}.`);
            setTimeout(() => location.reload(), 900);
        } catch {
            overlays.toast('The dead letters could not be pruned. Nothing was changed.', { live: 'assertive', close: true });
        }
    });
}
