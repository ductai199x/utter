// A live waveform from microphone levels (RMS, about 60 a second): each bar shows the peak of
// its slice of the recent past, newest on the right, rising fast and falling gently. The Bar
// (docs/ux/bar.md) and first run's microphone check draw the same way.

const ATTACK_MS = 30;
const RELEASE_MS = 140;

/** RMS (0..1) to a 0..1 bar height: speech sits around 0.02-0.2 RMS, so a log scale. */
export function shape(rms: number): number {
  const db = 20 * Math.log10(Math.max(rms, 1e-5));
  return Math.min(1, Math.max(0, (db + 55) / 40));
}

export interface Geometry {
  bars: number;
  barW: number;
  gap: number;
  /** Bar heights, px. */
  min: number;
  max: number;
}

export class Waveform {
  private readonly levels: Float32Array;
  private head = 0;
  private readonly heights: Float32Array;

  /** `history`: how many levels the bars span (60 per second). */
  constructor(
    readonly g: Geometry,
    history: number,
  ) {
    this.levels = new Float32Array(history);
    this.heights = new Float32Array(g.bars).fill(g.min);
  }

  get width(): number {
    return this.g.bars * this.g.barW + (this.g.bars - 1) * this.g.gap;
  }

  push(rms: number) {
    this.levels[this.head] = rms;
    this.head = (this.head + 1) % this.levels.length;
  }

  clear() {
    this.levels.fill(0);
  }

  /** Eases the bars toward the levels over `dt` ms (to rest when not `live`) and draws them,
   *  vertically centered, on a canvas of `width` x `max` CSS px. */
  draw(canvas: HTMLCanvasElement, color: string, dt: number, live: boolean) {
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    const { bars, barW, gap, min, max } = this.g;
    const dpr = window.devicePixelRatio || 1;
    const w = this.width;
    if (canvas.width !== Math.round(w * dpr) || canvas.height !== Math.round(max * dpr)) {
      canvas.width = Math.round(w * dpr);
      canvas.height = Math.round(max * dpr);
    }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, max);
    ctx.fillStyle = color;
    const history = this.levels.length;
    const per = history / bars;
    for (let i = 0; i < bars; i++) {
      let peak = 0;
      for (let k = 0; k < per; k++) {
        peak = Math.max(peak, this.levels[(this.head + Math.floor(i * per) + k) % history] ?? 0);
      }
      const target = live ? min + shape(peak) * (max - min) : min;
      const h = this.heights[i]!;
      const tau = target > h ? ATTACK_MS : RELEASE_MS;
      const bh = h + (target - h) * (1 - Math.exp(-dt / tau));
      this.heights[i] = bh;
      ctx.beginPath();
      ctx.roundRect(i * (barW + gap), (max - bh) / 2, barW, bh, barW / 2);
      ctx.fill();
    }
  }
}
