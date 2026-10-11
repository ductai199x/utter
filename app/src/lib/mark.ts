// Utter's mark (breath-lines, Tai's pick 2026-10-11): a voice, a ribbon with one soft rise,
// that lays itself down into three lines of text. Drawn on a 64-unit grid with round caps, so
// it is crisp at 16 px and every stroke can animate.

/** Stroke width in grid units. */
export const MARK_STROKE = 4.6;

/** The ribbon: a small hook, one rise, then the first line. `rise` scales the rise's height
 *  (1 = the mark as drawn; the splash breathes it between about 0.82 and 1.06). */
export function ribbonPath(rise = 1, end = LINE_END): string {
  const f = (n: number) => n.toFixed(2);
  const base = 29.6; // the first line's height
  const peak = base - 13.4 * rise; // the rise's top
  const dip = base + 1.7 * rise; // the hook's low point
  return [
    `M 9.2 ${f(base - 1.4)}`,
    // The hook, then up into a broad, soft rise and down into the first line.
    `C 10.6 ${f(dip)}, 13.6 ${f(dip + 0.4)}, 15.6 ${f(dip - 3.4)}`,
    `C 17.4 ${f(peak + 7.4)}, 19.4 ${f(peak)}, 23.4 ${f(peak)}`,
    `C 27.4 ${f(peak)}, 28.8 ${f(peak + 8.2)}, 30.8 ${f(base - 3.2)}`,
    `C 32.4 ${f(base - 0.7)}, 34 ${f(base)}, 36.8 ${f(base)}`,
    `L ${f(end)} ${f(base)}`,
  ].join(' ');
}

/** Where the lines end, as drawn. */
export const LINE_END = 54.4;
/** The first line's height (the ribbon ends on it), then the two lines under it. */
export const LINE_Y = [29.6, 37.4, 45.2];
/** Where the two lines under the first start. */
export const LINE_START = 33.4;

/** One of the two lines under the first, ending at `end`. */
export function linePath(y: number, end = LINE_END): string {
  return `M ${LINE_START} ${y} L ${end.toFixed(2)} ${y}`;
}

/** The two lines under the first, as drawn. */
export const LINES = [linePath(LINE_Y[1]!), linePath(LINE_Y[2]!)];
