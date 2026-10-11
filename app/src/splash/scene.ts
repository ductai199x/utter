// The splash scene (docs/ux/splash.md): Utter's mark, told at the size of the card. Ribbons
// of light come in from the left the way a voice does, louder and quieter in phrases, and
// settle into the ruled lines of a page on the right. When a phrase arrives, a glowing
// write-head writes it out on the line as words; in a pause the head waits. When the page is
// full it rests, fades, and the next page begins. Each loading stage sends a pulse of light
// along the ribbons into the page.

export interface SceneOptions {
  width: number;
  height: number;
  reducedMotion: boolean;
}

interface Ribbon {
  from: number; // height at the card's left edge
  line: number; // the line it lays into
  amp: number; // wave height, px
  wavelength: number; // px
  speed: number; // radians per second
  phase: number;
  color: [number, number, number];
}

interface Word {
  x0: number;
  x1: number;
}

const JUNCTION = 300; // where the ribbons become lines
const LINE_START = JUNCTION + 10; // where the writing starts
const LINE_END = 588;
const LINE_Y = [104, 130, 156, 182, 208];
const WORD_HEIGHT = 4.5;
const WORD_GAP = 9;
const TRAVEL = 120; // how fast a phrase travels along the ribbons, px per second
const WRITE_SPEED = 120; // the write-head at full voice, px per second
const PAGE_REST_MS = 1800; // a full page, still, before it fades
const PAGE_FADE_MS = 1200;
const PULSE_MS = 1500; // a stage's pulse crossing the ribbons

const RIBBONS: Ribbon[] = [
  { from: 150, line: 0, amp: 24, wavelength: 132, speed: 1.9, phase: 0.0, color: [120, 138, 255] },
  { from: 232, line: 1, amp: 30, wavelength: 158, speed: 1.6, phase: 1.7, color: [107, 124, 255] },
  { from: 84, line: 2, amp: 34, wavelength: 118, speed: 2.1, phase: 3.1, color: [140, 156, 255] },
  { from: 196, line: 3, amp: 26, wavelength: 146, speed: 1.75, phase: 4.4, color: [118, 150, 255] },
  { from: 118, line: 4, amp: 22, wavelength: 170, speed: 1.5, phase: 5.6, color: [132, 120, 250] },
];

const clamp01 = (v: number) => Math.min(1, Math.max(0, v));
const smooth = (a: number, b: number, v: number) => {
  const s = clamp01((v - a) / (b - a));
  return s * s * (3 - 2 * s);
};
const smoother = (s: number) => s * s * s * (s * (s * 6 - 15) + 10);
const easeInOut = (s: number) => (s < 0.5 ? 4 * s * s * s : 1 - Math.pow(-2 * s + 2, 3) / 2);

/** How loud the voice is at time `s` (seconds), 0..1: phrases with pauses between them. */
function voice(s: number): number {
  const raw = Math.sin(s * 1.13) + 0.6 * Math.sin(s * 0.47 + 1.3) + 0.4 * Math.sin(s * 2.31 + 0.4);
  return smooth(-0.3, 1.1, raw);
}

export class Scene {
  private readonly o: SceneOptions;
  private page: Word[][] = [];
  private line = 0;
  private head = LINE_START;
  private headGlow = 0;
  private fullAt = -1; // when the page filled, ms
  private pulses: number[] = [];
  private last = -1;
  private seed = 7;

  constructor(options: SceneOptions) {
    this.o = options;
    this.newPage();
    if (options.reducedMotion) {
      // Still: a written page, the ribbons where they rest.
      this.line = LINE_Y.length;
    }
  }

  /** A stage finished: a pulse of light along the ribbons. */
  pulse(now: number) {
    if (!this.o.reducedMotion) this.pulses.push(now);
  }

  draw(canvas: HTMLCanvasElement, now: number) {
    const { width, height } = this.o;
    const dpr = Math.min(2, window.devicePixelRatio || 1);
    if (canvas.width !== Math.round(width * dpr)) {
      canvas.width = Math.round(width * dpr);
      canvas.height = Math.round(height * dpr);
    }
    const ctx = canvas.getContext('2d')!;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, height);

    const t = this.o.reducedMotion ? 4.2 : now / 1000;
    const dt = this.last < 0 ? 0 : Math.min(0.05, (now - this.last) / 1000);
    this.last = now;
    if (!this.o.reducedMotion) this.write(t, dt, now);
    this.pulses = this.pulses.filter((p) => now - p < PULSE_MS + 600);

