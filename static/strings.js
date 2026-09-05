// Every user-visible string this package can produce [KT5].
//
// The fault this closes, in Kenny's words: "Wij bieden de basis, de
// consumenten vullen de inhoud in." Until 2.0.0 every component spoke
// hardcoded Dutch in both channels, so an English consumer could adopt
// exactly the components that carry no text — and that was measurably
// the case: JobTracker took Button, Badge, Card, Alert, Health,
// EmptyState and Toasts, which are precisely the files with a zero in
// the count. Everything from round two, including the DataTable and the
// forms Kenny had asked for, was unreachable.
//
// **The fault is not "Dutch strings".** It is a user-visible string with
// no way in from outside, which the JobTracker session named more
// precisely than the first version of the correction did. A Dutch app
// that wants "Overnemen" where this says "Opslaan" is just as stuck, and
// translating everything to English would leave the same defect wearing
// a different word. So the shape of the fix is a way IN, and the default
// language is a separate decision that happens to be English.
//
// Screen-reader-only text is in here too, and that is the half that
// matters most. `Copyable` already took its visible labels as props and
// kept its announcement hardcoded, so an English page said one thing on
// screen and another to a screen reader. Half-translatable is worse than
// untranslatable: it fails silently, and only for the people who cannot
// see that it failed.
//
// Three ways to supply your own, all of them optional:
//
//   import { setStrings } from '@kp-soft/themes/js/strings';
//   setStrings({ formRequired: 'required' });        // framework-free
//
//   <StringsProvider value={{ formRequired: 'vereist' }}>…   // React
//
//   <DataTable strings={{ tableSearch: 'Rechercher' }} />    // per use
//
// Anything not supplied falls back to the English below, so a consumer
// overrides what it cares about and nothing else.

/**
 * @typedef {object} Strings
 * @property {string} alertSuccess
 * @property {string} alertWarning
 * @property {string} alertInfo
 * @property {string} alertError
 * @property {string} busy
 * @property {string} close
 * @property {string} previous
 * @property {string} next
 * @property {string} finish
 * @property {string} back
 * @property {(name: string) => string} removeNamed
 * @property {string} noResults
 * @property {string} oneResult
 * @property {(n: number) => string} manyResults
 * @property {string} noCommands
 * @property {string} oneCommand
 * @property {(n: number) => string} manyCommands
 * @property {string} commandPlaceholder
 * @property {string} commandsLabel
 * @property {string} shortcutsLabel
 * @property {string} tableSearch
 * @property {string} tableSearchLabel
 * @property {string} tableSelectAll
 * @property {(key: string) => string} tableSelectRow
 * @property {string} tableEmpty
 * @property {(n: number) => string} tableRows
 * @property {(shown: number, total: number) => string} tableRowsFiltered
 * @property {(at: number, of: number) => string} tablePage
 * @property {string} formRequired
 * @property {string} formInvalid
 * @property {string} formSummaryOne
 * @property {(n: number) => string} formSummaryMany
 * @property {string} fieldFallbackName
 * @property {string} calendarOpen
 * @property {string} calendarButton
 * @property {string} dateFormatHint
 * @property {string} previousMonth
 * @property {string} nextMonth
 * @property {(month: string, year: number) => string} monthTitle
 * @property {string[]} weekdays
 * @property {string[]} months
 * @property {(day: number, month: string, year: number) => string} dayLabel
 * @property {string} uploadZone
 * @property {(size: string) => string} uploadTooLarge
 * @property {(max: number) => string} uploadTooMany
 * @property {(max: string) => string} uploadTotalTooLarge
 * @property {(accept: string) => string} uploadWrongType
 * @property {(name: string) => string} uploadProgress
 * @property {(at: number, of: number) => string} wizardStep
 * @property {string} copy
 * @property {string} copied
 * @property {string} copyBlocked
 * @property {(value: string) => string} copiedAnnouncement
 * @property {string} copyBlockedAnnouncement
 * @property {string} undo
 * @property {string} deleted
 * @property {string} splitLabel
 * @property {(name: string) => string} reorderHandle
 * @property {(name: string, at: number, of: number) => string} reorderMoved
 * @property {string} tileFallbackName
 * @property {(name: string, column: number, row: number, w: number, h: number) => string} tileLabel
 * @property {(token: string) => string} contrastMissing
 * @property {(ratio: string, token: string, verdict: string) => string} contrastReport
 * @property {string} colourHue
 * @property {string} colourSaturation
 * @property {string} colourLightness
 * @property {string} contrastPasses
 * @property {string} contrastFails
 * @property {string} confirm
 * @property {string} save
 * @property {string} mainNavigation
 * @property {string} skipToContent
 * @property {string} breadcrumb
 * @property {string} pagination
 * @property {string} themePicker
 * @property {string} themeSaveFailed
 * @property {string} themeSaveRefused
 * @property {string} contractDestructive
 * @property {string} contractSemantic
 * @property {string} themeGroupLight
 * @property {string} themeGroupDark
 */

