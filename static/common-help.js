// kyu: common-help.js, Help in the bar, its drawer with every key, and the first-visit tour of the topic page.

import { shortcuts, tour } from './common-kit.js';
import { TOUR_NAME, ON_TOPIC_PAGE, all, one } from './common-context.js';
import { dialog, help } from './common-shell.js';
import { publishField } from './common-palette.js';

/** @type {HTMLButtonElement | null} */
let helpButton = null;
const menu = one('.kp-nav .kp-theme-menu');
if (help && menu) {
    helpButton = document.createElement('button');
    helpButton.type = 'button';
    helpButton.className = 'kp-icon-button kyu-help-button';
    helpButton.setAttribute('aria-label', 'Help');
    helpButton.setAttribute('title', 'Help');
    helpButton.textContent = '?';
    menu.before(helpButton);
    helpButton.addEventListener('click', () => help.showModal());
    for (const close of all('[data-kyu-help-close]', help)) close.addEventListener('click', () => help.close());

    const list = one('[data-kyu-help-keys]', help);
    if (list) {
        const rows = [['Mod K', 'The command palette.'], ['?', 'Every key.'], ...shortcuts.listShortcuts().map((s) => [s.keys, `${s.what}.`])];
        for (const [combo, what] of rows) {
            const term = document.createElement('dt');
            combo.split(' ').forEach((part, index) => {
                if (index > 0) term.append(' ');
                const cap = document.createElement('kbd');
                cap.className = 'kp-kbd';
                cap.textContent = part.toLowerCase() === 'mod' ? (/mac|iphone|ipad/i.test(navigator.platform) ? '⌘' : 'Ctrl') : part;
                term.append(cap);
            });
            const def = document.createElement('dd');
            def.textContent = what;
            list.append(term, def);
        }
    }

    // On a page that is not a topic, the tour is a link to one: the first topic the palette lists.
    one('[data-kyu-help-tour]', help)?.addEventListener('click', () => {
        help.close();
        if (ON_TOPIC_PAGE) return runTour(0);
        const first = one('[data-kyu-topic][href]:not([data-kyu-sub])', dialog ?? document);
        if (first) location.assign(`${first.getAttribute('href')}?tour`);
    });
    if (!ON_TOPIC_PAGE && !one('[data-kyu-topic]:not([data-kyu-sub])', dialog ?? document)) one('[data-kyu-help-tour]', help)?.remove();
}

/**
 * The part of the topic page a step talks about: a marked element, else the content that follows the heading.
 * @param {string} mark @param {RegExp} heading
 */
function section(mark, heading) {
    return () => {
        const marked = one(`[data-kyu-tour="${mark}"]`);
        if (marked) return marked;
        const title = all('main h2').find((h) => heading.test(h.textContent ?? ''));
        if (!title) return null;
        let next = title.nextElementSibling;
        while (next && next.matches('p, .text-secondary, .explain')) next = next.nextElementSibling;
        return /** @type {HTMLElement | null} */ (next ?? title);
    };
}

const STEPS = [
    { target: () => one('[data-kyu-tour="snippets"]') ?? one('#snippet-publish'), title: 'Your commands', text: 'Four commands, one for each step: publish, receive, acknowledge, check. The secret is masked; Reveal shows it for ten seconds and Copy never shows it.' },
    { target: section('subs', /^Subscriptions/i), title: 'Subscriptions', text: 'Each app that reads this topic has its own place in line. Flagged means a message died or a lease ran out.' },
    { target: section('dead', /^Dead letters/i), title: 'Dead letters', text: 'A message nobody could deliver. Requeue puts it back in line; Delete removes it after you type the name.' },
    { target: () => publishField()?.closest('form') ?? publishField(), title: 'Publish a test message', text: 'Type JSON: it is checked as you type, with the line and column of a mistake. Ctrl+Enter publishes.' },
    { target: () => helpButton, title: 'Help', text: 'This tour again, and every key. Adding ?tour to the address starts it too.' },
];

function runTour(start) {
    return tour.startTour(STEPS, { start, remember: TOUR_NAME, returnFocus: helpButton });
}

if (ON_TOPIC_PAGE) {
    const at = tour.shouldStartTour({
        search: location.search,
        remembered: tour.tourRemembered(TOUR_NAME),
        automated: navigator.webdriver === true,
    });
    // The page builder's sections are in the DOM by now; give a late render one frame.
    if (at !== null) requestAnimationFrame(() => runTour(at));
}
