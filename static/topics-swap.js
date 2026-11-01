// kyu: topics-swap.js, the kit's refresh tick: it swaps the key-figure group and the page follows from the data block that carries.

import { tickExports, updateExports } from '/static/kp/dist/kp-themes.min.js';
import { $$ } from './topics-util.js';
import { M, ui, takeData, decideSlots } from './topics-state.js';
import { kpis, kpiTiles, shown, remember } from './topics-kpis.js';
import { drawBand } from './topics-attention.js';
import { drawHubBar } from './topics-paint.js';
import { table, syncRows } from './topics-table.js';
const { tickTo } = tickExports;
const { update } = updateExports;

/** New tiles start on what the old ones showed and count to the new numbers; the words change the theme's way.
 * @param {HTMLElement} group @param {Map<string | null, string>} before */
function animateTiles(group, before) {
    for (const el of $$('[data-kp-tick][data-tp-key]', group)) {
        const old = before.get(el.getAttribute('data-tp-key'));
        const next = el.textContent ?? '';
        if (old !== undefined && old !== next) {
            el.textContent = old;
            tickTo(el, next);
        }
    }
    for (const words of $$('.kp-kpi__trend[data-tp-key]', group)) {
        const old = before.get(words.getAttribute('data-tp-key'));
        const next = words.textContent ?? '';
        if (old !== undefined && old !== next) {
            words.textContent = old;
            update(words, next);
        }
    }
}

let swapping = false;
// The automatic refresh swaps the key-figure group; the rest follows from the block it carries.
if (kpis) {
    new MutationObserver(() => {
        if (swapping || !ui.ready) return;
        const data = takeData();
        if (!data) return;
        swapping = true;
        const before = new Map(shown);
        M.model = data;
        decideSlots();
        kpis.replaceChildren(...kpiTiles());
        animateTiles(kpis, before);
        remember();
        drawBand();
        drawHubBar();
        syncRows();
        table()?.refresh();
        swapping = false;
    }).observe(kpis, { childList: true });
}

/** The kit's refresh tick (chassis ux-chassis-10): read this page again and swap in its key-figure group; the
 * observer above draws the rest. A failed read throws, and the kit's control says so with Retry now. */
async function readAgain() {
    const res = await fetch(location.pathname + location.search, { headers: { Accept: 'text/html' }, credentials: 'same-origin' });
    if (!res.ok) throw new Error(`the page answered ${res.status}`);
    const next = new DOMParser().parseFromString(await res.text(), 'text/html').getElementById('tp-kpis');
    if (!next) throw new Error('the page had no key figures');
    kpis?.replaceChildren(...next.childNodes);
    window.chassisRefresh?.markFresh();
}
if (kpis) window.chassisRefresh?.onTick(readAgain);
