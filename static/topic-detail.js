// kyu: topic-detail.js, the detail pane of the recent messages: facts, payload block, Open large and Replay.

import { setCode } from '/static/kp/js/code.js';
import { runFlashButton } from '/static/kp/js/flash.js';
import { $, make, dmy, bytesText } from './topic-util.js';
import { S, MESSAGES, T, call, refusal } from './topic-state.js';
import { openPayload } from './topic-viewer.js';
import { reload } from './topic-refresh.js';

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
let filledId = '';
/** Forget which message the pane shows, so the next fillDetail draws it again. */
export function resetDetail() {
    filledId = '';
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

/** The Replay button: publishes the same payload again. @param {any} m */
function replayButton(m) {
    const replay = make('button', 'kp-button kp-button--primary', 'Replay this message');
    replay.type = 'button';
    replay.setAttribute('data-kp-flash-button', '');
    replay.setAttribute('data-kp-flash-words', 'Replayed,Refused,Failed');
    replay.setAttribute('data-kp-flash-word', 'Replayed');
    replay.setAttribute('data-kp-refusal', 'toast');
    replay.addEventListener('click', () =>
        runFlashButton(replay, async () => {
            const r = await call(`${T}/messages/${encodeURIComponent(m.id)}/replay`);
            if (!r.ok) {
                return /** @type {any} */ (r).missing
                    ? { error: 'Replay is not available on this hub yet.', remedy: 'Update kyu, or publish the payload again from the box below.' }
                    : refusal(/** @type {any} */ (r));
            }
            void reload();
            return { word: 'Replayed', line: `Published again as ${r.json?.id ?? 'a new message'}. Every active subscription will receive it.` };
        }),
    );
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
    if (!m || filledId === m.id) return;
    filledId = m.id;
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