/**
 * The defaults. English, by Kenny's decision of 2026-09-04 — the package
 * has to speak something, and English is the language a consumer is least
 * likely to have to replace.
 *
 * @type {Strings}
 */
export const DEFAULT_STRINGS = Object.freeze({
    alertSuccess: 'Success',
    alertWarning: 'Warning',
    alertInfo: 'Info',
    alertError: 'Error',
    busy: 'Working…',
    close: 'Close',
    previous: 'Previous',
    next: 'Next',
    finish: 'Finish',
    back: 'Back',
    // Named rather than a bare ×: a column of identical remove buttons is
    // useless to anyone who cannot see which row they are in.
    removeNamed: (name) => `Remove ${name}`,
    noResults: 'No results',
    oneResult: '1 result',
    manyResults: (n) => `${n} results`,
    noCommands: 'No commands',
    oneCommand: '1 command',
    manyCommands: (n) => `${n} commands`,
    commandPlaceholder: 'Type a command…',
    commandsLabel: 'Commands',
    shortcutsLabel: 'Keyboard shortcuts',
    tableSearch: 'Search…',
    tableSearchLabel: 'Search the table',
    tableSelectAll: 'Select every visible row',
    tableSelectRow: (key) => `Select row ${key}`,
    tableEmpty: 'Nothing found.',
    tableRows: (n) => `${n} rows`,
    tableRowsFiltered: (shown, total) => `${shown} of ${total} rows`,
    /** The pager's position, "2 / 5". A function, so a consumer reorders it. @param {number} at @param {number} of */
    tablePage: (at, of) => `${at} / ${of}`,
    formRequired: 'required',
    formInvalid: 'This field is not filled in correctly.',
    formSummaryOne: '1 field is not filled in correctly.',
    formSummaryMany: (n) => `${n} fields are not filled in correctly.`,
    fieldFallbackName: 'Field',
    calendarOpen: 'Open the calendar',
    calendarButton: 'Calendar',
    dateFormatHint: 'dd-mm-yyyy',
    previousMonth: 'Previous month',
    nextMonth: 'Next month',
    /** The calendar's heading. A function, so a locale that writes the year first can. @param {string} month @param {number} year */
    monthTitle: (month, year) => `${month} ${year}`,
    weekdays: ['Mo', 'Tu', 'We', 'Th', 'Fr', 'Sa', 'Su'],
    months: ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'],
    // The full date, because "4" alone tells a screen reader nothing about
    // which month it is in.
    dayLabel: (day, month, year) => `${day} ${month} ${year}`,
    uploadZone: 'Drop files here or choose them',
    uploadTooLarge: (size) => `Larger than ${size}`,
    /** @param {number} max */
    uploadTooMany: (max) => `No more than ${max} files.`,
    /** @param {string} max */
    uploadTotalTooLarge: (max) => `Together the files may not exceed ${max}.`,
    /** @param {string} accept */
    uploadWrongType: (accept) => `Only ${accept} files.`,
    uploadProgress: (name) => `Progress of ${name}`,
    wizardStep: (at, of) => `Step ${at} of ${of}`,
    copy: 'Copy',
    copied: 'Copied',
    copyBlocked: 'Blocked',
    // Announced as well as shown: a button's own label changing is not
    // something a screen reader reports on its own.
    copiedAnnouncement: (value) => `${value} copied`,
    copyBlockedAnnouncement: 'Copying is blocked in this browser',
    undo: 'Undo',
    deleted: 'Deleted.',
    splitLabel: 'Resize the panes',
    reorderHandle: (name) => `Move ${name}`,
    /** Announced after a keyboard or pointer move. @param {string} name @param {number} at @param {number} of */
    reorderMoved: (name, at, of) => `${name} moved to position ${at} of ${of}`,
    tileFallbackName: 'Tile',
    tileLabel: (name, column, row, w, h) => `${name}, column ${column}, row ${row}, ${w} by ${h}`,
    contrastMissing: (token) => `No contrast to measure: ${token} does not exist in this theme.`,
    contrastReport: (ratio, token, verdict) => `${ratio}:1 against ${token} — ${verdict}`,
    colourHue: 'Hue',
    colourSaturation: 'Saturation',
    colourLightness: 'Lightness',
    contrastPasses: 'passes',
    contrastFails: 'too little',
    confirm: 'Confirm',
    save: 'Save',
    mainNavigation: 'Main navigation',
    skipToContent: 'Skip to the content',
    breadcrumb: 'Breadcrumb',
    pagination: 'Pagination',
    themePicker: 'Choose a theme',
    themeSaveFailed: 'This choice will not be remembered — storage is blocked in this browser.',
    themeSaveRefused: 'Not saved on the server — your choice has been put back.',
    /** The two contract violations enforceContracts reports [DI10, DI4]. */
    contractDestructive:
        'A destructive action must offer an undo (data-kp-undo) or a confirmation (data-kp-confirm="phrase"). SC 3.3.4 accepts either; it accepts neither of them missing.',
    contractSemantic: 'A control carrying a semantic colour must also say what it means: colour is never the only carrier.',
    /** The two sections of a grouped theme picker [TH63]. */
    themeGroupLight: 'Light',
    themeGroupDark: 'Dark',
});

