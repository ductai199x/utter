// The splash's picture (docs/ux/splash.md): voice becoming text. Points of light ride a
// living waveform, louder and quieter in phrases; every few seconds they stream out of it, left
// to right like text arriving, settle into a word, hold, and flow back into the wave.

export interface VoiceOptions {
  /** CSS pixels of the drawing area. */
  width: number;
  height: number;
  /** Where words form (their center) and where the wave runs. */
  wordCenter: { x: number; y: number };
  waveY: number;
  words: string[];
  font: string;
  reducedMotion: boolean;
}

const N = 2400;
const FORM_S = 1.6;
const HOLD_S = 1.4;
const RELEASE_S = 0.9;
const REST_S = 0.8;
/** Of a word's formation, the share spent on the left-to-right stagger. */
const STAGGER = 0.45;

const easeOut = (p: number) => 1 - Math.pow(1 - p, 3);
const easeInOut = (p: number) => (p < 0.5 ? 4 * p * p * p : 1 - Math.pow(-2 * p + 2, 3) / 2);

export class Voice {
  private readonly o: VoiceOptions;
  private readonly u = new Float32Array(N); // position along the wave, 0..1
  private readonly band = new Float32Array(N); // offset across the wave, about -1..1
  private readonly twinkle = new Float32Array(N);
  private readonly drift = new Float32Array(N);
  // The current word: per point, its target and when it starts moving (0 = not in the word).
  private readonly tx = new Float32Array(N);
  private readonly ty = new Float32Array(N);
  private readonly delay = new Float32Array(N);
  private readonly inWord = new Uint8Array(N);
  private wordIndex = -1;
  private phase: 'rest' | 'form' | 'hold' | 'release' = 'rest';
  private since = 0;
  private ripples: number[] = [];
  private buffer: HTMLCanvasElement;
  private bctx: CanvasRenderingContext2D;
  private dpr = 1;

  constructor(options: VoiceOptions) {
    this.o = options;
    let seed = 11;
    const rnd = () => (seed = (seed * 16807) % 2147483647) / 2147483647;
    for (let i = 0; i < N; i++) {
      this.u[i] = rnd();
      // More points near the middle of the band: a bright core and a soft edge.
      this.band[i] = (rnd() + rnd() + rnd() - 1.5) / 1.5;
      this.twinkle[i] = rnd() * Math.PI * 2;
      this.drift[i] = 0.6 + rnd() * 0.8;
    }
    this.buffer = document.createElement('canvas');
    this.bctx = this.buffer.getContext('2d')!;
  }

  /** A stage finished: a swell runs through the wave. */
  ripple(now: number) {
    this.ripples.push(now / 1000);
  }

  /** The wave's displacement at x, and its loudness there (0..1). */
  private wave(x: number, t: number): [number, number] {
    const { width } = this.o;
    const s = x / width;
    // Phrases: the loudness rises and falls like speech, never quite silent.
    let loud = 0.35 + 0.3 * Math.sin(t * 0.9) + 0.2 * Math.sin(t * 2.3 + 1.1) + 0.15 * Math.sin(t * 5.1 + 2.0);
    for (const r of this.ripples) {
      const age = t - r;
      const front = age * 0.9; // the swell travels left to right
      loud += Math.max(0, 1 - Math.abs(s - front) * 6) * Math.max(0, 1 - age / 1.4) * 0.9;
    }
    loud = Math.min(1.2, Math.max(0.12, loud));
    // Quieter at the card's edges, so the wave seems to come from somewhere and go somewhere.
    const edge = Math.sin(Math.PI * Math.min(1, Math.max(0, s)));
    const amp = 34 * loud * edge;
    const y =
      amp *
      (Math.sin(s * 18 - t * 3.2) * 0.55 + Math.sin(s * 7.3 + t * 1.7) * 0.3 + Math.sin(s * 41 - t * 6.5) * 0.15);
    return [y, loud * edge];
  }

  /** Picks the next word's points: the word drawn off screen, its filled pixels sampled. */
  private shapeWord(index: number) {
    const { width, height, wordCenter, font } = this.o;
    const c = document.createElement('canvas');
    c.width = width;
    c.height = height;
    const ctx = c.getContext('2d')!;
    ctx.font = font;
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillStyle = '#fff';
    ctx.fillText(this.o.words[index % this.o.words.length]!, wordCenter.x, wordCenter.y);
    const data = ctx.getImageData(0, 0, width, height).data;
    const targets: [number, number][] = [];
    const step = 2;
    for (let y = 0; y < height; y += step) {
      for (let x = 0; x < width; x += step) {
        if (data[(y * width + x) * 4 + 3]! > 140) targets.push([x + Math.random() - 0.5, y + Math.random() - 0.5]);
      }
    }
    // As many points as the word needs, at most two thirds: the rest stay in the wave.
    const count = Math.min(targets.length, Math.floor(N * 0.66));
    for (let i = targets.length - 1; i > 0; i--) {
      const j = Math.floor(Math.random() * (i + 1));
      [targets[i], targets[j]] = [targets[j]!, targets[i]!];
    }
    const chosen = targets.slice(0, count).sort((a, b) => a[0] - b[0]);
    // The wave's points, ordered along it: left points make the left letters, so the word
    // streams out of the wave left to right.
    const order = Array.from({ length: N }, (_, i) => i).sort((a, b) => this.u[a]! - this.u[b]!);
    const stride = N / Math.max(1, count);
    const minX = chosen[0]?.[0] ?? 0;
    const spanX = Math.max(1, (chosen[chosen.length - 1]?.[0] ?? 1) - minX);
    this.inWord.fill(0);
    for (let k = 0; k < count; k++) {
      const i = order[Math.floor(k * stride)]!;
      const [x, y] = chosen[k]!;
      this.inWord[i] = 1;
      this.tx[i] = x;
      this.ty[i] = y;
      this.delay[i] = ((x - minX) / spanX) * FORM_S * STAGGER;
    }
  }

