// kyu's own bootstrap for the vendored @kp-soft/themes modules.
//
// Since v3.0.0 every js/*.js module is pure [KT6]: importing one attaches
// nothing. The package's own answer is js/auto.js, which attaches all
// sixteen behaviours it ships — date pickers, a data table, a wizard, drag
// reorder — none of which this dashboard has any markup for. Loading it
// would cost real bytes for behaviour with nothing to attach to, so kyu
// calls only the four attach functions its own templates use.
//
// This file is kyu's, not vendored: `.claude/hooks/gates.sh` does not
// check it against the package, because there is nothing upstream to
// check it against.

import { attachThemePickers } from './theme-picker.js';
import { enforceContracts, attachConfirmations, attachSkipLinks } from './components.js';

attachThemePickers();

// DI10 and DI4 [components.js]: a destructive button needs an undo or a
// confirmation, and a control carrying a semantic colour needs to say what
// it means in words. kyu's own badges already carry their state as text
// (`active`, `flagged`, `3 dead`), so this mainly guards the day a future
// page adds one that does not.
enforceContracts();

// The arm-then-act pattern behind `data-kp-confirm` on Revoke: a first
// click asks, a second one — inside the window — acts.
attachConfirmations();

attachSkipLinks();
