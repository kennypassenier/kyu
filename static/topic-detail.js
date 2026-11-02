// kyu: topic-detail.js, the detail pane of the recent messages: facts, payload block, Open large and Replay.

import { codeExports, undoToastExports, overlaysExports } from '/static/kp/dist/kp-themes.min.js';
import { $, make, dmy, bytesText } from './topic-util.js';
import { S, MESSAGES, T, call, sentence } from './topic-state.js';
import { payloadTitle } from './topic-dead-core.js';
import { openPayload } from './topic-viewer.js';
import { reload } from './topic-refresh.js';
const { setCode } = codeExports;
const { undoToast } = undoToastExports;
const { toast } = overlaysExports;

export const recent = /** @type {HTMLElement} */ ($('[data-kt-recent]'));
export const mdRoot = /** @type {HTMLElement} */ ($('[data-kt-md]', recent));
/** Lines of the payload shown in the pane before the block offers more. */
const PAYLOAD_PREVIEW_LINES = 12;
const detailBody = /** @type {HTMLElement} */ ($('[data-kt-md-body]', recent));
/** The payload block of the open message and what it was drawn with, so a read can put it in loading, failed and back. @type {{ fig: HTMLElement, spec: any } | null} */
let detailCode = null;
/** @param {'loading' | 'error' | 'ready'} state @param {string} [message] */
export function setDetailCode(state, message) {
    if (!detailCode) return;
    const { fig, spec } = detailCode;
    if (state === 'ready') setCode(fig, spec);
    else setCode(fig, { ...spec, state, message });
}
/** The message the pane shows, as its id and data; the pane is drawn again only when this changes. */
let filledSig = '';
/** Empty the pane when no message is left, so the old one and its Replay button do not stay. */
export function clearDetail() {
    filledSig = '';
    detailCode = null;
    const heading = $('[data-kp-pane-heading]', mdRoot);
    if (heading) heading.textContent = '';
    detailBody.replaceChildren();
}

/** The facts list of a message. @param {any} m */
function factsBlock(m) {
    const facts = make('div', 'kp-formgrid');
    const kv = make('dl', 'kp-kv');
    facts.append(kv);
    const rows = [
        ['Published', dmy(m.published_ms)],
        ['Content type', m.content_type ?? 'not given'],
        ['Size', bytesText(m.payload.bytes)],
        ...(m.due_ms ? [['Due', dmy(m.due_ms)]] : []),
    ];
    for (const [k, v] of rows) {
        const row = make('div', 'kp-kv__row');
        row.append(make('dt', '', k), make('dd', '', v));
        kv.append(row);
    }
    return facts;
}

/** How long Replay waits, so it can be taken back, before the message goes out. */
const REPLAY_UNDO_MS = 6000;

/** Messages whose Replay waits in an Undo window, so one message is never replayed twice at once. @type {Set<string>} */
const replaying = new Set();

/** Replay with an Undo window, like delete: nothing is sent until it closes. @param {any} m @param {HTMLButtonElement} button */
function replayWithUndo(m, button) {
    if (replaying.has(m.id)) return;
    replaying.add(m.id);
    button.disabled = true;
    // The pane may have been redrawn meanwhile: free whichever Replay button of this message is on screen now.
    const settle = () => {
        replaying.delete(m.id);
        for (const b of detailBody.querySelectorAll('[data-kt-replay]')) /** @type {HTMLButtonElement} */ (b).disabled = false;
    };
    const title = payloadTitle(m.payload) ?? 'this message';
    undoToast(`Replaying ${title} to every subscription`, {
        ms: REPLAY_UNDO_MS,
        clock: 'ring',
        mode: 'defer',
        max: 3,
        commit: async () => {
            /** @type {Awaited<ReturnType<typeof call>>} */
            let r;
            try {
                r = await call(`${T}/messages/${encodeURIComponent(m.id)}/replay`);
            } finally {
                settle();
            }
            if (!r.ok) {
                throw new Error(
                    /** @type {any} */ (r).missing
                        ? 'Replay is not available on this hub yet. Update kyu, or publish the payload again from the box below.'
                        : sentence(/** @type {any} */ (r)),
                );
            }
            toast(`Replayed ${title}. Every active subscription will receive it.`, { ms: 4000 });
            void reload();
        },
        undo: settle,
        restore: settle,
    });
}

/** The Replay button: a quiet one, since it sends to every subscription at once. @param {any} m */
function replayButton(m) {
    const replay = make('button', 'kp-button', 'Replay this message');
    replay.type = 'button';
    replay.setAttribute('data-kt-replay', '');
    replay.disabled = replaying.has(m.id);
    replay.addEventListener('click', () => replayWithUndo(m, replay));
    return replay;
}

/** The Open large and Replay buttons of a message. @param {any} m */
function detailActions(m) {
    const actions = make('div', 'kt-detail-actions');
    const open = make('button', 'kp-button', 'Open large');
    open.type = 'button';
    open.addEventListener('click', () => openPayload(m, open));
    actions.append(open, replayButton(m));
    return actions;
}

/** What the payload block is drawn with. @param {any} m */
function payloadSpec(m) {
    return {
        text: m.payload.text,
        title: 'Payload',
        language: 'JSON',
        lines: true,
        maxLines: PAYLOAD_PREVIEW_LINES,
        size: bytesText(m.payload.bytes),
        state: m.payload.binary ? 'binary' : m.payload.text === '' ? 'empty' : 'ready',
    };
}

/** Draw the detail pane for a message id (the first message when the id is unknown). @param {string} id */
export function fillDetail(id) {
    const m = MESSAGES.get(id) ?? [...MESSAGES.values()][0];
    // Stable fields only: the views' "3 minutes ago" texts change every read and must not redraw the pane.
    const sig = m ? JSON.stringify([m.id, m.content_type, m.payload, m.note, m.published_ms, m.due_ms]) : '';
    if (!m || filledSig === sig) return;
    filledSig = sig;
    const heading = $('[data-kp-pane-heading]', mdRoot);
    if (heading) heading.textContent = `Message ${m.id}`;
    detailBody.replaceChildren();
    const fig = make('figure', 'kp-code');
    fig.setAttribute('data-kp-code', '');
    const actions = detailActions(m);
    detailBody.append(factsBlock(m), fig, actions);
    const spec = payloadSpec(m);
    detailCode = { fig, spec };
    setCode(fig, spec);
    if (S.booted) S.attachAll?.(actions);
}

/** The payload block of the open message, or undefined. */
export const detailFig = () => detailCode?.fig;
