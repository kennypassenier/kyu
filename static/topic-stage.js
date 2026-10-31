// kyu: topic-stage.js, a region that keeps its footprint in every state (busy card, plate, ready).

import { busyCardExports, artExports } from '/static/kp/dist/kp-themes.min.js';
import { $ } from './topic-util.js';
const { openBusyCard } = busyCardExports;
const { attachArt } = artExports;

/** @param {HTMLElement} host a `[data-kt-stage]` */
export function stage(host) {
    const plate = /** @type {HTMLElement} */ ($('[data-kt-plate]', host));
    /** @type {import('/static/kp/js/busy-card.js').BusyCard | null} */
    let card = null;
    const clear = () => {
        card?.remove();
        card = null;
        plate.hidden = true;
        plate.replaceChildren();
        plate.removeAttribute('data-kp-art');
        host.removeAttribute('data-kt-state');
    };
    return {
        ready: clear,
        /** @param {string} words */
        loading(words) {
            clear();
            host.setAttribute('data-kt-state', 'loading');
            card = openBusyCard(host, { words, area: () => [host], arrive: false });
        },
        /** @param {HTMLElement} node @param {string} [art] */
        plate(node, art) {
            clear();
            host.setAttribute('data-kt-state', 'plate');
            if (art) {
                node.setAttribute('data-kp-art', art);
                attachArt(node);
            }
            plate.append(node);
            plate.hidden = false;
        },
    };
}
