// kyu: common.js, the parts that cross every page (kp-themes 11.0.0 adoption, demo kyu-11-overview).
// The entry: each part is a common-*.js module, imported here in the order it must run.
//
// Loaded by templates/_kp.html on every page, before the kp-themes bundle. The server wrote
// the palette's options into a <template id="kyu-shell"> (every topic, every subscription,
// Publish, Prune, Apps); this file moves the dialogs into the body and gives them their
// behaviour:
//
//   - the command palette (Ctrl+K), with Publish and Prune run here;
//   - the keys: g t, g a, /, p, the ? sheet and the hint line under the page;
//   - Help in the bar, its drawer, and the first-visit tour of the topic page;
//   - link anchors on headings and on rows;
//   - "who reads what" on the Topics page, from the palette's own options.
//
// Every control is a kp-themes component; kyu only decides what they say and do.

import './common-kit.js';
import './common-shell.js';
import './common-palette.js';
import './common-keys.js';
import './common-help.js';
import './common-anchors.js';
