/** The permission modes the API screen offers for new conversations. One
 * that skips approvals is not among them: only the desktop may start with
 * it, and not from here. */
export const API_PERMISSION_MODES: readonly { value: string; label: string; hint: string }[] = [
  { value: 'default', label: 'Ask me', hint: 'Every tool call needs approval' },
  { value: 'plan', label: 'Plan only', hint: 'Claude plans but changes nothing' },
  { value: 'acceptEdits', label: 'Accept edits', hint: 'File edits go through; other tools ask' },
  { value: 'dontAsk', label: "Don't ask", hint: 'No prompts — it just works' },
  { value: 'auto', label: 'Auto', hint: 'A classifier decides what needs you' },
];
