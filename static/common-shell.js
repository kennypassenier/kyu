// kyu: common-shell.js, moves the server's <template id="kyu-shell"> into the body and finds its dialogs.

import { ON_TOPICS_PAGE } from './common-context.js';
import { mountUses } from './common-uses.js';

const shell = /** @type {HTMLTemplateElement | null} */ (document.getElementById('kyu-shell'));
if (shell) {
    const fragment = /** @type {DocumentFragment} */ (shell.content.cloneNode(true));
    const uses = fragment.getElementById('kyu-uses');
    uses?.remove();
    document.body.append(fragment);
    if (uses && ON_TOPICS_PAGE) mountUses(uses);
}

export const dialog = /** @type {HTMLDialogElement | null} */ (document.getElementById('kyu-palette'));
export const sheet = /** @type {HTMLDialogElement | null} */ (document.getElementById('kyu-sheet'));
export const help = /** @type {HTMLDialogElement | null} */ (document.getElementById('kyu-help'));
export const prune = /** @type {HTMLButtonElement | null} */ (document.getElementById('kyu-prune'));
