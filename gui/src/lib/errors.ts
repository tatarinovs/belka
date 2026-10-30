/** An error for the user: what happened and what to do, with the raw text kept for details. */
export interface Explained {
  summary: string;
  /** The original message, set when `summary` is an explanation of it. */
  detail?: string;
}

/**
 * The backend explains errors itself (`translate` in `error.rs`) and puts
 * the original message after this mark; the page only splits them.
 */
const DETAIL_MARK = ". Подробности: ";

export function explainError(raw: string): Explained {
  const text = raw.trim();
  const mark = text.indexOf(DETAIL_MARK);
  if (mark < 0) return { summary: text };
  return { summary: text.slice(0, mark) + ".", detail: text.slice(mark + DETAIL_MARK.length) };
}
