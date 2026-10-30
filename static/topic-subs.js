// kyu: topic-subs.js, section 4 of the Topic page: the subscriptions table, its filter, and archive/delete with Undo.

import { setCardState } from '/static/kp/js/card-state.js';
import { undoToast } from '/static/kp/js/undo-toast.js';
import { toast } from '/static/kp/js/overlays.js';
import { setSegmentedCounts, setSegmentedValues } from '/static/kp/js/segmented.js';
import { $, $$, make, plural } from './topic-util.js';
import { S, SUBS, T, TOPIC, call, sentence, holdPending } from './topic-state.js';
import { reconcile } from './topic-reconcile.js';
import { subRow, paintRow } from './topic-subs-rows.js';
import { reload } from './topic-refresh.js';

export const subsCard = /** @type {HTMLElement} */ ($('[data-kt-subs]'));
export const subsBody = /** @type {HTMLElement} */ ($('[data-kt-subs-rows]', subsCard));
const filterGroup = /** @type {HTMLElement} */ ($('[data-kt-filter]', subsCard));
export const subsTableEl = /** @type {HTMLElement} */ ($('[data-kt-subs-table]', subsCard));
subsBody.setAttribute('data-kp-idmark-set', '');

export function updateCounts() {
    const list = [...SUBS.values()];
    const by = (/** @type {string} */ state) => list.filter((s) => s.state === state).length;
    setSegmentedCounts(filterGroup, { all: list.length, active: by('active'), flagged: by('flagged'), archived: by('archived') });
    const total = $('[data-kt-sub-total]', subsCard);
    if (total) total.textContent = plural(list.length, 'subscription', 'subscriptions');
}

/** Subscriptions table and filter, from S.data. */
export function renderSubs() {
    SUBS.clear();
    for (const s of S.data.subscriptions) SUBS.set(s.name, { ...s });
    const added = reconcile(subsBody, [...SUBS.values()], (s) => s.name, subRow);
    fixTooltips(added);
    updateCounts();
    if (SUBS.size === 0)
        setCardState(subsCard, 'empty', { title: 'Nothing has polled this topic yet', text: `A subscription shows up here the first time something polls ${TOPIC}.` });
    else setCardState(subsCard, 'ready');
    return added;
}

/** The tooltip wrapper of a button that can be off stands in the row's button box: it carries the button's role. @param {HTMLElement[]} rows */
export function fixTooltips(rows, attach = true) {
    if (!S.booted) return;
    for (const row of rows) {
        if (attach) S.attachAll?.(row);
        const name = row.querySelector('.kt-name');
        if (name && !name.querySelector('.kp-idmark') && row.dataset.ktSub) {
            const markEl = make('span', 'kp-idmark kp-idmark--sm');
            markEl.setAttribute('data-kp-idmark', row.dataset.ktSub);
            markEl.setAttribute('aria-hidden', 'true');
            name.prepend(markEl);
        }
        for (const anchor of $$('.kp-row-actions > .kp-tooltip-anchor', row)) {
            const role = anchor.querySelector('[data-kp-action]')?.getAttribute('data-kp-action');
            if (role) anchor.setAttribute('data-kp-action', role);
        }
    }
}

filterGroup.addEventListener('kp-segmented-change', (event) => {
    const { values } = /** @type {CustomEvent} */ (event).detail;
    S.subsTable?.filter(1, values.length ? values : null);
});

// The table's own "Clear the filter" empties its filters; the state chips
// follow it back to All, without firing their change event a second time.
filterGroup
    .closest('section')
    ?.querySelector('[data-kp-datatable-clear]')
    ?.addEventListener('click', () => setSegmentedValues(filterGroup, []));

/** @param {any} s @param {string} state */
function setSubState(s, state) {
    s.state = state;
    const tr = subsBody.querySelector(`tr[data-kt-sub="${CSS.escape(s.name)}"]`);
    if (tr instanceof HTMLElement) paintRow(tr, s);
    updateCounts();
    S.subsTable?.refresh();
}

/** Archive with Undo: the page changes at once, the hub hears it when the window closes. @param {any} s */
function archiveWithUndo(s) {
    const before = s.state;
    setSubState(s, 'archived');
    const settle = holdPending();
    undoToast(`${s.name} archived`, {
        ms: 6000,
        clock: 'ring',
        mode: 'defer',
        max: 3,
        commit: async () => {
            const r = await call(`${T}/subs/${encodeURIComponent(s.name)}/archive`);
            settle();
            if (!r.ok) throw new Error(sentence(/** @type {any} */ (r)));
            void reload();
        },
        undo: () => {
            settle();
            setSubState(s, before);
        },
        restore: () => {
            settle();
            setSubState(s, before);
        },
    });
}

/** @param {any} s */
async function deleteSub(s) {
    const r = await call(`${T}/subs/${encodeURIComponent(s.name)}/delete`);
    if (!r.ok) {
        toast(`${s.name} was not deleted: ${sentence(/** @type {any} */ (r))}`, { live: 'assertive', sticky: true });
        return;
    }
    subsBody.querySelector(`tr[data-kt-sub="${CSS.escape(s.name)}"]`)?.remove();
    SUBS.delete(s.name);
    updateCounts();
    S.subsTable?.refresh();
    if (SUBS.size === 0) setCardState(subsCard, 'empty', { title: 'Nothing has polled this topic yet', text: `A subscription shows up here the first time something polls ${TOPIC}.` });
    toast(`${s.name} deleted`, { ms: 4000 });
    void reload();
}

subsCard.addEventListener('click', (event) => {
    const target = event.target instanceof Element ? event.target : null;
    const tr = target?.closest('tr[data-kt-sub]');
    const s = SUBS.get(tr?.getAttribute('data-kt-sub') ?? '');
    if (!target || !tr || !s) return;
    if (target.closest('[data-kt-archive]')) archiveWithUndo(s);
    else if (target.closest('[data-kt-delete]')) void deleteSub(s);
});
