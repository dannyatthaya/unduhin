// Pure helpers behind the settings search: matching, the "did you mean"
// suggestion for a query with no results, and splitting text around the
// query so the match can be marked.

/** One searchable setting, already translated. */
export interface SearchableSetting {
  id: string;
  /** Everything the query may match: label, description, section, keywords. */
  texts: string[];
}

export function normalizeQuery(query: string): string {
  return query.trim().toLowerCase();
}

export function matchesQuery(setting: SearchableSetting, query: string): boolean {
  const q = normalizeQuery(query);
  if (q === "") return true;
  return setting.texts.some((text) => text.toLowerCase().includes(q));
}

/** Levenshtein distance, abandoned once it must exceed `limit`. */
export function editDistance(a: string, b: string, limit: number): number {
  if (Math.abs(a.length - b.length) > limit) return limit + 1;
  let previous = Array.from({ length: b.length + 1 }, (_, i) => i);
  for (let i = 1; i <= a.length; i += 1) {
    const current = [i];
    let rowMin = i;
    for (let j = 1; j <= b.length; j += 1) {
      const cost = a[i - 1] === b[j - 1] ? 0 : 1;
      const value = Math.min(previous[j] + 1, current[j - 1] + 1, previous[j - 1] + cost);
      current.push(value);
      rowMin = Math.min(rowMin, value);
    }
    if (rowMin > limit) return limit + 1;
    previous = current;
  }
  return previous[b.length];
}

const MIN_SUGGESTION_LENGTH = 4;
const MAX_TYPOS = 2;

/**
 * The word a mistyped query most likely meant: the closest word (at most
 * two typos away) that appears in any setting. `null` when nothing is close.
 */
export function suggestQuery(query: string, settings: SearchableSetting[]): string | null {
  const q = normalizeQuery(query);
  if (q.length < MIN_SUGGESTION_LENGTH || q.includes(" ")) return null;
  let best: string | null = null;
  let bestDistance = MAX_TYPOS + 1;
  const seen = new Set<string>();
  for (const setting of settings) {
    for (const text of setting.texts) {
      for (const word of text.toLowerCase().split(/[^\p{L}\p{N}-]+/u)) {
        if (word.length < MIN_SUGGESTION_LENGTH || seen.has(word)) continue;
        seen.add(word);
        const distance = editDistance(q, word, MAX_TYPOS);
        if (distance > 0 && distance < bestDistance) {
          best = word;
          bestDistance = distance;
        }
      }
    }
  }
  return best;
}

export interface TextPiece {
  text: string;
  match: boolean;
}

/** Split `text` into pieces, flagging each case-insensitive match of `query`. */
export function splitByQuery(text: string, query: string): TextPiece[] {
  const q = normalizeQuery(query);
  if (q === "" || text === "") return [{ text, match: false }];
  const lower = text.toLowerCase();
  const pieces: TextPiece[] = [];
  let from = 0;
  for (let at = lower.indexOf(q); at !== -1; at = lower.indexOf(q, from)) {
    if (at > from) pieces.push({ text: text.slice(from, at), match: false });
    pieces.push({ text: text.slice(at, at + q.length), match: true });
    from = at + q.length;
  }
  if (from < text.length) pieces.push({ text: text.slice(from), match: false });
  return pieces;
}