  /** Draws one frame at time `now` (ms) onto `canvas`, sized to the options' CSS pixels. */
  draw(canvas: HTMLCanvasElement, now: number) {
    const { width: W, height: H, waveY, reducedMotion } = this.o;
    const dpr = Math.min(2, window.devicePixelRatio || 1);
    if (dpr !== this.dpr || canvas.width !== Math.round(W * dpr)) {
      this.dpr = dpr;
      for (const c of [canvas, this.buffer]) {
        c.width = Math.round(W * dpr);
        c.height = Math.round(H * dpr);
      }
    }
    const t = reducedMotion ? 0 : now / 1000;
    this.ripples = this.ripples.filter((r) => t - r < 1.6);

    // The word cycle.
    if (!reducedMotion && this.o.words.length) {
      const age = t - this.since;
      if (this.phase === 'rest' && age > REST_S) {
        this.wordIndex += 1;
        this.shapeWord(this.wordIndex);
        this.phase = 'form';
        this.since = t;
      } else if (this.phase === 'form' && age > FORM_S) {
        this.phase = 'hold';
        this.since = t;
      } else if (this.phase === 'hold' && age > HOLD_S) {
        this.phase = 'release';
        this.since = t;
      } else if (this.phase === 'release' && age > RELEASE_S) {
        this.phase = 'rest';
        this.since = t;
        this.inWord.fill(0);
      }
    }
    const age = t - this.since;

    const ctx = this.bctx;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, W, H);
    ctx.globalCompositeOperation = 'lighter';

    // A soft glow behind a formed word.
    const formed =
      this.phase === 'hold' ? 1 : this.phase === 'form' ? easeOut(Math.min(1, age / FORM_S)) : this.phase === 'release' ? 1 - age / RELEASE_S : 0;
    if (formed > 0.01) {
      const { x, y } = this.o.wordCenter;
      const g = ctx.createRadialGradient(x, y, 8, x, y, 190);
      g.addColorStop(0, `rgba(120, 150, 255, ${(0.16 * formed).toFixed(3)})`);
      g.addColorStop(1, 'rgba(120, 150, 255, 0)');
      ctx.fillStyle = g;
      ctx.fillRect(0, 0, W, H);
    }

    for (let i = 0; i < N; i++) {
      // Where the point is on the wave.
      const u = (this.u[i]! + t * 0.012 * this.drift[i]!) % 1;
      const wx = u * (W + 40) - 20;
      const [dy, loud] = this.wave(wx, t);
      const spread = 3 + 16 * loud;
      const wy = waveY + dy + this.band[i]! * spread;
      let x = wx;
      let y = wy;
      let p = 0; // 0 on the wave, 1 in the word
      if (this.inWord[i]) {
        if (this.phase === 'form') p = easeInOut(Math.min(1, Math.max(0, (age - this.delay[i]!) / (FORM_S * (1 - STAGGER)))));
        else if (this.phase === 'hold') p = 1;
        else if (this.phase === 'release') p = 1 - easeInOut(Math.min(1, age / RELEASE_S));
        if (p > 0) {
          // A small arc on the way, so points rise out of the wave rather than slide.
          const lift = Math.sin(p * Math.PI) * 18;
          x = wx + (this.tx[i]! - wx) * p;
          y = wy + (this.ty[i]! - wy) * p - lift;
          if (this.phase === 'hold') {
            x += Math.sin(t * 2.1 + this.twinkle[i]!) * 0.35;
            y += Math.cos(t * 1.7 + this.twinkle[i]!) * 0.35;
          }
        }
      }
      const tw = 0.75 + 0.25 * Math.sin(t * 2.4 + this.twinkle[i]!);
      const core = 1 - Math.min(1, Math.abs(this.band[i]!));
      const a = (0.22 + 0.5 * core) * tw * (1 - p) + 0.92 * p * tw;
      // Wave points in the accent's blue; points in a word warm to near white.
      const r = Math.round(107 + (232 - 107) * p);
      const gch = Math.round(140 + (238 - 140) * p);
      ctx.fillStyle = `rgba(${r}, ${gch}, 255, ${a.toFixed(3)})`;
      const size = 1.1 + 1.1 * core * (1 - p) + 0.9 * p;
      ctx.fillRect(x - size / 2, y - size / 2, size, size);
    }

    const out = canvas.getContext('2d')!;
    out.setTransform(1, 0, 0, 1, 0, 0);
    out.clearRect(0, 0, canvas.width, canvas.height);
    out.drawImage(this.buffer, 0, 0);
  }
}
