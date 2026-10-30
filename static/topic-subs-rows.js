// kyu: topic-subs-rows.js, one subscription row: its markup, its policy text, its menu and its delete-confirmation template.

import { setMenu } from '/static/kp/js/menu-button.js';
import { $, esc, num, plural, dmy, iso, span } from './topic-util.js';
import { S, T } from './topic-state.js';

/** @param {any} s */
function policyHtml(s) {
    const lease = `lease ${span(s.lease_ms)}${s.explicit.lease_ms == null ? ' <i>(default)</i>' : ''}`;
    const tries = `${s.max_attempts} attempts${s.explicit.max_attempts == null ? ' <i>(default)</i>' : ''}`;
    const ttlText = s.ttl_ms ? `, ttl ${span(s.ttl_ms)}` : '';
    return `${lease}, ${tries}${ttlText}`;
}
const badge = (/** @type {any} */ s) =>
    `<span class="kp-badge${s.state === 'active' ? ' kp-badge--success' : s.state === 'flagged' ? ' kp-badge--warning' : ''}">${esc(s.state)}</span>`;

/** @param {any} s */
export function subRow(s) {
    const tr = document.createElement('tr');
    // Before the kit's boot the mark is in the markup and the boot paints it. Afterwards the kit's own document-wide
    // watcher paints a mark added to the page; attaching the row on top of it would paint the shared palette twice,
    // from two different roots, and the two would undo each other forever. So a late row gets its mark after attach.
    const mark = S.booted ? '' : `<span class="kp-idmark kp-idmark--sm" data-kp-idmark="${esc(s.name)}" aria-hidden="true"></span>`;
    const n = esc(s.name);
    const oldest = s.oldest_unacked_ms
        ? `<time datetime="${iso(s.oldest_unacked_ms)}">${dmy(s.oldest_unacked_ms)}</time>`
        : '<span class="kt-quiet">nothing waiting</span>';
    tr.dataset.kpKey = s.name;
    tr.dataset.ktSub = s.name;
    tr.innerHTML = `
        <td data-label="Name"><span class="kt-name">${mark}<a href="${T}/subs/${encodeURIComponent(s.name)}">${n}</a></span></td>
        <td data-label="State" data-kt-state-cell>${badge(s)}</td>
        <td data-label="Backlog" class="kp-text-end">${num(s.backlog)}</td>
        <td data-label="In flight" class="kp-text-end">${num(s.claimed)}</td>
        <td data-label="Dead" class="kp-text-end">${s.dead > 0 ? `<span class="kp-badge kp-badge--destructive">${num(s.dead)}</span>` : '0'}</td>
        <td data-label="Lapsed" class="kp-text-end">${num(s.lapsed)}</td>
        <td data-label="Oldest unacked" data-kp-sort-value="${s.oldest_unacked_ms ? iso(s.oldest_unacked_ms) : ''}">${oldest}</td>
        <td data-label="Policy" class="kt-policy"><span>${policyHtml(s)}</span></td>
        <td class="kt-col-actions">
            <div class="kp-row-actions">
                <button type="button" class="kp-button kp-button--sm" data-kp-action="archive" data-kt-archive data-kp-disabled-reason="${n} is already archived">Archive</button>
                <button type="button" class="kp-button kp-button--sm kp-button--destructive" data-kp-action="delete" data-kt-delete data-kp-destructive
                    data-kp-confirm="Delete subscription ${n} and its delivery history?" data-kp-confirm-type="${n}"
                    data-kp-confirm-loses="kt-loses-${n}"
                    data-kp-disabled-reason="Archive ${n} first: only an archived subscription can be deleted">Delete</button>
                <div class="kp-menu-button" data-kp-menu-button data-kp-action="more" data-kt-menu="${n}">
                    <button type="button" class="kp-button kp-button--sm" aria-label="More for ${n}">More ▾</button>
                    <div class="kp-menu kp-menu--rich" role="menu" aria-label="More for ${n}"></div>
                </div>
            </div>
        </td>`;
    paintRow(tr, s);
    return tr;
}

/** What deleting would lose, as the typed confirmation lists it. @param {any} s */
function losesTemplate(s) {
    const id = `kt-loses-${s.name}`;
    let t = /** @type {HTMLTemplateElement | null} */ (document.getElementById(id));
    if (!t) {
        t = document.createElement('template');
        t.id = id;
        document.body.append(t);
    }
    const held = s.backlog + s.claimed + s.dead;
    t.innerHTML = `<ul><li>The delivery history of <b>${esc(s.name)}</b> (${plural(held, 'message', 'messages')} still held)</li><li>Its policy: ${policyHtml(s).replace(/<\/?i>/g, '')}</li></ul>`;
}

/** @param {any} s */
function menuGroups(s) {
    const gone = s.state === 'archived';
    const away = `${s.name} is archived and no longer delivers`;
    return [
        {
            group: 'Look',
            items: [
                {
                    label: s.dead > 0 ? `Dead letters (${num(s.dead)})` : 'Dead letters',
                    hint: 'Messages that gave up after too many tries',
                    disabled: s.dead > 0 ? null : `Nothing died on ${s.name}`,
                    value: 'dead',
                },
                {
                    label: 'Oldest unacked message',
                    hint: 'The message that has waited longest',
                    disabled: s.backlog + s.claimed > 0 ? null : `Nothing is waiting on ${s.name}`,
                    value: 'oldest',
                },
            ],
        },
        {
            group: 'Use',
            items: [
                { label: 'Copy receive command', hint: 'Without the token', disabled: gone ? away : null, value: 'copy-receive' },
                { label: 'Copy acknowledge command', hint: 'Without the token', disabled: gone ? away : null, value: 'copy-ack' },
            ],
        },
    ];
}

/** Bring a row's buttons, badge and menu in step with its subscription. @param {HTMLElement} tr @param {any} s */
export function paintRow(tr, s) {
    tr.dataset.state = s.state;
    const cell = $('[data-kt-state-cell]', tr);
    if (cell) cell.innerHTML = badge(s);
    const archived = s.state === 'archived';
    const archive = /** @type {HTMLButtonElement} */ ($('[data-kt-archive]', tr));
    const del = /** @type {HTMLButtonElement} */ ($('[data-kt-delete]', tr));
    archive.disabled = archived;
    del.disabled = !archived;
    losesTemplate(s);
    const wrapper = $('[data-kt-menu]', tr);
    if (wrapper) setMenu(wrapper, menuGroups(s));
}
