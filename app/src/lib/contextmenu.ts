// The web view's own right-click menu offers Reload (and on Windows Back, Print, Save as),
// which would reload a page in the middle of setup or a dictation. It stays where it helps:
// in text fields and on selected text, for copy and paste.
document.addEventListener('contextmenu', (e) => {
  const t = e.target as HTMLElement | null;
  const editable = t?.closest('input, textarea, [contenteditable=""], [contenteditable="true"]');
  const selected = (window.getSelection()?.toString() ?? '') !== '';
  if (!editable && !selected) e.preventDefault();
});