    const fade = this.fullAt < 0 ? 1 : 1 - smooth(PAGE_REST_MS, PAGE_REST_MS + PAGE_FADE_MS, now - this.fullAt);
    this.drawRules(ctx, now);
    ctx.globalCompositeOperation = 'lighter';
    RIBBONS.forEach((r, i) => this.drawRibbon(ctx, r, i, t, now));
    ctx.globalCompositeOperation = 'source-over';
    this.drawWords(ctx, fade);
  }

  // The page ---------------------------------------------------------------------------------

  private random() {
    // A small seeded generator, so pages vary but a review replays the same way.
    this.seed = (this.seed * 16807) % 2147483647;
    return this.seed / 2147483647;
  }

  private newPage() {
    this.page = LINE_Y.map((_, i) => {
      const words: Word[] = [];
      // The last line of a paragraph stops short.
      const end = i === LINE_Y.length - 1 ? LINE_START + (LINE_END - LINE_START) * (0.35 + 0.3 * this.random()) : LINE_END;
      let x = LINE_START;
      for (;;) {
        const length = 14 + Math.pow(this.random(), 1.4) * 52;
        if (x + length > end) break;
        words.push({ x0: x, x1: x + length });
        x += length + WORD_GAP;
      }
      return words;
    });
    this.line = 0;
    this.head = LINE_START;
    this.fullAt = -1;
  }

  /** The write-head moves as fast as the voice arriving at the page is loud. */
  private write(t: number, dt: number, now: number) {
    if (this.fullAt >= 0) {
      if (now - this.fullAt > PAGE_REST_MS + PAGE_FADE_MS + 300) this.newPage();
      this.headGlow = 0;
      return;
    }
    const arriving = voice(t - JUNCTION / TRAVEL + this.line * 0.08);
    const speed = WRITE_SPEED * smooth(0.15, 0.8, arriving);
    this.head += speed * dt;
    this.headGlow += (smooth(0.2, 0.9, arriving) - this.headGlow) * Math.min(1, dt * 6);
    const words = this.page[this.line]!;
    if (this.head >= (words[words.length - 1]?.x1 ?? LINE_START)) {
      this.line++;
      this.head = LINE_START;
      if (this.line >= LINE_Y.length) this.fullAt = now;
    }
  }

  private drawRules(ctx: CanvasRenderingContext2D, now: number) {
    // Faint ruled lines, a little brighter as a stage's pulse arrives.
    const arrival = Math.max(0, ...this.pulses.map((p) => {
      const s = (now - p) / PULSE_MS;
      return s > 0.75 ? Math.exp(-Math.pow((s - 0.95) / 0.18, 2)) : 0;
    }));
    ctx.lineCap = 'round';
    ctx.lineWidth = 1;
    for (const y of LINE_Y) {
      const g = ctx.createLinearGradient(JUNCTION - 8, 0, LINE_END + 20, 0);
      const a = 0.07 + 0.12 * arrival;
      g.addColorStop(0, `rgba(150, 164, 255, ${a * 1.6})`);
      g.addColorStop(0.7, `rgba(150, 164, 255, ${a})`);
      g.addColorStop(1, 'rgba(150, 164, 255, 0)');
      ctx.strokeStyle = g;
      ctx.beginPath();
      ctx.moveTo(JUNCTION - 8, y);
      ctx.lineTo(LINE_END + 20, y);
      ctx.stroke();
    }
  }

  // The ribbons -------------------------------------------------------------------------------

  private ribbonY(r: Ribbon, i: number, x: number, t: number) {
    const u = clamp01(x / JUNCTION);
    const lineY = LINE_Y[r.line]!;
    const base = r.from + (lineY - r.from) * smoother(u);
    // Louder phrases travel right with the wave; the wave settles to nothing at the page.
    const loud = 0.3 + 0.7 * voice(t - x / TRAVEL + i * 0.08);
    const taper = Math.pow(1 - u, 1.6);
    const k = (Math.PI * 2) / r.wavelength;
    const wave = Math.sin(k * x - r.speed * t + r.phase) + 0.25 * Math.sin(2.3 * k * x - 1.7 * r.speed * t + r.phase * 2);
    return base + r.amp * loud * taper * wave;
  }

  /** A ribbon's colour along its length: from nothing at the card's edge, brightest as it
   *  settles, and easing into the faint rule at the page. */
  private along(ctx: CanvasRenderingContext2D, r: Ribbon, a: number) {
    const [cr, cg, cb] = r.color;
    const g = ctx.createLinearGradient(0, 0, JUNCTION, 0);
    g.addColorStop(0, `rgba(${cr}, ${cg}, ${cb}, 0)`);
    g.addColorStop(0.3, `rgba(${cr}, ${cg}, ${cb}, ${0.3 * a})`);
    g.addColorStop(0.62, `rgba(${cr}, ${cg}, ${cb}, ${0.8 * a})`);
    g.addColorStop(0.9, `rgba(${cr}, ${cg}, ${cb}, ${0.35 * a})`);
    g.addColorStop(1, `rgba(${cr}, ${cg}, ${cb}, ${0.08 * a})`);
    return g;
  }

  private drawRibbon(ctx: CanvasRenderingContext2D, r: Ribbon, i: number, t: number, now: number) {
    // The ribbon feeding the line being written is a little brighter.
    const writing = this.fullAt < 0 && r.line === this.line ? 1 : 0;
    const lift = 1 + 0.35 * writing * this.headGlow;
    // Two faint strands beside the ribbon, drifting a little out of phase, so it reads as
    // fine silk rather than a wire; they close up as the ribbon reaches the page.
    ctx.lineCap = 'round';
    ctx.lineJoin = 'round';
    for (const side of [-1, 1]) {
      ctx.beginPath();
      for (let x = -12; x <= JUNCTION; x += 4) {
        const spread = 5 * Math.pow(1 - clamp01(x / JUNCTION), 1.2);
        const y = this.ribbonY(r, i, x, t - side * 0.35) + side * spread;
        if (x === -12) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }
      ctx.strokeStyle = this.along(ctx, r, 0.32 * lift);
      ctx.lineWidth = 0.9;
      ctx.stroke();
    }
    ctx.beginPath();
    for (let x = -12; x <= JUNCTION; x += 4) {
      const y = this.ribbonY(r, i, x, t);
      if (x === -12) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    // A wide soft glow, then the bright core.
    ctx.strokeStyle = this.along(ctx, r, 0.16 * lift);
    ctx.lineWidth = 7;
    ctx.stroke();
    ctx.strokeStyle = this.along(ctx, r, 0.95 * lift);
    ctx.lineWidth = 1.6;
    ctx.stroke();
    // Each stage's pulse: a brighter stretch travelling along the ribbon into the page.
    for (const p of this.pulses) {
      const s = (now - p) / PULSE_MS - i * 0.04;
      if (s <= 0 || s >= 1.15) continue;
      const x = easeInOut(Math.min(1, s)) * (JUNCTION + 20);
      // It fades in from the card's edge, as the ribbon does, and out into the page.
      const a = (s < 1 ? 1 : 1 - (s - 1) / 0.15) * smooth(0, 110, x);
      const g = ctx.createLinearGradient(x - 90, 0, x + 6, 0);
      g.addColorStop(0, 'rgba(225, 230, 255, 0)');
      g.addColorStop(0.85, `rgba(225, 230, 255, ${0.75 * a})`);
      g.addColorStop(1, 'rgba(225, 230, 255, 0)');
      ctx.strokeStyle = g;
      ctx.lineWidth = 2.2;
      ctx.stroke();
    }
  }

  // The words ---------------------------------------------------------------------------------

  private drawWords(ctx: CanvasRenderingContext2D, fade: number) {
    ctx.lineCap = 'round';
    ctx.lineWidth = WORD_HEIGHT;
    ctx.strokeStyle = `rgba(160, 174, 255, ${0.62 * fade})`;
    ctx.beginPath();
    this.page.forEach((words, li) => {
      if (li > this.line) return;
      const y = LINE_Y[li]!;
      for (const w of words) {
        const x1 = li < this.line ? w.x1 : Math.min(w.x1, this.head);
        if (x1 <= w.x0) continue;
        ctx.moveTo(w.x0, y);
        ctx.lineTo(x1, y);
      }
    });
    ctx.stroke();

    // The write-head: the word being written brightens toward its end, like the mark's glint.
    if (this.fullAt >= 0 || this.line >= LINE_Y.length || this.headGlow < 0.02) return;
    const word = this.page[this.line]!.find((w) => w.x0 < this.head && this.head < w.x1 + 2);
    if (!word) return;
    const y = LINE_Y[this.line]!;
    const x1 = Math.min(word.x1, this.head);
    const g = ctx.createLinearGradient(x1 - 34, 0, x1, 0);
    g.addColorStop(0, 'rgba(228, 232, 255, 0)');
    g.addColorStop(0.7, `rgba(228, 232, 255, ${0.4 * this.headGlow})`);
    g.addColorStop(1, `rgba(228, 232, 255, ${0.8 * this.headGlow})`);
    ctx.strokeStyle = g;
    ctx.beginPath();
    ctx.moveTo(Math.max(word.x0, x1 - 34), y);
    ctx.lineTo(x1, y);
    ctx.stroke();
    // A soft light around it.
    const halo = ctx.createRadialGradient(x1, y, 0, x1, y, 16);
    halo.addColorStop(0, `rgba(200, 210, 255, ${0.28 * this.headGlow})`);
    halo.addColorStop(1, 'rgba(200, 210, 255, 0)');
    ctx.fillStyle = halo;
    ctx.fillRect(x1 - 16, y - 16, 32, 32);
  }
}
