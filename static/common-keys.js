// kyu: common-keys.js, the keys (g t, g a, /, p, the ? sheet) and the hint line under the page.

import { palette, shortcuts, keys } from './common-kit.js';
import { ON_TOPIC_PAGE, main } from './common-context.js';
import { dialog, sheet } from './common-shell.js';
import { publishField, goPublish } from './common-palette.js';

shortcuts.configureShortcuts({ unknownHint: true });
shortcuts.registerShortcuts('page', [
    { keys: 'g t', what: 'Go to topics', group: 'Anywhere', run: () => location.assign('/topics') },
    { keys: 'g a', what: 'Go to apps', group: 'Anywhere', run: () => location.assign('/clients') },
    { keys: '/', what: 'Search topics and apps', group: 'Anywhere', run: () => palette.palette(dialog)?.open(), when: () => !!dialog },
    {
        keys: 'p',
        what: 'Publish a test message',
        group: 'Anywhere',
        run: () => goPublish(),
        when: () => ON_TOPIC_PAGE && !!publishField(),
        reason: 'Open a topic first',
    },
]);
if (sheet) shortcuts.attachShortcuts(document.body);

// The hint line, under the page.
if (main) {
    const line = document.createElement('div');
    line.className = 'kp-keys kyu-keys';
    keys.setKeys(line, [
        [['g', 't'], 'topics'],
        [['g', 'a'], 'apps'],
        ['/', 'search'],
        ...(ON_TOPIC_PAGE ? [['p', 'publish']] : []),
        ['?', 'all keys'],
        [['Mod', 'K'], 'palette'],
    ]);
    main.append(line);
}
