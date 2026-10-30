// kyu: topic-dead.js, section 5 of the Topic page: the dead-letters table (rows, payload cells, the "newest N" note).

import { $, make, esc, num, plural, iso, dmy, span } from './topic-util.js';
import { S, SUBS } from './topic-state.js';
import { reconcile } from './topic-reconcile.js';
import { deadKey } from './topic-dead-core.js';
import { renderProblems } from './topic-problems.js';

const deadSection = /** @type {HTMLElement} */ ($('[data-kt-dead]'));
export const deadBody = /** @type {HTMLElement} */ ($('[data-kt-dead-rows]', deadSection));
export const deadTableEl = /** @type {HTMLElement} */ ($('[data-kt-dead-table]', deadSection));

/** Why a message died, in the words kyu has. @param {any} d */
function whyDied(d) {
    const s = SUBS.get(d.subscription);
    const lease = s ? ` (lease ${span(s.lease_ms)})` : '';
    return `${d.subscription} did not acknowledge it in ${plural(d.attempts, 'attempt', 'attempts')}${lease}`;
}
/** A payload as a table cell. @param {any} p */
function cellFigure(p) {
    const fig = make('figure', 'kp-code kp-code--cell');
    fig.setAttribute('data-kp-code', '');
    fig.setAttribute('data-kp-code-language', 'auto');
    fig.setAttribute('data-kp-code-open', 'inline');
    if (p.binary) fig.setAttribute('data-kp-binary-size', String(p.bytes));
    else if (p.truncated) fig.setAttribute('data-kp-truncated', `${new Blob([p.text]).size}/${p.bytes}`);
    const pre = make('pre');
    pre.append(make('code', '', p.binary ? '' : p.text));
    fig.append(pre);
    return fig;
}

/** @param {any} d */
function deadRow(d) {
    const tr = make('tr');
    const key = deadKey(d);
    tr.dataset.kpKey = key;
    tr.setAttribute('data-kp-row-key', key);
    tr.setAttribute('data-kp-anchor', key);
    tr.setAttribute('data-kp-row-label', `Dead letter ${d.id}`);
    tr.dataset.ktSub = d.subscription;
    tr.dataset.ktId = d.id;
    const when = d.dead_ms ?? d.published_ms;
    tr.innerHTML =
        `<td data-label="Select"><input class="kp-field__check" type="checkbox" data-kp-select-row aria-label="Select dead letter ${esc(d.id)}" /></td>` +
        `<td data-label="Died" data-kp-sort-value="${iso(when)}"><div class="kt-when"><time class="kp-ago" data-kp-ago data-kp-ago-verb="died" datetime="${iso(when)}">${d.dead_ms ? esc(d.dead_at) : 'unknown'}</time><small>${d.dead_ms ? dmy(d.dead_ms) : `published ${dmy(d.published_ms)}`}</small></div></td>` +
        `<td data-label="Subscription" data-kp-filter-value="${esc(d.subscription)}"><b>${esc(d.subscription)}</b></td>` +
        `<td data-label="Tries" class="kp-text-end">${num(d.attempts)}</td>` +
        `<td data-label="Why it died">${esc(whyDied(d))}</td>` +
        `<td data-label="Payload"></td>` +
        `<td data-label="Actions"><div class="kp-row-actions"><button type="button" class="kp-button kp-button--sm" data-kp-action="requeue" data-kt-row="requeue" data-kp-flash-words="Requeued,Refused,Failed" data-kp-refusal="toast">Requeue</button><button type="button" class="kp-button kp-button--sm kp-button--destructive" data-kp-action="delete" data-kt-row="delete">Delete</button></div></td>`;
    /** @type {HTMLElement} */ ($('td:nth-child(6)', tr)).append(cellFigure(d.payload));
    return tr;
}

const deadNote = /** @type {HTMLElement} */ ($('[data-kt-dead-note]', deadSection));
export function renderDead() {
    const rows = S.data.dead_letters;
    const added = reconcile(deadBody, rows, deadKey, deadRow);
    if (S.booted) {
        for (const row of added) S.attachAll?.(row);
        S.deadTable?.refresh();
    }
    const total = S.data.topic.dead;
    deadNote.textContent = total > rows.length ? `Showing the ${num(rows.length)} newest of ${num(total)} dead letters.` : '';
    renderProblems();
}