/**
 * Dutch, kept as an export rather than as the default.
 *
 * Three consumers — kyu, almanac and kp-soft — were reading Dutch until
 * 2.0.0 and would otherwise have had to write it out again. One line
 * restores what they had:
 *
 *   setStrings(STRINGS_NL);
 *
 * @type {Strings}
 */
export const STRINGS_NL = Object.freeze({
    alertSuccess: 'Gelukt',
    alertWarning: 'Let op',
    alertInfo: 'Info',
    alertError: 'Fout',
    busy: 'Bezig…',
    close: 'Sluiten',
    previous: 'Vorige',
    next: 'Volgende',
    finish: 'Afronden',
    back: 'Terug',
    removeNamed: (name) => `${name} verwijderen`,
    noResults: 'Geen resultaten',
    oneResult: '1 resultaat',
    manyResults: (n) => `${n} resultaten`,
    noCommands: 'Geen opdrachten',
    oneCommand: '1 opdracht',
    manyCommands: (n) => `${n} opdrachten`,
    commandPlaceholder: 'Typ een opdracht…',
    commandsLabel: 'Opdrachten',
    shortcutsLabel: 'Sneltoetsen',
    tableSearch: 'Zoeken…',
    tableSearchLabel: 'Zoeken in de tabel',
    tableSelectAll: 'Alle zichtbare rijen selecteren',
    tableSelectRow: (key) => `Rij ${key} selecteren`,
    tableEmpty: 'Niets gevonden.',
    tableRows: (n) => `${n} rijen`,
    tableRowsFiltered: (shown, total) => `${shown} van ${total} rijen`,
    tablePage: (at, of) => `${at} / ${of}`,
    formRequired: 'verplicht',
    formInvalid: 'Dit veld is niet correct ingevuld.',
    formSummaryOne: 'Er is 1 veld niet correct ingevuld.',
    formSummaryMany: (n) => `Er zijn ${n} velden niet correct ingevuld.`,
    fieldFallbackName: 'Veld',
    calendarOpen: 'Kalender openen',
    calendarButton: 'Kalender',
    dateFormatHint: 'dd-mm-jjjj',
    previousMonth: 'Vorige maand',
    nextMonth: 'Volgende maand',
    monthTitle: (month, year) => `${month} ${year}`,
    weekdays: ['ma', 'di', 'wo', 'do', 'vr', 'za', 'zo'],
    months: ['januari', 'februari', 'maart', 'april', 'mei', 'juni', 'juli', 'augustus', 'september', 'oktober', 'november', 'december'],
    dayLabel: (day, month, year) => `${day} ${month} ${year}`,
    uploadZone: 'Sleep bestanden hierheen of kies ze',
    uploadTooLarge: (size) => `Groter dan ${size}`,
    uploadTooMany: (max) => `Niet meer dan ${max} bestanden.`,
    uploadTotalTooLarge: (max) => `Samen mogen de bestanden niet groter zijn dan ${max}.`,
    uploadWrongType: (accept) => `Alleen ${accept}-bestanden.`,
    uploadProgress: (name) => `Voortgang van ${name}`,
    wizardStep: (at, of) => `Stap ${at} van ${of}`,
    copy: 'Kopiëren',
    copied: 'Gekopieerd',
    copyBlocked: 'Geblokkeerd',
    copiedAnnouncement: (value) => `${value} gekopieerd`,
    copyBlockedAnnouncement: 'Kopiëren is geblokkeerd',
    undo: 'Ongedaan maken',
    deleted: 'Verwijderd.',
    splitLabel: 'Panelen verdelen',
    reorderHandle: (name) => `Verplaats ${name}`,
    reorderMoved: (name, at, of) => `${name} verplaatst naar positie ${at} van ${of}`,
    tileFallbackName: 'Tegel',
    tileLabel: (name, column, row, w, h) => `${name}, kolom ${column}, rij ${row}, ${w} bij ${h}`,
    contrastMissing: (token) => `Geen contrast te meten: ${token} bestaat niet in dit thema.`,
    contrastReport: (ratio, token, verdict) => `${ratio}:1 tegen ${token} — ${verdict}`,
    colourHue: 'Tint',
    colourSaturation: 'Verzadiging',
    colourLightness: 'Helderheid',
    contrastPasses: 'haalbaar',
    contrastFails: 'te weinig',
    confirm: 'Bevestigen',
    save: 'Opslaan',
    mainNavigation: 'Hoofdnavigatie',
    skipToContent: 'Naar de inhoud',
    breadcrumb: 'Kruimelpad',
    pagination: 'Paginering',
    themePicker: 'Thema kiezen',
    themeSaveFailed: 'Deze keuze wordt niet onthouden — opslag is geblokkeerd in deze browser.',
    themeSaveRefused: 'Niet bewaard op de server — je keuze is teruggezet.',
    contractDestructive:
        'Een destructieve actie moet een undo (data-kp-undo) of een bevestiging (data-kp-confirm="zin") bieden. SC 3.3.4 aanvaardt beide; het aanvaardt niet dat beide ontbreken.',
    contractSemantic: 'Een element met een semantische kleur moet ook in woorden zeggen wat het betekent: kleur is nooit de enige drager.',
    themeGroupLight: 'Licht',
    themeGroupDark: 'Donker',
});

/** @type {Strings} */
let current = DEFAULT_STRINGS;

/**
 * Replace some or all of the strings, for the framework-free channel and
 * for anything that reads them outside React.
 *
 * Merged rather than replaced: a consumer that wants one word does not
 * have to restate the other seventy, and a key added in a later version
 * keeps working instead of becoming `undefined` on their page.
 *
 * @param {Partial<Strings>} next
 * @returns {Strings} the merged result
 */
export function setStrings(next) {
    current = Object.freeze({ ...current, ...next });
    return current;
}

/** @returns {Strings} the strings as they stand */
export function getStrings() {
    return current;
}

/**
 * The strings a component should use: the global ones, with anything the
 * caller passed layered on top.
 *
 * @param {Partial<Strings>} [overrides]
 * @returns {Strings}
 */
export function resolveStrings(overrides) {
    return overrides === undefined ? current : { ...current, ...overrides };
}
