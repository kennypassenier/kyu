// kyu: common-uses-graph.js, the hub-and-ring picture of who reads what (Topics page): kyu in the middle, a node per subscription.

import { graph } from './common-kit.js';
import { counted } from './common-context.js';

/** The kinds of link the picture draws: a subscription's state. */
const KINDS = [
    { kind: 'active', label: 'Active', hint: 'The app is reading and acknowledging', style: 'solid', colour: 'var(--chart-1)' },
    { kind: 'flagged', label: 'Flagged', hint: 'A message died or a lease ran out on this subscription', style: 'dash', colour: 'var(--destructive)' },
    { kind: 'archived', label: 'Archived', hint: 'No longer reading; kept for its backlog', style: 'dot', colour: 'var(--muted-foreground)' },
];

/** The graph's data: kyu in the middle, a node per subscription. @param {{ id: string, topic: string, app: string, kind: string, waiting: number, note: string }[]} subs @param {boolean} bySize */
function graphData(subs, bySize) {
    const most = Math.max(1, ...subs.map((s) => s.waiting));
    return {
        hub: 'kyu',
        kinds: KINDS,
        nodes: [
            { id: 'kyu', label: 'kyu', description: 'the message hub', weight: bySize ? 1 : null },
            ...subs.map((s) => ({
                id: s.id,
                label: `${s.topic} / ${s.app}`,
                description: `${counted(s.waiting, 'message')} waiting, ${s.note}`,
                weight: bySize ? Math.max(0.12, s.waiting / most) : null,
                flag: null,
            })),
        ],
        edges: subs.map((s) => ({ from: 'kyu', to: s.id, kind: s.kind, detail: `${counted(s.waiting, 'message')} waiting, ${s.note}` })),
    };
}

/**
 * The hub-and-ring picture of the same subscriptions: kyu in the middle, a node per subscription, the link
 * kind = the subscription's state. Node size is the package's `weight`: unset (all the same) or the backlog.
 * @param {HTMLElement} section @param {() => { subs: any[] } | null} current the subscriptions last read
 * @returns {(state: 'loading' | 'ready' | 'error') => void} draw the picture in that state
 */
export function mountGraph(section, current) {
    const el = /** @type {HTMLElement} */ (section.querySelector('#kyu-uses-graph'));
    const out = /** @type {HTMLElement} */ (section.querySelector('#kyu-uses-readout'));
    let bySize = false;
    graph.attachGraphs(section);
    /** @param {'loading' | 'ready' | 'error'} state */
    const draw = (state) => {
        if (state === 'loading') {
            graph.setGraphState(el, 'loading', 'Reading the subscriptions from kyu.');
            return;
        }
        if (state === 'error') {
            graph.setGraphState(el, 'error', 'Could not read who reads what', { text: 'kyu did not answer.' });
            return;
        }
        const subs = current()?.subs ?? [];
        if (subs.length === 0) {
            graph.setGraphState(el, 'empty', 'Nothing to draw yet: no topic has a subscription.');
            return;
        }
        graph.setGraphData(el, graphData(subs, bySize));
    };
    section.addEventListener('click', (event) => {
        const item = /** @type {HTMLElement | null} */ ((/** @type {Element} */ (event.target)).closest('[data-kyu-switch="size"] [data-value]'));
        if (!item) return;
        bySize = item.dataset.value === 'backlog';
        draw('ready');
    });
    el.addEventListener(graph.GRAPH_CHANGE_EVENT, (event) => {
        const { selected, hiddenKinds } = /** @type {CustomEvent} */ (event).detail;
        const name = (k) => KINDS.find((x) => x.kind === k)?.label ?? k;
        out.textContent =
            (selected.length ? `Picked: ${selected.join(', ')}.` : 'Nothing picked.') +
            (hiddenKinds.length ? ` Hidden: ${hiddenKinds.map(name).join(', ')}.` : ' Every kind of link is shown.');
    });
    return draw;
}
