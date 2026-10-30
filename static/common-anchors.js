// kyu: common-anchors.js, link anchors on headings and on rows, including rows a page draws later.

import { anchors } from './common-kit.js';
import { all, one, main } from './common-context.js';

/** The id a row is linked by, from what it holds: topic, subscription, dead letter or message. */
function rowId(row) {
    const own = row.getAttribute('data-kp-anchor');
    if (own) return own;
    const link = one('a[href]', row)?.getAttribute('href') ?? '';
    const sub = /\/dashboard\/subs\/([^/?#]+)/.exec(link);
    if (sub) return `sub-${anchors.anchorSlug(decodeURIComponent(sub[1]))}`;
    const topic = /^\/t\/([^/]+)\/dashboard\/?$/.exec(link);
    if (topic) return `topic-${anchors.anchorSlug(decodeURIComponent(topic[1]))}`;
    const code = one('td code, th code', row)?.textContent?.trim();
    if (code) {
        const table = row.closest('table');
        const dead = !!table && /dead/i.test(table.closest('section, div')?.previousElementSibling?.textContent ?? '') ;
        return `${dead ? 'dead' : 'message'}-${anchors.anchorSlug(code)}`;
    }
    return '';
}

/** Give every row of the page's tables its anchor, and let the tables carry the marks. */
function anchorRows() {
    for (const table of all('main table')) {
        let any = false;
        for (const row of all('tbody > tr', table)) {
            if (row.hasAttribute('data-kp-anchor') && row.getAttribute('data-kp-anchor')) { any = true; continue; }
            const id = rowId(row);
            if (!id) continue;
            row.setAttribute('data-kp-anchor', id);
            const label = (one('a, code', row)?.textContent ?? '').trim();
            if (label) row.setAttribute('data-kp-anchor-label', label);
            any = true;
        }
        if (any && !table.hasAttribute('data-kp-anchors')) table.setAttribute('data-kp-anchors', 'rows');
        if (any) anchors.attachAnchors(table.parentElement ?? table);
    }
}

if (main) {
    main.setAttribute('data-kp-anchors', 'headings');
    anchorRows();
    anchors.attachAnchors(main);
    // Tables a page fills or re-draws later (a data table's rows) get their anchors when they appear.
    let pending = 0;
    new MutationObserver((records) => {
        if (!records.some((r) => [...r.addedNodes].some((n) => n instanceof HTMLElement && (n.matches('tr') || n.querySelector('tr'))))) return;
        clearTimeout(pending);
        pending = window.setTimeout(anchorRows, 50);
    }).observe(main, { childList: true, subtree: true });
}
