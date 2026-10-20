// Shortcuts use the platform's command key: Cmd on a Mac, Ctrl elsewhere. A Mac's Ctrl+F and
// Ctrl+Z did nothing for people who press Cmd+F and Cmd+Z everywhere else.
export const isMac = navigator.userAgent.includes('Macintosh');

/** Whether the platform's command key (Cmd on a Mac, Ctrl elsewhere) is held. */
export const commandKey = (e: KeyboardEvent): boolean => (isMac ? e.metaKey : e.ctrlKey);

/** The command key as people write it in a shortcut: "Cmd" or "Ctrl". */
export const commandName = isMac ? 'Cmd' : 'Ctrl';
